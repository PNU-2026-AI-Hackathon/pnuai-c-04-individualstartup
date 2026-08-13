use super::*;
use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};
use base64::Engine;
use std::sync::Mutex;
use tokio::sync::Notify;

#[test]
fn transport_payload_policy_redacts_sensitive_keys_and_marks_truncation() {
    let normalized = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "result": { "accessToken": "secret-value", "environment": { "SAFE": "no" } },
        "output": "x".repeat(5_000)
    }));
    assert_eq!(normalized["result"]["accessToken"], "[redacted]");
    assert_eq!(normalized["result"]["environment"], "[redacted]");
    assert_eq!(normalized["_cadastropheTransportPolicy"]["redacted"], true);
    assert_eq!(normalized["_cadastropheTransportPolicy"]["truncated"], true);
    assert!(normalized["output"]
        .as_str()
        .unwrap()
        .ends_with("[truncated]"));
}

#[test]
fn transport_payload_policy_removes_hidden_reasoning_and_user_prompt_content() {
    let reasoning_delta = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "method": "item/reasoning/textDelta",
        "params": { "threadId": "thread-1", "turnId": "turn-1", "delta": "raw reasoning" }
    }));
    assert_eq!(reasoning_delta["params"]["delta"], "[redacted]");
    assert_eq!(
        reasoning_delta["_cadastropheTransportPolicy"]["redacted"],
        true
    );

    let summary_delta = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "method": "item/reasoning/summaryTextDelta",
        "params": { "delta": "safe summary" }
    }));
    assert_eq!(summary_delta["params"]["delta"], "safe summary");

    let reasoning = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "method": "item/completed",
        "params": { "item": {
            "id": "reasoning-1",
            "type": "reasoning",
            "content": [{ "type": "reasoningText", "text": "hidden chain of thought" }],
            "summary": ["public summary"]
        } }
    }));
    assert_eq!(reasoning["params"]["item"]["content"], "[redacted]");
    assert_eq!(
        reasoning["params"]["item"]["summary"],
        serde_json::json!(["public summary"])
    );
    assert_eq!(reasoning["_cadastropheTransportPolicy"]["redacted"], true);

    let user_message = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "method": "item/completed",
        "params": { "item": {
            "id": "user-1", "type": "userMessage", "content": "private prompt"
        } }
    }));
    assert_eq!(user_message["params"]["item"]["content"], "[redacted]");

    let agent_message = crate::agent_gateway::normalize_transport_payload(&serde_json::json!({
        "method": "item/completed",
        "params": { "item": {
            "id": "assistant-1", "type": "agentMessage", "content": "public assistant diagnostic"
        } }
    }));
    assert_eq!(
        agent_message["params"]["item"]["content"],
        "public assistant diagnostic"
    );
}

#[test]
fn completed_stream_event_follows_durable_agent_message_snapshot() {
    let service = SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "stream ordering".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let now = crate::session_service::timestamp();
    let thread = CadAgentThread {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: created.session_id.clone(),
        external_agent: "codex".to_string(),
        external_thread_id: "thread-order".to_string(),
        status: CadAgentThreadStatus::Active,
        connection_generation: Some(1),
        created_at: now.clone(),
        updated_at: now,
        last_resumed_at: None,
        archived_at: None,
        replaced_by_id: None,
        metadata: None,
    };
    service.upsert_agent_thread(thread.clone()).unwrap();
    service
        .bind_agent_run_to_thread(
            &created.session_id,
            &run.id,
            &thread.id,
            Some("turn-order".to_string()),
            Some(1),
            CadAgentRecoveryStatus::None,
        )
        .unwrap();
    let mut bridge = service.subscribe();
    let mut stream = service.subscribe_agent_stream();

    crate::agent_gateway::apply_adapter_event(
        &service,
        &run,
        AgentAdapterEvent::AgentMessageCompleted {
            external_thread_id: "thread-order".to_string(),
            external_turn_id: "turn-order".to_string(),
            external_item_id: "item-order".to_string(),
            phase: CadConversationPhase::FinalAnswer,
            content: "Durable first.".to_string(),
            sequence: 7,
            is_final: true,
            metadata: None,
        },
    )
    .unwrap();

    let snapshot = bridge.try_recv().unwrap();
    assert!(snapshot.state.conversation.iter().any(|message| {
        message.external_item_id.as_deref() == Some("item-order")
            && message.content == "Durable first."
    }));
    let completed = stream.try_recv().unwrap();
    assert!(completed.completed);
    assert_eq!(completed.item_id, "item-order");
}

