use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent};
use crate::protocol::*;
use crate::session_service::{timestamp, SessionService};
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
        let (run, message, state) = self.service.create_agent_run_with_user_message(
            &input.session_id,
            prompt.clone(),
            input.revision_id.clone(),
            Some(self.adapter.external_agent().to_string()),
            input.retry_of_run_id.clone(),
            Some(crate::session_service::metadata_from_value(
                serde_json::json!({"source": "web-ui"}),
            )),
        )?;
        self.enqueue(run.clone());
        Ok(CreateAgentRunResult {
            message,
            run,
            state,
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
        if run.external_turn_id.is_none() {
            self.active_runs
                .lock()
                .map_err(|_| "Agent gateway lock is poisoned.".to_string())?
                .remove(run_id);
            return self.service.update_agent_run(
                session_id,
                run_id,
                Some(CadAgentRunStatus::Cancelled),
                Some(None),
                None,
                Some(CadBridgeEventType::AgentRunUpdated),
                Some(serde_json::json!({"reason": "cancelled_before_turn_start"})),
            );
        }
        let adapter = Arc::clone(&self.adapter);
        let service = Arc::clone(&self.service);
        let active_runs = Arc::clone(&self.active_runs);
        let session_id_owned = session_id.to_string();
        let run_id_owned = run_id.to_string();
        let reconciling = self.service.mark_agent_run_reconciling(
            session_id,
            run_id,
            "Cancellation requested; waiting for Codex terminal state or history reconciliation."
                .to_string(),
        )?;
        tauri::async_runtime::spawn(async move {
            let result = adapter
                .interrupt_run(&session_id_owned, &run_id_owned)
                .await;
            if let Err(error) = result {
                if let Err(persist_error) = service.mark_agent_run_unknown_outcome(
                    &session_id_owned,
                    &run_id_owned,
                    format!("Cancel interrupt/reconciliation failed; outcome is unknown and run was not retried: {error}"),
                ) {
                    eprintln!(
                        "[cadastrophe:cancel-recovery] session_id={} run_id={} interrupt_error={error:?} persist_error={persist_error:?}",
                        session_id_owned, run_id_owned
                    );
                }
            }
            match active_runs.lock() {
                Ok(mut active) => {
                    active.remove(&run_id_owned);
                }
                Err(_) => eprintln!(
                    "[cadastrophe:cancel-recovery] session_id={} run_id={} active run lock poisoned during cleanup",
                    session_id_owned, run_id_owned
                ),
            }
        });
        Ok((reconciling, self.service.get_session_state(session_id)?))
    }

    pub fn recover_startup_runs(&self) -> Result<(), String> {
        for candidate in self.service.list_startup_agent_run_recovery_candidates()? {
            match candidate.action {
                CadAgentRunRecoveryAction::Reenqueue => {
                    let run = self
                        .service
                        .get_agent_run(&candidate.session_id, &candidate.run_id)?
                        .ok_or_else(|| {
                            format!("Startup recovery run not found: {}", candidate.run_id)
                        })?;
                    self.enqueue(run);
                }
                CadAgentRunRecoveryAction::QueryHistory => {
                    self.service.mark_agent_run_reconciling(
                        &candidate.session_id,
                        &candidate.run_id,
                        "Reconciling persisted Codex turn during application startup.".to_string(),
                    )?;
                    let adapter = Arc::clone(&self.adapter);
                    let service = Arc::clone(&self.service);
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) = adapter
                            .reconcile_run(&candidate.session_id, &candidate.run_id)
                            .await
                        {
                            if let Err(persist_error) = service.mark_agent_run_unknown_outcome(
                                &candidate.session_id,
                                &candidate.run_id,
                                format!("Startup Codex history reconciliation failed; outcome is unknown and run was not retried: {error}"),
                            ) {
                                eprintln!(
                                    "[cadastrophe:startup-recovery] failed to persist unknown outcome for {}/{}: {persist_error}",
                                    candidate.session_id, candidate.run_id
                                );
                            }
                        }
                    });
                }
                CadAgentRunRecoveryAction::MarkUnknownOutcome => {
                    self.service.mark_agent_run_unknown_outcome(
                        &candidate.session_id,
                        &candidate.run_id,
                        "Application restarted before this run received a Codex turn id; outcome cannot be determined and the run was not retried.".to_string(),
                    )?;
                }
            }
        }
        Ok(())
    }

    fn enqueue(&self, run: CadAgentRun) {
        let service = Arc::clone(&self.service);
        let adapter = Arc::clone(&self.adapter);
        let active_runs = Arc::clone(&self.active_runs);
        let session_lock = self.session_lock(&run.session_id);
        active_runs
            .lock()
            .expect("agent gateway active run lock poisoned")
            .insert(run.id.clone());
        tauri::async_runtime::spawn(async move {
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
            if is_active(&active_runs, &run.id)
                && !service
                    .get_agent_run(&run.session_id, &run.id)?
                    .is_some_and(|current| is_terminal_run_status(&current.status))
            {
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
            if service
                .get_agent_run(&run.session_id, &run.id)?
                .is_some_and(|current| is_terminal_run_status(&current.status))
            {
                remove_active_run(&active_runs, &run.id)?;
                return Ok(());
            }
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
    remove_active_run(&active_runs, &run.id)?;
    Ok(())
}

fn remove_active_run(
    active_runs: &Arc<Mutex<HashSet<String>>>,
    run_id: &str,
) -> Result<(), String> {
    active_runs
        .lock()
        .map_err(|_| "Agent gateway active run lock is poisoned during cleanup.".to_string())?
        .remove(run_id);
    Ok(())
}

fn is_terminal_run_status(status: &CadAgentRunStatus) -> bool {
    matches!(
        status,
        CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
    )
}

pub(crate) fn apply_adapter_event(
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
        AgentAdapterEvent::AgentMessageDelta {
            external_thread_id,
            external_turn_id,
            external_item_id,
            phase,
            delta,
            sequence,
        } => {
            service.emit_agent_stream(CadAgentStreamEvent {
                session_id: run.session_id.clone(),
                run_id: run.id.clone(),
                thread_id: external_thread_id,
                turn_id: external_turn_id,
                item_id: external_item_id,
                phase,
                delta,
                sequence,
                completed: false,
            })?;
        }
        AgentAdapterEvent::AgentMessageCompleted {
            external_thread_id,
            external_turn_id,
            external_item_id,
            phase,
            content,
            sequence,
            is_final,
            metadata,
        } => {
            let mut content = content;
            let mut metadata = metadata.unwrap_or_default();
            if let Some(report) = parse_vlm_judge_report(&content) {
                let submission = submit_inline_vlm_judge_report(service, run, report)?;
                content = if submission.passed {
                    format!(
                        "VLM accepted final artifact {} with score {:.2} (threshold {:.2}).",
                        short_id(&submission.artifact_id),
                        submission.score,
                        submission.pass_threshold
                    )
                } else {
                    format!(
                        "VLM requested refinement for artifact {} with score {:.2} (threshold {:.2}).",
                        short_id(&submission.artifact_id), submission.score, submission.pass_threshold
                    )
                };
                metadata.insert(
                    "source".to_string(),
                    Value::String("codex-inline-vlm-report".to_string()),
                );
                metadata.insert("rawVlmReportHidden".to_string(), Value::Bool(true));
            }
            let state = service.get_session_state(&run.session_id)?;
            service.upsert_agent_conversation_message(CadConversationMessage {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: run.session_id.clone(),
                revision_id: state.session.active_revision_id,
                role: CadConversationRole::Assistant,
                content,
                created_at: timestamp(),
                run_id: Some(run.id.clone()),
                external_thread_id: Some(external_thread_id.clone()),
                external_turn_id: Some(external_turn_id.clone()),
                external_item_id: Some(external_item_id.clone()),
                phase: Some(phase.clone()),
                sequence: Some(sequence),
                is_final,
                metadata: Some(metadata),
            })?;
            // upsert_agent_conversation_message synchronously emits the durable
            // AgentMessageCreated snapshot before this ephemeral tombstone.
            service.emit_agent_stream(CadAgentStreamEvent {
                session_id: run.session_id.clone(),
                run_id: run.id.clone(),
                thread_id: external_thread_id,
                turn_id: external_turn_id,
                item_id: external_item_id,
                phase,
                delta: String::new(),
                sequence,
                completed: true,
            })?;
        }
        AgentAdapterEvent::TransportNotification {
            agent_thread_id,
            external_turn_id,
            external_item_id,
            method,
            sequence,
            payload,
        } => {
            service.save_agent_transport_event(CadAgentTransportEvent {
                id: format!(
                    "transport:{}:{}:{}:{}",
                    run.id, agent_thread_id, external_turn_id, sequence
                ),
                session_id: run.session_id.clone(),
                run_id: Some(run.id.clone()),
                agent_thread_id: Some(agent_thread_id),
                external_turn_id: Some(external_turn_id),
                external_item_id,
                method,
                sequence,
                payload: normalize_transport_payload(&payload),
                created_at: timestamp(),
            })?;
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
            let normalized_label = normalize_transport_value(&Value::String(label), 0).0;
            let normalized_label = normalized_label
                .as_str()
                .ok_or_else(|| "Normalized progress label must remain a string.".to_string())?
                .to_string();
            let mut payload = serde_json::Map::from_iter([(
                "progressLabel".to_string(),
                serde_json::Value::String(normalized_label.clone()),
            )]);
            if let Some(message) = message {
                payload.insert(
                    "message".to_string(),
                    normalize_transport_value(&Value::String(message), 0).0,
                );
            }
            if let Some(metadata) = metadata {
                payload.insert(
                    "metadata".to_string(),
                    normalize_transport_payload(&Value::Object(metadata)),
                );
            }
            service.update_agent_run(
                &run.session_id,
                &run.id,
                None,
                Some(Some(normalized_label)),
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

const MAX_TRANSPORT_STRING_CHARS: usize = 4_096;
const MAX_TRANSPORT_COLLECTION_ITEMS: usize = 64;

pub(crate) fn normalize_transport_payload(payload: &Value) -> Value {
    let mut structurally_redacted = payload.clone();
    let hidden_content_redacted = redact_hidden_transport_content(&mut structurally_redacted);
    let (mut value, truncated, recursively_redacted) =
        normalize_transport_value(&structurally_redacted, 0);
    let redacted = hidden_content_redacted || recursively_redacted;
    if let Value::Object(object) = &mut value {
        object.insert(
            "_cadastropheTransportPolicy".to_string(),
            json!({ "truncated": truncated, "redacted": redacted }),
        );
    } else {
        value = json!({
            "value": value,
            "_cadastropheTransportPolicy": { "truncated": truncated, "redacted": redacted }
        });
    }
    value
}

fn redact_hidden_transport_content(payload: &mut Value) -> bool {
    if payload.get("method").and_then(Value::as_str) == Some("item/reasoning/textDelta") {
        if let Some(params) = payload.get_mut("params").and_then(Value::as_object_mut) {
            if params.contains_key("delta") {
                params.insert("delta".to_string(), Value::String("[redacted]".to_string()));
                return true;
            }
        }
    }
    let Some(item) = payload
        .get_mut("params")
        .and_then(|params| params.get_mut("item"))
        .and_then(Value::as_object_mut)
    else {
        return false;
    };
    let item_type = item.get("type").and_then(Value::as_str);
    if !matches!(item_type, Some("reasoning" | "userMessage")) {
        return false;
    }
    if item.contains_key("content") {
        item.insert(
            "content".to_string(),
            Value::String("[redacted]".to_string()),
        );
        true
    } else {
        false
    }
}

fn normalize_transport_value(value: &Value, depth: usize) -> (Value, bool, bool) {
    if depth >= 12 {
        return (
            Value::String("[truncated: maximum nesting depth]".to_string()),
            true,
            false,
        );
    }
    match value {
        Value::String(text) => {
            let mut chars = text.chars();
            let prefix = chars
                .by_ref()
                .take(MAX_TRANSPORT_STRING_CHARS)
                .collect::<String>();
            if chars.next().is_some() {
                (Value::String(format!("{prefix}\n[truncated]")), true, false)
            } else {
                (value.clone(), false, false)
            }
        }
        Value::Array(values) => {
            let mut truncated = values.len() > MAX_TRANSPORT_COLLECTION_ITEMS;
            let mut redacted = false;
            let normalized = values
                .iter()
                .take(MAX_TRANSPORT_COLLECTION_ITEMS)
                .map(|value| {
                    let (value, was_truncated, was_redacted) =
                        normalize_transport_value(value, depth + 1);
                    truncated |= was_truncated;
                    redacted |= was_redacted;
                    value
                })
                .collect();
            (Value::Array(normalized), truncated, redacted)
        }
        Value::Object(values) => {
            let mut truncated = values.len() > MAX_TRANSPORT_COLLECTION_ITEMS;
            let mut redacted = false;
            let mut normalized = serde_json::Map::new();
            for (key, value) in values.iter().take(MAX_TRANSPORT_COLLECTION_ITEMS) {
                if is_sensitive_transport_key(key) {
                    normalized.insert(key.clone(), Value::String("[redacted]".to_string()));
                    redacted = true;
                    continue;
                }
                let (value, was_truncated, was_redacted) =
                    normalize_transport_value(value, depth + 1);
                truncated |= was_truncated;
                redacted |= was_redacted;
                normalized.insert(key.clone(), value);
            }
            (Value::Object(normalized), truncated, redacted)
        }
        _ => (value.clone(), false, false),
    }
}

fn is_sensitive_transport_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "environment",
        "env",
        "secret",
        "token",
        "password",
        "authorization",
        "cookie",
        "prompt",
    ]
    .iter()
    .any(|sensitive| normalized == *sensitive || normalized.ends_with(sensitive))
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
    mut report: Value,
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
    let report_object = report
        .as_object_mut()
        .ok_or_else(|| "VLM judge report must be a JSON object.".to_string())?;
    report_object
        .entry("runId".to_string())
        .or_insert_with(|| Value::String(run.id.clone()));
    report_object
        .entry("artifactId".to_string())
        .or_insert_with(|| Value::String(pending.artifact_id.clone()));
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
    let revision_id = pending.revision_id.clone();
    let structural_report = pending.structural_report.clone().unwrap_or_else(|| {
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
            dfm_report: pending.dfm_report.clone(),
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
        service.update_agent_run(
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
    if let Some(value) = report.get("runId") {
        match value.as_str() {
            Some(value) if value == run_id => {}
            _ => return Err("VLM judge report runId does not match pending VLM.".to_string()),
        }
    }
    if let Some(value) = report.get("artifactId") {
        match value.as_str() {
            Some(value) if value == artifact_id => {}
            _ => return Err("VLM judge report artifactId does not match pending VLM.".to_string()),
        }
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
