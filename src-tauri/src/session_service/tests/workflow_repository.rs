use super::*;

#[test]
fn sqlite_repository_restores_conversation_runs_and_run_events_after_restart() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-run-log-repo-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = created.state.session.active_revision_id.clone();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a persisted run log fixture.".to_string(),
            input_revision_id.clone(),
            Some("test-agent".to_string()),
            None,
        )
        .unwrap();
    service
        .create_conversation_message(
            &created.session_id,
            input_revision_id.clone(),
            CadConversationRole::Assistant,
            "I will update the model.".to_string(),
            Some(run.id.clone()),
            Some(metadata_from_value(json!({"source": "test"}))),
        )
        .unwrap();
    service
        .update_agent_run_external_metadata(
            &created.session_id,
            &run.id,
            Some("test-agent".to_string()),
            Some("thread-1".to_string()),
            Some("turn-1".to_string()),
        )
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            Some(CadAgentRunStatus::Running),
            Some(Some("generate_source".to_string())),
            None,
            Some(CadBridgeEventType::AgentToolStarted),
            Some(json!({"tool": "generate_source"})),
        )
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            None,
            Some(None),
            None,
            Some(CadBridgeEventType::AgentToolCompleted),
            Some(json!({"tool": "generate_source"})),
        )
        .unwrap();
    let output = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "sphere(r = 4);".to_string(),
            parent_revision_id: input_revision_id.clone(),
            parameters: None,
        })
        .unwrap();
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, output.revision_id)
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            Some(CadAgentRunStatus::Completed),
            Some(None),
            None,
            Some(CadBridgeEventType::AgentRunCompleted),
            Some(json!({"status": "completed"})),
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    assert!(state
        .conversation
        .iter()
        .any(|message| message.run_id.as_deref() == Some(run.id.as_str())
            && message.role == CadConversationRole::Assistant
            && message.content == "I will update the model."));
    let restored_run = state
        .agent_runs
        .iter()
        .find(|candidate| candidate.id == run.id)
        .expect("agent run restored");
    assert_eq!(restored_run.status, CadAgentRunStatus::Completed);
    assert_eq!(restored_run.external_agent.as_deref(), Some("test-agent"));
    assert_eq!(restored_run.external_thread_id.as_deref(), Some("thread-1"));
    assert_eq!(restored_run.external_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
        restored_run.input_revision_id.as_deref(),
        input_revision_id.as_deref()
    );
    assert!(restored_run.output_revision_id.is_some());
    let event_types = state
        .agent_run_events
        .iter()
        .filter(|event| event.run_id == run.id)
        .map(|event| event.event_type.clone())
        .collect::<Vec<_>>();
    assert!(event_types.contains(&CadAgentRunEventType::AgentRunCreated));
    assert!(event_types.contains(&CadAgentRunEventType::AgentMessageCreated));
    assert!(event_types.contains(&CadAgentRunEventType::AgentToolStarted));
    assert!(event_types.contains(&CadAgentRunEventType::AgentToolCompleted));
    assert!(event_types.contains(&CadAgentRunEventType::AgentRunCompleted));
    let generate_tool_event = state
        .agent_run_events
        .iter()
        .find(|event| event.event_type == CadAgentRunEventType::AgentToolStarted)
        .expect("tool event restored");
    assert_eq!(
        generate_tool_event
            .payload
            .get("tool")
            .and_then(Value::as_str),
        Some("generate_source")
    );
}