#[test]
fn delete_session_rejects_nonterminal_agent_run_without_mutating_session() {
    let service = SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    service
        .create_agent_run(
            &created.session_id,
            "still executing".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("codex".to_string()),
            None,
        )
        .unwrap();

    let error = service.delete_session(&created.session_id).unwrap_err();
    assert!(error.contains("active agent run"));
    assert_eq!(
        service
            .get_session_state(&created.session_id)
            .unwrap()
            .session
            .id,
        created.session_id
    );
}

#[test]
fn gateway_start_run_is_safe_from_sync_tauri_command_context() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(MessageOnlyAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();

    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "Create a sync-command launch regression fixture.".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();

    let state = wait_for_run_status(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Completed,
    );
    let run = state
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(run.status, CadAgentRunStatus::Completed);
}

#[tokio::test]
async fn gateway_completes_prompt_to_preview_loop() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(SourceUpdateAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "Create a slotted fixture plate.".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    let state = wait_for_run_status_async(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Completed,
    )
    .await;
    let completed = state
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(completed.status, CadAgentRunStatus::Completed);
    assert!(state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == CadArtifactKind::PreviewMesh));
    let run = state
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(
        run.input_revision_id.as_deref(),
        created.state.session.active_revision_id.as_deref()
    );
    assert_eq!(
        run.output_revision_id.as_deref(),
        state.session.active_revision_id.as_deref()
    );
    let active_summary = state
        .session
        .revisions
        .iter()
        .find(|revision| Some(revision.id.as_str()) == state.session.active_revision_id.as_deref())
        .unwrap();
    assert!(active_summary
        .run_links
        .iter()
        .any(|link| link.run_id == started.run.id && link.role == "output"));
}

#[tokio::test]
async fn gateway_records_adapter_failure() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(FailingAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "trigger adapter failure".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let state = service.get_session_state(&created.session_id).unwrap();
    let failed = state
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(failed.status, CadAgentRunStatus::Failed);
    assert!(failed
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("Adapter failed by test request"));
    assert_eq!(state.session.status, CadSessionStatus::Failed);
}

#[tokio::test]
async fn gateway_persists_adapter_progress_before_completion() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(StreamingProgressAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "stream progress".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let in_progress = service.get_session_state(&created.session_id).unwrap();
    let run = in_progress
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(run.status, CadAgentRunStatus::Running);
    assert_eq!(run.active_step.as_deref(), Some("Planning geometry"));
    assert!(in_progress.agent_run_events.iter().any(|event| {
        event.run_id == started.run.id
            && event.event_type == CadAgentRunEventType::AgentRunUpdated
            && event.payload.get("progressLabel").and_then(Value::as_str)
                == Some("Planning geometry")
    }));

    let completed = wait_for_run_status(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Completed,
    );
    assert_eq!(
        completed
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap()
            .status,
        CadAgentRunStatus::Completed
    );
}

#[tokio::test]
async fn gateway_cancel_marks_running_run_cancelled() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(DelayedAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "Create a delayed run.".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    let (cancelled, state) = gateway
        .cancel_run(&created.session_id, &started.run.id)
        .unwrap();
    assert_eq!(cancelled.status, CadAgentRunStatus::Cancelled);
    assert_eq!(
        state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap()
            .status,
        CadAgentRunStatus::Cancelled
    );
}

