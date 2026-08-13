use super::*;

impl SessionService {
    pub fn create_agent_run_with_user_message(
        &self,
        session_id: &str,
        prompt: String,
        input_revision_id: Option<String>,
        external_agent: Option<String>,
        retry_of_run_id: Option<String>,
        message_metadata: Option<Metadata>,
    ) -> Result<(CadAgentRun, CadConversationMessage, CadSessionState), String> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() {
            return Err("Agent prompt cannot be empty.".to_string());
        }
        let run_id = uuid();
        let message_id = uuid();
        let (run, message, snapshot) =
            {
                let mut state = self.inner.lock().map_err(lock_error)?;
                let session = require_session(&state, session_id)?;
                let resolved_input_revision_id = input_revision_id
                    .clone()
                    .or_else(|| session.active_revision_id.clone());
                if let Some(active) = state
                    .agent_runs
                    .get(session_id)
                    .into_iter()
                    .flatten()
                    .find(|run| !is_terminal_run_status(&run.status))
                {
                    return Err(format!(
                        "Session {session_id} already has an active agent run: {} ({:?}).",
                        active.id, active.status
                    ));
                }
                if let Some(revision_id) = &resolved_input_revision_id {
                    let revision = require_revision(&state, revision_id)?;
                    if revision.session_id != session_id {
                        return Err(format!(
                            "CAD revision {revision_id} does not belong to session {session_id}."
                        ));
                    }
                }
                if let Some(retry_of_run_id) = &retry_of_run_id {
                    let retry_source_exists = state
                        .agent_runs
                        .get(session_id)
                        .into_iter()
                        .flatten()
                        .any(|run| run.id == *retry_of_run_id);
                    if !retry_source_exists {
                        return Err(format!(
                            "Retry source agent run not found: {retry_of_run_id}"
                        ));
                    }
                }

                let now = timestamp();
                let run = CadAgentRun {
                    id: run_id.clone(),
                    session_id: session_id.to_string(),
                    input_revision_id: resolved_input_revision_id.clone(),
                    output_revision_id: None,
                    status: CadAgentRunStatus::Queued,
                    prompt: prompt.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    started_at: None,
                    completed_at: None,
                    error: None,
                    active_step: None,
                    external_agent,
                    agent_thread_id: None,
                    external_thread_id: None,
                    external_turn_id: None,
                    connection_generation: None,
                    recovery_status: CadAgentRecoveryStatus::None,
                };
                let message = CadConversationMessage {
                    id: message_id.clone(),
                    session_id: session_id.to_string(),
                    revision_id: resolved_input_revision_id.clone(),
                    role: CadConversationRole::User,
                    content: prompt.clone(),
                    created_at: now.clone(),
                    run_id: Some(run_id.clone()),
                    external_thread_id: None,
                    external_turn_id: None,
                    external_item_id: None,
                    phase: None,
                    sequence: None,
                    is_final: true,
                    metadata: message_metadata,
                };
                let event = CadAgentRunEvent {
                    id: uuid(),
                    session_id: session_id.to_string(),
                    run_id: run_id.clone(),
                    revision_id: resolved_input_revision_id.clone(),
                    event_type: CadAgentRunEventType::AgentRunCreated,
                    sequence: 1,
                    created_at: now.clone(),
                    payload: metadata_from_value(json!({
                        "status": &run.status,
                        "prompt": &run.prompt,
                        "inputRevisionId": resolved_input_revision_id,
                        "retryOfRunId": retry_of_run_id
                    })),
                    metadata: None,
                };
                let mut staged_session = session.clone();
                if staged_session.title_source != CadSessionTitleSource::User {
                    if let Some(proposed_title) = propose_session_title(&prompt) {
                        if staged_session.title.as_deref() != Some(proposed_title.as_str()) {
                            staged_session.title = Some(proposed_title);
                            staged_session.title_source = CadSessionTitleSource::Agent;
                        }
                    }
                }
                staged_session.updated_at = now;

                let (saved_message, saved_event) = self
                    .repository
                    .create_agent_run_with_user_message(&staged_session, &run, &message, &event)?;
                let session = require_session_mut(&mut state, session_id)?;
                *session = staged_session;
                state
                    .agent_runs
                    .entry(session_id.to_string())
                    .or_default()
                    .push(run.clone());
                state
                    .agent_run_events
                    .entry(session_id.to_string())
                    .or_default()
                    .push(saved_event);
                state
                    .conversation
                    .entry(session_id.to_string())
                    .or_default()
                    .push(saved_message.clone());
                rebuild_revision_summaries(&mut state, session_id);
                let snapshot = build_state(&state, session_id)?;
                (run, saved_message, snapshot)
            };
        self.emit(
            CadBridgeEventType::AgentRunCreated,
            session_id,
            snapshot.clone(),
        );
        self.emit(
            CadBridgeEventType::AgentMessageCreated,
            session_id,
            snapshot.clone(),
        );
        Ok((run, message, snapshot))
    }

    pub fn create_agent_run(
        &self,
        session_id: &str,
        prompt: String,
        input_revision_id: Option<String>,
        external_agent: Option<String>,
        retry_of_run_id: Option<String>,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let run_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let resolved_input_revision_id = {
                let session = require_session(&state, session_id)?;
                input_revision_id.or_else(|| session.active_revision_id.clone())
            };
            if let Some(active) =
                state
                    .agent_runs
                    .get(session_id)
                    .into_iter()
                    .flatten()
                    .find(|run| {
                        matches!(
                            run.status,
                            CadAgentRunStatus::Queued
                                | CadAgentRunStatus::Running
                                | CadAgentRunStatus::WaitingForUser
                        )
                    })
            {
                return Err(format!(
                    "Session {session_id} already has an active agent run: {} ({:?}).",
                    active.id, active.status
                ));
            }
            if let Some(revision_id) = &resolved_input_revision_id {
                let revision = require_revision(&state, revision_id)?;
                if revision.session_id != session_id {
                    return Err(format!(
                        "CAD revision {revision_id} does not belong to session {session_id}."
                    ));
                }
            }
            if let Some(retry_of_run_id) = &retry_of_run_id {
                let retry_source_exists = state
                    .agent_runs
                    .get(session_id)
                    .into_iter()
                    .flatten()
                    .any(|run| run.id == *retry_of_run_id);
                if !retry_source_exists {
                    return Err(format!(
                        "Retry source agent run not found: {retry_of_run_id}"
                    ));
                }
            }
            let now = timestamp();
            let run = CadAgentRun {
                id: run_id.clone(),
                session_id: session_id.to_string(),
                input_revision_id: resolved_input_revision_id.clone(),
                output_revision_id: None,
                status: CadAgentRunStatus::Queued,
                prompt,
                created_at: now.clone(),
                updated_at: now,
                started_at: None,
                completed_at: None,
                error: None,
                active_step: None,
                external_agent,
                agent_thread_id: None,
                external_thread_id: None,
                external_turn_id: None,
                connection_generation: None,
                recovery_status: CadAgentRecoveryStatus::None,
            };
            state
                .agent_runs
                .entry(session_id.to_string())
                .or_default()
                .push(run.clone());
            self.repository.save_agent_run(&run)?;
            let title_updated =
                self.maybe_update_session_title_from_text(&mut state, session_id, &run.prompt)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                &run.id,
                resolved_input_revision_id.clone(),
                CadAgentRunEventType::AgentRunCreated,
                json!({
                    "status": &run.status,
                    "prompt": &run.prompt,
                    "inputRevisionId": resolved_input_revision_id,
                    "retryOfRunId": retry_of_run_id
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = timestamp();
            if title_updated {
                self.persist_session_graph(&state, session_id)?;
            }
            build_state(&state, session_id)?
        };
        let run = snapshot
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
            .ok_or_else(|| "Run write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::AgentRunCreated,
            session_id,
            snapshot.clone(),
        );
        Ok((run, snapshot))
    }

    pub fn link_agent_run_output_revision(
        &self,
        session_id: &str,
        run_id: &str,
        output_revision_id: String,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, &output_revision_id)?;
            if revision.session_id != session_id {
                return Err(format!(
                    "CAD revision {output_revision_id} does not belong to session {session_id}."
                ));
            }
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            run.output_revision_id = Some(output_revision_id.clone());
            run.updated_at = timestamp();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                Some(output_revision_id.clone()),
                CadAgentRunEventType::AgentRunUpdated,
                json!({ "outputRevisionId": output_revision_id }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            rebuild_revision_summaries(&mut state, session_id);
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn update_agent_run(
        &self,
        session_id: &str,
        run_id: &str,
        status: Option<CadAgentRunStatus>,
        active_step: Option<Option<String>>,
        error: Option<String>,
        event_type: Option<CadBridgeEventType>,
        event_payload: Option<Value>,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let now = timestamp();
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            let previous_status = run.status.clone();
            let previous_active_step = run.active_step.clone();
            if let Some(status) = status {
                if status == CadAgentRunStatus::Running && run.started_at.is_none() {
                    run.started_at = Some(now.clone());
                }
                if is_terminal_run_status(&status) && run.completed_at.is_none() {
                    run.completed_at = Some(now.clone());
                }
                run.status = status;
            }
            if let Some(active_step) = active_step {
                run.active_step = active_step;
            }
            if let Some(error) = error.clone() {
                run.error = Some(error);
            }
            run.updated_at = now.clone();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event_type = run_event_type_for_update(event_type.as_ref(), &run.status);
            let mut payload = metadata_from_value(event_payload.unwrap_or_else(|| json!({})));
            payload.insert(
                "previousStatus".to_string(),
                serde_json::to_value(previous_status).map_err(|error| error.to_string())?,
            );
            payload.insert(
                "status".to_string(),
                serde_json::to_value(&run.status).map_err(|error| error.to_string())?,
            );
            if previous_active_step != run.active_step {
                payload.insert(
                    "previousActiveStep".to_string(),
                    serde_json::to_value(previous_active_step)
                        .map_err(|error| error.to_string())?,
                );
                payload.insert(
                    "activeStep".to_string(),
                    serde_json::to_value(&run.active_step).map_err(|error| error.to_string())?,
                );
            }
            if let Some(error) = error {
                payload.insert("error".to_string(), Value::String(error));
            }
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                run.input_revision_id.clone(),
                event_type,
                Value::Object(payload),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = now;
            build_state(&state, session_id)?
        };
        let run = snapshot
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
            .ok_or_else(|| "Run update failed.".to_string())?;
        let event_type = event_type.unwrap_or_else(|| event_type_for_run_status(&run.status));
        self.emit(event_type, session_id, snapshot.clone());
        Ok((run, snapshot))
    }

    pub fn update_agent_run_external_metadata(
        &self,
        session_id: &str,
        run_id: &str,
        external_agent: Option<String>,
        external_thread_id: Option<String>,
        external_turn_id: Option<String>,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            if external_agent.is_some() {
                run.external_agent = external_agent.clone();
            }
            if external_thread_id.is_some() {
                run.external_thread_id = external_thread_id.clone();
            }
            if external_turn_id.is_some() {
                run.external_turn_id = external_turn_id.clone();
            }
            run.updated_at = timestamp();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                run.input_revision_id.clone(),
                CadAgentRunEventType::AgentRunUpdated,
                json!({
                    "externalAgent": external_agent,
                    "externalThreadId": external_thread_id,
                    "externalTurnId": external_turn_id
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn save_workflow_plan(
        &self,
        session_id: &str,
        plan: CadWorkflowPlan,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &plan.run_id)?;
            if let Some(revision_id) = &plan.revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            if plan.plan.source_language != plan.source_language {
                return Err(
                    "Workflow plan source_language must match CadModelPlan source_language."
                        .to_string(),
                );
            }
            self.repository.save_workflow_plan(&plan)?;
            state.workflow_plans.insert(plan.run_id.clone(), plan);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn save_workflow_outer_iteration(
        &self,
        session_id: &str,
        iteration: CadWorkflowOuterIteration,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &iteration.run_id)?;
            if let Some(revision_id) = &iteration.revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            self.repository.save_workflow_outer_iteration(&iteration)?;
            let iterations = state
                .workflow_outer_iterations
                .entry(iteration.run_id.clone())
                .or_default();
            if let Some(existing) = iterations
                .iter_mut()
                .find(|candidate| candidate.id == iteration.id)
            {
                *existing = iteration;
            } else {
                iterations.push(iteration);
            }
            iterations.sort_by(|left, right| {
                left.iteration
                    .cmp(&right.iteration)
                    .then_with(|| left.created_at.cmp(&right.created_at))
            });
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn save_workflow_pending_vlm(
        &self,
        session_id: &str,
        pending_vlm: CadWorkflowPendingVlm,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &pending_vlm.run_id)?;
            validate_artifact_session(&state, session_id, &pending_vlm.artifact_id)?;
            let artifact = state
                .artifacts
                .get(&pending_vlm.artifact_id)
                .ok_or_else(|| format!("CAD artifact not found: {}", pending_vlm.artifact_id))?;
            if let Some(revision_id) = &pending_vlm.revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
                if artifact.revision_id != *revision_id {
                    return Err(format!(
                        "Pending VLM artifact {} belongs to revision {}, not {}.",
                        artifact.id, artifact.revision_id, revision_id
                    ));
                }
            }
            self.repository.save_workflow_pending_vlm(&pending_vlm)?;
            state
                .workflow_pending_vlm
                .insert(pending_vlm.run_id.clone(), pending_vlm);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn clear_workflow_pending_vlm(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, run_id)?;
            self.repository.clear_workflow_pending_vlm(run_id)?;
            state.workflow_pending_vlm.remove(run_id);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn record_agent_tool_event(
        &self,
        session_id: &str,
        run_id: &str,
        revision_id: Option<String>,
        event_type: CadAgentRunEventType,
        payload: Value,
    ) -> Result<CadAgentRunEvent, String> {
        let bridge_event_type = match event_type {
            CadAgentRunEventType::AgentToolStarted => CadBridgeEventType::AgentToolStarted,
            CadAgentRunEventType::AgentToolCompleted => CadBridgeEventType::AgentToolCompleted,
            _ => {
                return Err(
                    "record_agent_tool_event only accepts agent.tool.started/completed."
                        .to_string(),
                )
            }
        };
        let (event, snapshot) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, run_id)?;
            if let Some(revision_id) = &revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                revision_id,
                event_type,
                payload,
                None,
            );
            let event =
                persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = timestamp();
            let snapshot = build_state(&state, session_id)?;
            (event, snapshot)
        };
        self.emit(bridge_event_type, session_id, snapshot);
        Ok(event)
    }

    pub fn revision_prompt_context(
        &self,
        session_id: &str,
        revision_id: Option<&str>,
    ) -> Result<(Option<String>, Option<CadSourceLanguage>, Option<String>), String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let session = require_session(&state, session_id)?;
        let revision_id = revision_id
            .map(ToString::to_string)
            .or_else(|| session.active_revision_id.clone());
        let Some(revision_id) = revision_id else {
            return Ok((None, None, None));
        };
        let revision = require_revision(&state, &revision_id)?;
        if revision.session_id != session_id {
            return Err(format!(
                "CAD revision {revision_id} does not belong to session {session_id}."
            ));
        }
        Ok((
            Some(revision_id),
            Some(revision.source_language.clone()),
            Some(revision.source.clone()),
        ))
    }

    pub fn get_revision(&self, session_id: &str, revision_id: &str) -> Result<CadRevision, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        validate_revision_session(&state, session_id, revision_id)?;
        require_revision(&state, revision_id).cloned()
    }

    pub fn record_agent_failure(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let active_revision_id = require_session(&state, session_id)?
                .active_revision_id
                .clone();
            if let Some(revision_id) = active_revision_id {
                let revision = require_revision_mut(&mut state, &revision_id)?;
                revision.diagnostics.ok = false;
                revision.diagnostics.items.push(CadDiagnostic {
                    severity: "error".to_string(),
                    message: message.to_string(),
                    line: None,
                    column: None,
                });
            }
            let session = require_session_mut(&mut state, session_id)?;
            session.status = CadSessionStatus::Failed;
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, session_id);
            self.persist_session_graph(&state, session_id)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn list_agent_runs(&self, session_id: &str) -> Result<Vec<CadAgentRun>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .agent_runs
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn get_agent_run(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<Option<CadAgentRun>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .agent_runs
            .get(session_id)
            .and_then(|runs| runs.iter().find(|run| run.id == run_id).cloned()))
    }
}
