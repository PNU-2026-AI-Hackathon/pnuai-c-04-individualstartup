use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent};
use crate::protocol::*;
use crate::session_service::SessionService;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub struct AgentGateway {
    service: Arc<SessionService>,
    adapter: Arc<dyn AgentAdapter>,
    active_runs: Arc<Mutex<HashSet<String>>>,
    session_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

impl AgentGateway {
    pub fn new(service: Arc<SessionService>, adapter: Arc<dyn AgentAdapter>) -> Self {
        Self {
            service,
            adapter,
            active_runs: Arc::new(Mutex::new(HashSet::new())),
            session_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_run(&self, input: CreateAgentRunInput) -> Result<CreateAgentRunResult, String> {
        let prompt = input.prompt.trim().to_string();
        if prompt.is_empty() {
            return Err("Agent prompt cannot be empty.".to_string());
        }
        let (run, _) = self.service.create_agent_run(
            &input.session_id,
            prompt.clone(),
            input.revision_id.clone(),
            Some(self.adapter.external_agent().to_string()),
            input.retry_of_run_id.clone(),
        )?;
        let (message, _) = self.service.create_conversation_message(
            &input.session_id,
            run.input_revision_id.clone(),
            CadConversationRole::User,
            prompt,
            Some(run.id.clone()),
            Some(crate::session_service::metadata_from_value(
                serde_json::json!({"source": "web-ui"}),
            )),
        )?;
        self.enqueue(run.clone());
        Ok(CreateAgentRunResult {
            message,
            run,
            state: self.service.get_session_state(&input.session_id)?,
        })
    }

    pub fn list_runs(&self, session_id: &str) -> Result<Vec<CadAgentRun>, String> {
        self.service.list_agent_runs(session_id)
    }

    pub fn get_run(&self, session_id: &str, run_id: &str) -> Result<Option<CadAgentRun>, String> {
        self.service.get_agent_run(session_id, run_id)
    }

    pub fn cancel_run(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let Some(run) = self.service.get_agent_run(session_id, run_id)? else {
            return Err(format!("Agent run not found: {run_id}"));
        };
        if matches!(
            run.status,
            CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
        ) {
            return Ok((run, self.service.get_session_state(session_id)?));
        }
        self.active_runs
            .lock()
            .map_err(|_| "Agent gateway lock is poisoned.".to_string())?
            .remove(run_id);
        self.service.update_agent_run(
            session_id,
            run_id,
            Some(CadAgentRunStatus::Cancelled),
            Some(None),
            None,
            Some(CadBridgeEventType::AgentRunUpdated),
            Some(serde_json::json!({"reason": "cancel_requested"})),
        )
    }

    fn enqueue(&self, run: CadAgentRun) {
        let service = Arc::clone(&self.service);
        let adapter = Arc::clone(&self.adapter);
        let active_runs = Arc::clone(&self.active_runs);
        let session_lock = self.session_lock(&run.session_id);
        tauri::async_runtime::spawn(async move {
            if let Ok(mut active) = active_runs.lock() {
                active.insert(run.id.clone());
            }
            let _session_guard = session_lock.lock().await;
            let result = execute_run(service, adapter, active_runs.clone(), run).await;
            if let Err(error) = result {
                eprintln!("agent gateway failed: {error}");
            }
        });
    }

    fn session_lock(&self, session_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .session_locks
            .lock()
            .expect("agent gateway lock poisoned");
        Arc::clone(
            locks
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }
}

async fn execute_run(
    service: Arc<SessionService>,
    adapter: Arc<dyn AgentAdapter>,
    active_runs: Arc<Mutex<HashSet<String>>>,
    run: CadAgentRun,
) -> Result<(), String> {
    if !is_active(&active_runs, &run.id) {
        return Ok(());
    }
    service.update_agent_run(
        &run.session_id,
        &run.id,
        Some(CadAgentRunStatus::Running),
        Some(Some("Starting agent run".to_string())),
        None,
        None,
        Some(serde_json::json!({"activeStep": "Starting agent run"})),
    )?;
    let (revision_id, revision_source_language, revision_source) =
        service.revision_prompt_context(&run.session_id, run.input_revision_id.as_deref())?;
    let latest_workflow_failure_report =
        latest_workflow_failure_report(&service.get_session_state(&run.session_id)?, &run.id);
    let event_service = Arc::clone(&service);
    let event_run = run.clone();
    let event_active_runs = Arc::clone(&active_runs);
    let event_sink: crate::agent_adapter::AgentAdapterEventSink =
        Arc::new(move |event: AgentAdapterEvent| {
            if !is_active(&event_active_runs, &event_run.id) {
                return Ok(());
            }
            apply_adapter_event(&event_service, &event_run, event)
        });
    let events = adapter
        .run(crate::agent_adapter::AgentAdapterRunInput {
            session_id: run.session_id.clone(),
            run_id: run.id.clone(),
            app_data_dir: service.app_data_dir().to_path_buf(),
            prompt: run.prompt.clone(),
            revision_id,
            revision_source_language,
            revision_source,
            latest_workflow_failure_report,
            event_sink: Some(event_sink),
        })
        .await;

    match events {
        Ok(events) => {
            for event in events {
                if !is_active(&active_runs, &run.id) {
                    return Ok(());
                }
                apply_adapter_event(&service, &run, event)?;
            }
            if is_active(&active_runs, &run.id) {
                service.update_agent_run(
                    &run.session_id,
                    &run.id,
                    Some(CadAgentRunStatus::Completed),
                    Some(None),
                    None,
                    None,
                    Some(serde_json::json!({"status": "completed"})),
                )?;
            }
        }
        Err(error) => {
            service.record_agent_failure(&run.session_id, &error)?;
            service.create_conversation_message(
                &run.session_id,
                None,
                CadConversationRole::System,
                format!("Agent run failed: {error}"),
                Some(run.id.clone()),
                Some(crate::session_service::metadata_from_value(
                    serde_json::json!({"severity": "error"}),
                )),
            )?;
            service.update_agent_run(
                &run.session_id,
                &run.id,
                Some(CadAgentRunStatus::Failed),
                Some(None),
                Some(error.clone()),
                None,
                Some(serde_json::json!({"diagnostic": error})),
            )?;
        }
    }
    if let Ok(mut active) = active_runs.lock() {
        active.remove(&run.id);
    }
    Ok(())
}

fn apply_adapter_event(
    service: &SessionService,
    run: &CadAgentRun,
    event: AgentAdapterEvent,
) -> Result<(), String> {
    match event {
        AgentAdapterEvent::RunMetadata {
            external_agent,
            external_thread_id,
            external_turn_id,
        } => {
            service.update_agent_run_external_metadata(
                &run.session_id,
                &run.id,
                external_agent,
                external_thread_id,
                external_turn_id,
            )?;
        }
        AgentAdapterEvent::MessageCreated {
            role,
            content,
            metadata,
        } => {
            if handle_inline_vlm_judge_report(service, run, &role, &content, metadata.clone())? {
                return Ok(());
            }
            let state = service.get_session_state(&run.session_id)?;
            service.create_conversation_message(
                &run.session_id,
                state.session.active_revision_id,
                role,
                content,
                Some(run.id.clone()),
                metadata,
            )?;
        }
        AgentAdapterEvent::ToolStarted { name } => {
            let tool_name = name.clone();
            service.update_agent_run(
                &run.session_id,
                &run.id,
                None,
                Some(Some(name)),
                None,
                Some(CadBridgeEventType::AgentToolStarted),
                Some(serde_json::json!({"tool": tool_name})),
            )?;
        }
        AgentAdapterEvent::ToolCompleted { name } => {
            let tool_name = name.clone();
            service.update_agent_run(
                &run.session_id,
                &run.id,
                None,
                Some(None),
                None,
                Some(CadBridgeEventType::AgentToolCompleted),
                Some(serde_json::json!({"tool": tool_name})),
            )?;
            if is_cadastrophe_cli_command(&name) {
                service.refresh_session_from_repository(&run.session_id)?;
            }
        }
        AgentAdapterEvent::Progress {
            label,
            message,
            metadata,
        } => {
            let mut payload = serde_json::Map::from_iter([(
                "progressLabel".to_string(),
                serde_json::Value::String(label.clone()),
            )]);
            if let Some(message) = message {
                payload.insert("message".to_string(), serde_json::Value::String(message));
            }
            if let Some(metadata) = metadata {
                payload.insert("metadata".to_string(), serde_json::Value::Object(metadata));
            }
            service.update_agent_run(
                &run.session_id,
                &run.id,
                None,
                Some(Some(label)),
                None,
                Some(CadBridgeEventType::AgentRunUpdated),
                Some(serde_json::Value::Object(payload)),
            )?;
        }
        AgentAdapterEvent::SourceUpdated {
            source_language,
            source,
        } => {
            let state = service.get_session_state(&run.session_id)?;
            let updated = service.update_model_source(UpdateModelSourceInput {
                session_id: run.session_id.clone(),
                source_language,
                source,
                parent_revision_id: state.session.active_revision_id,
                parameters: None,
            })?;
            service.link_agent_run_output_revision(
                &run.session_id,
                &run.id,
                updated.revision_id.clone(),
            )?;
            service.render_preview(RenderPreviewInput {
                session_id: run.session_id.clone(),
                revision_id: Some(updated.revision_id),
            })?;
        }
    }
    Ok(())
}

struct InlineVlmSubmission {
    artifact_id: String,
    passed: bool,
    score: f64,
    pass_threshold: f64,
}

fn handle_inline_vlm_judge_report(
    service: &SessionService,
    run: &CadAgentRun,
    role: &CadConversationRole,
    content: &str,
    metadata: Option<crate::protocol::Metadata>,
) -> Result<bool, String> {
    if role != &CadConversationRole::Assistant {
        return Ok(false);
    }
    let Some(report) = parse_vlm_judge_report(content) else {
        return Ok(false);
    };
    let submission = submit_inline_vlm_judge_report(service, run, report)?;
    let message = if submission.passed {
        format!(
            "VLM accepted final artifact {} with score {:.2} (threshold {:.2}).",
            short_id(&submission.artifact_id),
            submission.score,
            submission.pass_threshold
        )
    } else {
        format!(
            "VLM requested refinement for artifact {} with score {:.2} (threshold {:.2}).",
            short_id(&submission.artifact_id),
            submission.score,
            submission.pass_threshold
        )
    };
    let mut message_metadata = metadata.unwrap_or_default();
    message_metadata.insert(
        "source".to_string(),
        Value::String("codex-inline-vlm-report".to_string()),
    );
    message_metadata.insert("rawVlmReportHidden".to_string(), Value::Bool(true));
    let state = service.get_session_state(&run.session_id)?;
    service.create_conversation_message(
        &run.session_id,
        state.session.active_revision_id,
        CadConversationRole::Assistant,
        message,
        Some(run.id.clone()),
        Some(message_metadata),
    )?;
    Ok(true)
}

fn submit_inline_vlm_judge_report(
    service: &SessionService,
    run: &CadAgentRun,
    report: Value,
) -> Result<InlineVlmSubmission, String> {
    let state = service.get_session_state(&run.session_id)?;
    let pending = state
        .workflow
        .pending_vlm
        .iter()
        .find(|pending| pending.run_id == run.id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Received VLM judge report for run {}, but no pending VLM contract exists.",
                run.id
            )
        })?;
    validate_inline_vlm_report(&report, &run.id, &pending.artifact_id)?;
    let score = report
        .get("score")
        .and_then(Value::as_f64)
        .ok_or_else(|| "VLM judge report missing numeric score.".to_string())?;
    if !(0.0..=1.0).contains(&score) {
        return Err("VLM judge report score must be between 0.0 and 1.0.".to_string());
    }
    let judge_passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "VLM judge report missing boolean passed field.".to_string())?;
    let passed = judge_passed && score >= pending.pass_threshold;
    let revision_id = pending
        .contract
        .get("revisionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let structural_report = pending
        .contract
        .get("structuralReport")
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "contractType": "cadastrophe.structural_report.v1",
                "runId": run.id,
                "artifactId": pending.artifact_id,
                "passed": true,
                "checks": []
            })
        });
    let failure_report = if passed {
        None
    } else {
        Some(vlm_failure_report(&report, score, pending.pass_threshold))
    };
    let next_iteration = state
        .workflow
        .outer_iterations
        .iter()
        .filter(|iteration| iteration.run_id == run.id)
        .map(|iteration| iteration.iteration)
        .max()
        .unwrap_or(0)
        + 1;

    service.record_agent_tool_event(
        &run.session_id,
        &run.id,
        revision_id.clone(),
        CadAgentRunEventType::AgentToolStarted,
        json!({
            "phase": "vlm-judge-callback",
            "status": "started",
            "source": "codex-inline-vlm-report"
        }),
    )?;
    service.save_workflow_outer_iteration(
        &run.session_id,
        CadWorkflowOuterIteration {
            id: format!("workflow-outer-{}-{next_iteration}", run.id),
            run_id: run.id.clone(),
            iteration: next_iteration,
            revision_id: revision_id.clone(),
            structural_report,
            vlm_report: Some(report),
            failure_report,
            passed,
            created_at: pending.created_at.clone(),
        },
    )?;
    let workflow = service.clear_workflow_pending_vlm(&run.session_id, &run.id)?;
    service.record_agent_tool_event(
        &run.session_id,
        &run.id,
        revision_id,
        CadAgentRunEventType::AgentToolCompleted,
        json!({
            "phase": "vlm-judge-callback",
            "status": "completed",
            "ok": true,
            "artifactId": pending.artifact_id,
            "passed": passed,
            "score": score,
            "passThreshold": pending.pass_threshold,
            "nextAction": if passed { "complete" } else { "outer_loop_refine_source" },
            "pendingVlm": workflow.pending_vlm.len()
        }),
    )?;
    if passed {
        let _ = service.update_agent_run(
            &run.session_id,
            &run.id,
            Some(CadAgentRunStatus::Completed),
            Some(None),
            None,
            None,
            Some(json!({
                "artifactId": pending.artifact_id,
                "nextAction": "complete",
                "vlmPassed": true
            })),
        )?;
    }
    Ok(InlineVlmSubmission {
        artifact_id: pending.artifact_id,
        passed,
        score,
        pass_threshold: pending.pass_threshold,
    })
}