#[tokio::test]
async fn gateway_cancel_after_external_turn_reconciles_and_terminal_outcome_wins() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let metadata_ready = Arc::new(Notify::new());
    let release_run = Arc::new(Notify::new());
    let gateway = AgentGateway::new(
        Arc::clone(&service),
        Arc::new(ControlledCancelAdapter {
            service: Arc::clone(&service),
            metadata_ready: Arc::clone(&metadata_ready),
            release_run: Arc::clone(&release_run),
            interrupt_should_fail: false,
        }),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "cancel after metadata".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    metadata_ready.notified().await;

    let (returned, state) = gateway
        .cancel_run(&created.session_id, &started.run.id)
        .unwrap();
    assert_eq!(returned.status, CadAgentRunStatus::Running);
    assert_eq!(
        returned.recovery_status,
        CadAgentRecoveryStatus::Reconciling
    );
    assert_eq!(
        state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap()
            .status,
        CadAgentRunStatus::Running
    );

    let settled = wait_for_run_status_async(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Cancelled,
    )
    .await;
    let settled_run = settled
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(
        settled_run.recovery_status,
        CadAgentRecoveryStatus::RecoveredFromHistory
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        service
            .get_agent_run(&created.session_id, &started.run.id)
            .unwrap()
            .unwrap()
            .status,
        CadAgentRunStatus::Cancelled
    );
}

#[tokio::test]
async fn gateway_cancel_interrupt_failure_records_unknown_outcome() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let metadata_ready = Arc::new(Notify::new());
    let release_run = Arc::new(Notify::new());
    let gateway = AgentGateway::new(
        Arc::clone(&service),
        Arc::new(ControlledCancelAdapter {
            service: Arc::clone(&service),
            metadata_ready: Arc::clone(&metadata_ready),
            release_run: Arc::clone(&release_run),
            interrupt_should_fail: true,
        }),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "interrupt failure".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    metadata_ready.notified().await;
    gateway
        .cancel_run(&created.session_id, &started.run.id)
        .unwrap();

    let settled = wait_for_run_status_async(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Failed,
    )
    .await;
    let settled_run = settled
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(
        settled_run.recovery_status,
        CadAgentRecoveryStatus::UnknownOutcome
    );
}

#[tokio::test]
async fn gateway_includes_latest_workflow_failure_report_in_retry_input() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let captured_failure = Arc::new(Mutex::new(None));
    let gateway = AgentGateway::new(
        Arc::clone(&service),
        Arc::new(CapturingFailureAdapter {
            captured_failure: Arc::clone(&captured_failure),
        }),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (failed_source_run, _) = service
        .create_agent_run(
            &created.session_id,
            "failed source".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    service
        .save_workflow_outer_iteration(
            &created.session_id,
            CadWorkflowOuterIteration {
                id: "workflow-outer-failed-source".to_string(),
                run_id: failed_source_run.id.clone(),
                iteration: 1,
                revision_id: created.state.session.active_revision_id.clone(),
                structural_report: serde_json::json!({
                    "contractType": "cadastrophe.structural_report.v1",
                    "passed": false
                }),
                dfm_report: None,
                vlm_report: None,
                failure_report: Some(serde_json::json!({
                    "contractType": "cadastrophe.failure_report.v1",
                    "reason": "missing_support_tab",
                    "nextAction": "outer_loop_refine_source"
                })),
                passed: false,
                created_at: "2026-07-29T00:00:00.000Z".to_string(),
            },
        )
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &failed_source_run.id,
            Some(CadAgentRunStatus::Failed),
            Some(None),
            Some("outer loop refinement required".to_string()),
            None,
            None,
        )
        .unwrap();

    gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "retry with previous failure".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: Some(failed_source_run.id),
        })
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let captured = captured_failure
        .lock()
        .expect("captured failure lock")
        .clone()
        .expect("retry failure report should be passed to adapter");
    assert_eq!(
        captured.get("reason").and_then(Value::as_str),
        Some("missing_support_tab")
    );
}