#[test]
fn sqlite_repository_restores_workflow_state_after_restart() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-workflow-repo-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let updated = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "// @main_component wall_bracket\ncube([30, 10, 20]);".to_string(),
            parent_revision_id: None,
            parameters: None,
        })
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Create a workflow state fixture.".to_string(),
            Some(updated.revision_id.clone()),
            Some("test-agent".to_string()),
            None,
        )
        .unwrap();
    let plan: CadModelPlan = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/cad_model_plan.v1.json"
    ))
    .unwrap();
    let workflow_plan = CadWorkflowPlan {
        run_id: run.id.clone(),
        revision_id: Some(updated.revision_id.clone()),
        source_language: plan.source_language.clone(),
        plan,
        created_at: "2026-07-29T00:00:00.000Z".to_string(),
    };
    service
        .save_workflow_plan(&created.session_id, workflow_plan)
        .unwrap();
    service
        .save_workflow_outer_iteration(
            &created.session_id,
            CadWorkflowOuterIteration {
                id: "workflow-outer-test-1".to_string(),
                run_id: run.id.clone(),
                iteration: 1,
                revision_id: Some(updated.revision_id.clone()),
                structural_report: serde_json::from_str(include_str!(
                    "../../../../fixtures/contracts/structural_report.v1.json"
                ))
                .unwrap(),
                dfm_report: Some(json!({
                    "contractType": "cadastrophe.dfm_report.v1",
                    "revisionId": updated.revision_id,
                    "profileHash": "a".repeat(64),
                    "passed": true
                })),
                vlm_report: Some(
                    serde_json::from_str(include_str!(
                        "../../../../fixtures/contracts/vlm_judge_report.v1.json"
                    ))
                    .unwrap(),
                ),
                failure_report: Some(json!({
                    "contractType": "cadastrophe.failure_report.v1",
                    "reason": "missing_support_tab",
                    "nextAction": "outer_loop_refine_source"
                })),
                passed: false,
                created_at: "2026-07-29T00:00:01.000Z".to_string(),
            },
        )
        .unwrap();
    let artifact = service
        .persist_runtime_artifact(PersistRuntimeArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: updated.revision_id.clone(),
            kind: CadArtifactKind::Stl,
            format: "stl".to_string(),
            contents_base64: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .encode(b"solid workflow_fixture\nendsolid workflow_fixture\n")
            },
            diagnostics: ok_diagnostics(1),
            metadata: metadata_from_value(json!({
                "runtime": "openscad-wasm",
                "sourceLanguage": "openscad"
            })),
        })
        .unwrap()
        .artifact;
    service
        .save_workflow_pending_vlm(
            &created.session_id,
            CadWorkflowPendingVlm {
                run_id: run.id.clone(),
                artifact_id: artifact.id.clone(),
                revision_id: Some(updated.revision_id.clone()),
                contract: serde_json::from_str(include_str!(
                    "../../../../fixtures/contracts/vlm_judge_contract.v1.json"
                ))
                .unwrap(),
                pass_threshold: 0.8,
                structural_report: Some(json!({
                    "contractType": "cadastrophe.structural_report.v1",
                    "runId": run.id,
                    "artifactId": artifact.id,
                    "passed": true
                })),
                dfm_report: Some(json!({
                    "contractType": "cadastrophe.dfm_report.v1",
                    "runId": run.id,
                    "revisionId": updated.revision_id,
                    "artifactId": artifact.id,
                    "profileHash": "a".repeat(64),
                    "passed": true
                })),
                created_at: "2026-07-29T00:00:02.000Z".to_string(),
            },
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();

    assert_eq!(state.workflow.plans.len(), 1);
    assert_eq!(state.workflow.plans[0].run_id, run.id);
    assert_eq!(
        state.workflow.plans[0].revision_id.as_deref(),
        Some(updated.revision_id.as_str())
    );
    assert_eq!(
        state.workflow.plans[0].plan.main_component.name,
        "wall_bracket"
    );
    assert_eq!(state.workflow.outer_iterations.len(), 1);
    assert_eq!(state.workflow.outer_iterations[0].iteration, 1);
    assert!(!state.workflow.outer_iterations[0].passed);
    assert_eq!(
        state.workflow.outer_iterations[0]
            .dfm_report
            .as_ref()
            .and_then(|report| report.get("contractType"))
            .and_then(Value::as_str),
        Some("cadastrophe.dfm_report.v1")
    );
    assert_eq!(
        state.workflow.outer_iterations[0]
            .failure_report
            .as_ref()
            .and_then(|report| report.get("contractType"))
            .and_then(Value::as_str),
        Some("cadastrophe.failure_report.v1")
    );
    assert_eq!(state.workflow.pending_vlm.len(), 1);
    assert_eq!(state.workflow.pending_vlm[0].artifact_id, artifact.id);
    assert_eq!(state.workflow.pending_vlm[0].pass_threshold, 0.8);
    assert_eq!(
        state.workflow.pending_vlm[0]
            .dfm_report
            .as_ref()
            .and_then(|report| report.get("contractType"))
            .and_then(Value::as_str),
        Some("cadastrophe.dfm_report.v1")
    );
}

