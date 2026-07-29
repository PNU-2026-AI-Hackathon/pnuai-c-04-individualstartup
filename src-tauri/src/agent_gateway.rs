use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent};
use crate::protocol::*;
use crate::session_service::SessionService;
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
        "cadastrophe-preview-render",
        "cadastrophe-artifact-export",
        "cadastrophe-evaluate-structural",
        "cadastrophe-finalize",
        "cadastrophe-vlm-submit",
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