#[tokio::test]
async fn gateway_refreshes_workflow_state_after_cadastrophe_cli_completion() {
    let app_data_dir = std::env::temp_dir().join(format!(
        "cadastrophe-gateway-workflow-refresh-test-{}",
        uuid::Uuid::new_v4()
    ));
    let layout = storage::StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();
    let service = Arc::new(
        SessionService::with_repository(
            layout.clone(),
            Arc::new(session_repository::SqliteSessionRepository::new(layout)),
        )
        .unwrap(),
    );
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(ExternalWorkflowAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();

    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "commit workflow externally".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let state = service.get_session_state(&created.session_id).unwrap();
    let run = state
        .agent_runs
        .iter()
        .find(|run| run.id == started.run.id)
        .unwrap();
    assert_eq!(run.status, CadAgentRunStatus::Completed);
    assert_eq!(state.workflow.plans.len(), 1);
    assert_eq!(state.workflow.plans[0].run_id, started.run.id);
    assert_eq!(
        state.workflow.plans[0].plan.main_component.name,
        "wall_bracket"
    );
}

#[tokio::test]
async fn gateway_consumes_inline_vlm_judge_report_without_showing_raw_json() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(
        Arc::clone(&service),
        Arc::new(InlineVlmReportAdapter {
            service: Arc::clone(&service),
        }),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let revision_id = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "cube([2, 2, 2]);".to_string(),
            parent_revision_id: None,
            parameters: None,
        })
        .unwrap()
        .revision_id;

    let started = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "return inline vlm report".to_string(),
            revision_id: Some(revision_id),
            retry_of_run_id: None,
        })
        .unwrap();

    let state = wait_for_run_status_async(
        &service,
        &created.session_id,
        &started.run.id,
        CadAgentRunStatus::Completed,
    )
    .await;
    assert!(state.workflow.pending_vlm.is_empty());
    assert_eq!(state.workflow.outer_iterations.len(), 1);
    assert!(state.workflow.outer_iterations[0].passed);
    assert!(state.workflow.outer_iterations[0].vlm_report.is_some());
    assert!(state
        .conversation
        .iter()
        .all(|message| { !message.content.contains("cadastrophe.vlm_judge_report.v1") }));
    assert!(state.conversation.iter().any(|message| {
        message.role == CadConversationRole::Assistant
            && message.content.contains("VLM accepted final artifact")
    }));
    assert!(state.agent_run_events.iter().any(|event| {
        event.run_id == started.run.id
            && event.event_type == CadAgentRunEventType::AgentToolCompleted
            && event.payload.get("phase").and_then(Value::as_str) == Some("vlm-judge-callback")
    }));
}

#[tokio::test]
async fn gateway_rejects_duplicate_active_run_in_same_session() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(OutOfOrderAdapter));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let first = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "first slow source".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap();
    let error = gateway
        .start_run(CreateAgentRunInput {
            session_id: created.session_id.clone(),
            prompt: "second fast source".to_string(),
            revision_id: created.state.session.active_revision_id.clone(),
            retry_of_run_id: None,
        })
        .unwrap_err();
    assert!(error.contains("already has an active agent run"));

    let _ = wait_for_run_status(
        &service,
        &created.session_id,
        &first.run.id,
        CadAgentRunStatus::Completed,
    );
    let state = service.get_session_state(&created.session_id).unwrap();
    assert_eq!(
        state
            .agent_runs
            .iter()
            .find(|run| run.id == first.run.id)
            .unwrap()
            .status,
        CadAgentRunStatus::Completed
    );
    assert_eq!(state.agent_runs.len(), 1);
}

#[tokio::test]
async fn gateway_allows_active_runs_in_distinct_sessions() {
    let service = Arc::new(SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
    ));
    let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(DelayedAdapter));
    let first_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let second_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let first = gateway
        .start_run(CreateAgentRunInput {
            session_id: first_session.session_id.clone(),
            prompt: "first session".to_string(),
            revision_id: None,
            retry_of_run_id: None,
        })
        .unwrap();
    let second = gateway
        .start_run(CreateAgentRunInput {
            session_id: second_session.session_id.clone(),
            prompt: "second session".to_string(),
            revision_id: None,
            retry_of_run_id: None,
        })
        .unwrap();

    wait_for_run_status_async(
        &service,
        &first_session.session_id,
        &first.run.id,
        CadAgentRunStatus::Completed,
    )
    .await;
    wait_for_run_status_async(
        &service,
        &second_session.session_id,
        &second.run.id,
        CadAgentRunStatus::Completed,
    )
    .await;
}