#[test]
fn sqlite_repository_assigns_agent_event_sequence_from_database() {
    let app_data_dir = std::env::temp_dir().join(format!(
        "cadastrophe-run-event-sequence-race-test-{}",
        uuid()
    ));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let app_service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = app_service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let input_revision_id = created.state.session.active_revision_id.clone();
    let (run, _) = app_service
        .create_agent_run(
            &created.session_id,
            "Create a stale writer race fixture.".to_string(),
            input_revision_id,
            Some("codex".to_string()),
            None,
        )
        .unwrap();

    let stale_cli_service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    app_service
        .update_agent_run(
            &created.session_id,
            &run.id,
            None,
            Some(Some("cadastrophe-plan-commit".to_string())),
            None,
            Some(CadBridgeEventType::AgentToolStarted),
            Some(json!({"tool": "cadastrophe-plan-commit"})),
        )
        .unwrap();
    let cli_event = stale_cli_service
        .record_agent_tool_event(
            &created.session_id,
            &run.id,
            None,
            CadAgentRunEventType::AgentToolStarted,
            json!({"command": "cadastrophe-plan-commit", "status": "started"}),
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let events = reloaded
        .get_session_state(&created.session_id)
        .unwrap()
        .agent_run_events
        .into_iter()
        .filter(|event| event.run_id == run.id)
        .collect::<Vec<_>>();
    let sequences = events
        .iter()
        .map(|event| event.sequence)
        .collect::<Vec<_>>();
    assert_eq!(sequences, vec![1, 2, 3]);
    assert_eq!(cli_event.sequence, 3);
}

#[test]
fn refresh_session_from_repository_merges_external_workflow_state() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-workflow-refresh-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let app_service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = app_service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = app_service
        .create_agent_run(
            &created.session_id,
            "Create a workflow refresh fixture.".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let external_cli_service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let plan: CadModelPlan = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/cad_model_plan.v1.json"
    ))
    .unwrap();
    external_cli_service
        .save_workflow_plan(
            &created.session_id,
            CadWorkflowPlan {
                run_id: run.id.clone(),
                revision_id: created.state.session.active_revision_id.clone(),
                source_language: plan.source_language.clone(),
                plan,
                created_at: "2026-07-29T00:00:00.000Z".to_string(),
            },
        )
        .unwrap();
    let revision_id = create_test_revision(
        &external_cli_service,
        &created.session_id,
        "cube([4, 4, 4]);",
    );
    external_cli_service
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();
    let artifact = external_cli_service
        .persist_runtime_artifact(PersistRuntimeArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: revision_id.clone(),
            kind: CadArtifactKind::Stl,
            format: "stl".to_string(),
            contents_base64: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .encode(b"solid refresh\nendsolid refresh\n")
            },
            diagnostics: ok_diagnostics(0),
            metadata: Metadata::new(),
        })
        .unwrap()
        .artifact;
    let queued = external_cli_service
        .create_next_validation_evaluation(CadValidationEvaluationCreate {
            session_id: created.session_id.clone(),
            run_id: run.id.clone(),
            revision_id,
            artifact_id: artifact.id,
            kind: CadValidationEvaluationKind::Vlm,
            input_contract: json!({
                "contractType": "cadastrophe.vlm_evaluation_input.v1"
            }),
            pass_threshold: 0.8,
        })
        .unwrap();

    assert!(app_service
        .get_session_state(&created.session_id)
        .unwrap()
        .workflow
        .plans
        .is_empty());
    assert!(app_service
        .get_session_state(&created.session_id)
        .unwrap()
        .validation_evaluations
        .is_empty());

    let refreshed = app_service
        .refresh_session_from_repository(&created.session_id)
        .unwrap();
    assert_eq!(refreshed.workflow.plans.len(), 1);
    assert_eq!(refreshed.workflow.plans[0].run_id, run.id);
    assert_eq!(refreshed.validation_evaluations.len(), 1);
    assert_eq!(refreshed.validation_evaluations[0].id, queued.id);
    assert_eq!(
        refreshed.validation_evaluations[0].input_contract["evaluationId"],
        queued.id
    );
    assert_eq!(
        refreshed.workflow.plans[0].plan.main_component.name,
        "wall_bracket"
    );
}

#[test]
fn workflow_service_rejects_cross_session_and_missing_references() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-workflow-integrity-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let first = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let second = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let first_revision_id = create_test_revision(&service, &first.session_id, "cube([2, 2, 2]);");
    let second_revision_id = create_test_revision(&service, &second.session_id, "cube([3, 3, 3]);");
    let (run, _) = service
        .create_agent_run(
            &first.session_id,
            "Validate workflow references.".to_string(),
            Some(first_revision_id),
            Some("test-agent".to_string()),
            None,
        )
        .unwrap();
    let plan: CadModelPlan = serde_json::from_str(include_str!(
        "../../../../fixtures/contracts/cad_model_plan.v1.json"
    ))
    .unwrap();
    let error = service
        .save_workflow_plan(
            &first.session_id,
            CadWorkflowPlan {
                run_id: run.id.clone(),
                revision_id: Some(second_revision_id),
                source_language: plan.source_language.clone(),
                plan,
                created_at: "2026-07-29T00:00:00.000Z".to_string(),
            },
        )
        .expect_err("cross-session revision should be rejected");
    assert!(error.contains("does not belong to session"));

    let error = service
        .save_workflow_pending_vlm(
            &first.session_id,
            CadWorkflowPendingVlm {
                run_id: run.id,
                artifact_id: "missing-artifact".to_string(),
                revision_id: None,
                contract: json!({"contractType": "cadastrophe.vlm_judge.v1"}),
                pass_threshold: 0.8,
                structural_report: None,
                dfm_report: None,
                created_at: "2026-07-29T00:00:02.000Z".to_string(),
            },
        )
        .expect_err("missing artifact should be rejected");
    assert!(error.contains("CAD artifact not found"));
}