fn parse_vlm_judge_report(content: &str) -> Option<Value> {
    let value = parse_json_object(content)?;
    (value.get("contractType").and_then(Value::as_str) == Some("cadastrophe.vlm_judge_report.v1"))
        .then_some(value)
}

fn parse_json_object(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    if let Ok(value @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let fenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))?
        .trim();
    let fenced = fenced.strip_suffix("```")?.trim();
    match serde_json::from_str::<Value>(fenced).ok()? {
        value @ Value::Object(_) => Some(value),
        _ => None,
    }
}

fn validate_inline_vlm_report(
    report: &Value,
    run_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    if report.get("runId").and_then(Value::as_str) != Some(run_id) {
        return Err("VLM judge report runId does not match pending VLM.".to_string());
    }
    if report.get("artifactId").and_then(Value::as_str) != Some(artifact_id) {
        return Err("VLM judge report artifactId does not match pending VLM.".to_string());
    }
    Ok(())
}

fn vlm_failure_report(report: &Value, score: f64, pass_threshold: f64) -> Value {
    json!({
        "contractType": "cadastrophe.failure_report.v1",
        "reason": "vlm_judge_failed",
        "summary": report
            .get("diagnostic")
            .and_then(Value::as_str)
            .unwrap_or("VLM judge rejected the artifact."),
        "score": score,
        "passThreshold": pass_threshold,
        "vlmReport": report,
        "nextAction": "outer_loop_refine_source",
        "next_action": "outer_loop_refine_source"
    })
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn is_active(active_runs: &Arc<Mutex<HashSet<String>>>, run_id: &str) -> bool {
    active_runs
        .lock()
        .map(|active| active.contains(run_id))
        .unwrap_or(false)
}

fn is_cadastrophe_cli_command(name: &str) -> bool {
    [
        "cadastrophe-session-current",
        "cadastrophe-session-state",
        "cadastrophe-plan-commit",
        "cadastrophe-source-apply",
        "cadastrophe-finalize",
    ]
    .iter()
    .any(|command| name.contains(command))
}

fn latest_workflow_failure_report(
    state: &CadSessionState,
    current_run_id: &str,
) -> Option<serde_json::Value> {
    state
        .workflow
        .outer_iterations
        .iter()
        .filter(|iteration| iteration.run_id != current_run_id)
        .filter_map(|iteration| {
            iteration
                .failure_report
                .as_ref()
                .map(|report| (iteration.created_at.as_str(), report))
        })
        .max_by(|left, right| left.0.cmp(right.0))
        .map(|(_, report)| report.clone())
}