struct OutOfOrderAdapter;

struct MessageOnlyAdapter;

struct SourceUpdateAdapter;

struct FailingAdapter;

struct DelayedAdapter;

struct ControlledCancelAdapter {
    service: Arc<SessionService>,
    metadata_ready: Arc<Notify>,
    release_run: Arc<Notify>,
    interrupt_should_fail: bool,
}

fn wait_for_run_status(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    status: CadAgentRunStatus,
) -> CadSessionState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = service.get_session_state(session_id).unwrap();
        if state
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .is_some_and(|run| run.status == status)
        {
            return state;
        }
        if std::time::Instant::now() > deadline {
            return state;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

async fn wait_for_run_status_async(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    status: CadAgentRunStatus,
) -> CadSessionState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = service.get_session_state(session_id).unwrap();
        if state
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .is_some_and(|run| run.status == status)
        {
            return state;
        }
        if std::time::Instant::now() > deadline {
            return state;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[async_trait::async_trait]
impl AgentAdapter for OutOfOrderAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        if input.prompt.contains("first") {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(vec![AgentAdapterEvent::SourceUpdated {
            source_language: CadSourceLanguage::Openscad,
            source: format!("// {}\ncube([1, 1, 1]);", input.prompt),
        }])
    }
}

#[async_trait::async_trait]
impl AgentAdapter for MessageOnlyAdapter {
    fn external_agent(&self) -> &'static str {
        "test-message-adapter"
    }

    async fn run(&self, _input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        Ok(vec![AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: "Run completed.".to_string(),
            metadata: None,
        }])
    }
}

#[async_trait::async_trait]
impl AgentAdapter for SourceUpdateAdapter {
    fn external_agent(&self) -> &'static str {
        "test-source-adapter"
    }

    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        Ok(vec![
            AgentAdapterEvent::ToolStarted {
                name: "generate_source".to_string(),
            },
            AgentAdapterEvent::SourceUpdated {
                source_language: CadSourceLanguage::Openscad,
                source: format!("// {}\ncube([1, 1, 1]);", input.prompt),
            },
            AgentAdapterEvent::ToolCompleted {
                name: "generate_source".to_string(),
            },
            AgentAdapterEvent::MessageCreated {
                role: CadConversationRole::Assistant,
                content: "Created OpenSCAD source.".to_string(),
                metadata: None,
            },
        ])
    }
}

#[async_trait::async_trait]
impl AgentAdapter for FailingAdapter {
    fn external_agent(&self) -> &'static str {
        "test-failure-adapter"
    }

    async fn run(&self, _input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        Err("Adapter failed by test request.".to_string())
    }
}

#[async_trait::async_trait]
impl AgentAdapter for DelayedAdapter {
    fn external_agent(&self) -> &'static str {
        "test-delayed-adapter"
    }

    async fn run(&self, _input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        Ok(vec![AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: "Delayed run completed.".to_string(),
            metadata: None,
        }])
    }
}

#[async_trait::async_trait]
impl AgentAdapter for ControlledCancelAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        input.emit_event(
            &mut Vec::new(),
            AgentAdapterEvent::RunMetadata {
                external_agent: Some("codex".to_string()),
                external_thread_id: Some("thread-cancel".to_string()),
                external_turn_id: Some("turn-cancel".to_string()),
            },
        )?;
        self.metadata_ready.notify_one();
        self.release_run.notified().await;
        Err("original run stopped after cancellation".to_string())
    }

    async fn interrupt_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        if self.interrupt_should_fail {
            self.release_run.notify_one();
            return Err("scripted interrupt failure".to_string());
        }
        self.service
            .apply_agent_run_history_recovery(CadAgentRunHistoryRecoveryInput {
                session_id: session_id.to_string(),
                run_id: run_id.to_string(),
                outcome: CadAgentRunHistoryOutcome::Interrupted {
                    reason: "cancelled by test".to_string(),
                },
            })?;
        self.release_run.notify_one();
        Ok(())
    }
}

struct StreamingProgressAdapter;

#[async_trait::async_trait]
impl AgentAdapter for StreamingProgressAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        if let Some(event_sink) = &input.event_sink {
            event_sink(AgentAdapterEvent::Progress {
                label: "Planning geometry".to_string(),
                message: Some("test progress event".to_string()),
                metadata: None,
            })?;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        Ok(vec![AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: "Streaming progress completed.".to_string(),
            metadata: None,
        }])
    }
}

struct CapturingFailureAdapter {
    captured_failure: Arc<Mutex<Option<Value>>>,
}

#[async_trait::async_trait]
impl AgentAdapter for CapturingFailureAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        *self.captured_failure.lock().expect("captured failure lock") =
            input.latest_workflow_failure_report;
        Ok(vec![AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: "Captured retry failure context.".to_string(),
            metadata: None,
        }])
    }
}

struct InlineVlmReportAdapter {
    service: Arc<SessionService>,
}

#[async_trait::async_trait]
impl AgentAdapter for InlineVlmReportAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        let revision_id = input
            .revision_id
            .clone()
            .ok_or_else(|| "test requires active revision".to_string())?;
        let artifact = self
            .service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: input.session_id.clone(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::Stl,
                format: "stl".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"solid inline_vlm\nendsolid inline_vlm\n"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 1,
                    items: Vec::new(),
                },
                metadata: serde_json::Map::new(),
            })?
            .artifact;
        self.service.save_workflow_pending_vlm(
            &input.session_id,
            CadWorkflowPendingVlm {
                run_id: input.run_id.clone(),
                artifact_id: artifact.id.clone(),
                revision_id: Some(revision_id.clone()),
                contract: serde_json::json!({
                    "contractType": "cadastrophe.vlm_judge.v1",
                    "handoff": "VLM Judge Handoff needed.",
                    "renderedImages": {
                        "available": true,
                        "path": "/tmp/fixture-render.png"
                    }
                }),
                pass_threshold: 0.8,
                structural_report: Some(serde_json::json!({
                    "contractType": "cadastrophe.structural_report.v1",
                    "runId": input.run_id,
                    "artifactId": artifact.id,
                    "passed": true
                })),
                dfm_report: None,
                created_at: "2026-07-30T00:00:00.000Z".to_string(),
            },
        )?;
        let report = serde_json::json!({
            "contractType": "cadastrophe.vlm_judge_report.v1",
            "score": 0.91,
            "passed": true,
            "findings": [],
            "diagnostic": "fixture accepted"
        });
        Ok(vec![AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?,
            metadata: None,
        }])
    }
}

struct ExternalWorkflowAdapter;

#[async_trait::async_trait]
impl AgentAdapter for ExternalWorkflowAdapter {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        let layout = storage::StorageLayout::from_app_data_dir(input.app_data_dir.clone());
        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(session_repository::SqliteSessionRepository::new(layout)),
        )?;
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .map_err(|error| error.to_string())?;
        service.save_workflow_plan(
            &input.session_id,
            CadWorkflowPlan {
                run_id: input.run_id.clone(),
                revision_id: input.revision_id.clone(),
                source_language: plan.source_language.clone(),
                plan,
                created_at: "2026-07-29T00:00:00.000Z".to_string(),
            },
        )?;
        Ok(vec![
            AgentAdapterEvent::ToolStarted {
                name: "cadastrophe-plan-commit".to_string(),
            },
            AgentAdapterEvent::ToolCompleted {
                name: "cadastrophe-plan-commit".to_string(),
            },
        ])
    }
}
