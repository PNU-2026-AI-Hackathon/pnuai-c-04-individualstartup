use super::*;

impl SessionService {
    pub fn list_agent_threads(&self, session_id: &str) -> Result<Vec<CadAgentThread>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .agent_threads
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn get_active_agent_thread(
        &self,
        scope: &ThreadScope,
        external_agent: &str,
    ) -> Result<Option<CadAgentThread>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        validate_thread_scope(scope)?;
        require_session(&state, &scope.session_id)?;
        let mut active = state
            .agent_threads
            .get(&scope.session_id)
            .into_iter()
            .flatten()
            .filter(|thread| {
                thread.external_agent == external_agent
                    && thread.plane == scope.plane
                    && thread.owner_id == scope.owner_id
                    && thread.archived_at.is_none()
                    && thread.replaced_by_id.is_none()
            });
        let result = active.next().cloned();
        if active.next().is_some() {
            return Err(format!(
                "Multiple active agent threads exist for scope {:?} and agent {external_agent}.",
                scope
            ));
        }
        Ok(result)
    }

    pub fn upsert_agent_thread(&self, thread: CadAgentThread) -> Result<CadAgentThread, String> {
        validate_agent_thread_fields(&thread)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, &thread.session_id)?;
        if thread.plane == CadAgentPlane::Validation {
            let evaluation = state
                .validation_evaluations
                .get(&thread.session_id)
                .into_iter()
                .flatten()
                .find(|evaluation| evaluation.id == thread.owner_id)
                .ok_or_else(|| {
                    format!(
                        "Validation agent thread owner evaluation not found: {}",
                        thread.owner_id
                    )
                })?;
            if matches!(
                evaluation.status,
                CadValidationEvaluationStatus::Succeeded | CadValidationEvaluationStatus::Failed
            ) {
                return Err(format!(
                    "Validation agent thread cannot be attached to terminal evaluation: {}",
                    thread.owner_id
                ));
            }
        }
        let threads = state
            .agent_threads
            .entry(thread.session_id.clone())
            .or_default();
        if thread.archived_at.is_none()
            && thread.replaced_by_id.is_none()
            && threads.iter().any(|candidate| {
                candidate.id != thread.id
                    && candidate.external_agent == thread.external_agent
                    && candidate.plane == thread.plane
                    && candidate.archived_at.is_none()
                    && candidate.replaced_by_id.is_none()
            })
        {
            return Err(format!(
                "Session {} already has an active {} agent thread.",
                thread.session_id, thread.external_agent
            ));
        }
        if let Some(replaced_by_id) = &thread.replaced_by_id {
            let replacement = threads
                .iter()
                .find(|candidate| candidate.id == *replaced_by_id)
                .ok_or_else(|| format!("Replacement agent thread not found: {replaced_by_id}"))?;
            if replacement.session_id != thread.session_id
                || replacement.external_agent != thread.external_agent
                || replacement.plane != thread.plane
                || replacement.owner_id != thread.owner_id
            {
                return Err(format!(
                    "Replacement agent thread {replaced_by_id} belongs to a different session or agent."
                ));
            }
        }
        self.repository.save_agent_thread(&thread)?;
        if let Some(existing) = threads
            .iter_mut()
            .find(|candidate| candidate.id == thread.id)
        {
            *existing = thread.clone();
        } else {
            threads.push(thread.clone());
        }
        threads.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(thread)
    }

    pub fn bind_agent_run_to_thread(
        &self,
        session_id: &str,
        run_id: &str,
        agent_thread_id: &str,
        external_turn_id: Option<String>,
        connection_generation: Option<u64>,
        recovery_status: CadAgentRecoveryStatus,
    ) -> Result<CadAgentRun, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        let thread = state
            .agent_threads
            .get(session_id)
            .into_iter()
            .flatten()
            .find(|thread| thread.id == agent_thread_id)
            .cloned()
            .ok_or_else(|| format!("Agent thread not found: {agent_thread_id}"))?;
        if thread.archived_at.is_some() || thread.replaced_by_id.is_some() {
            return Err(format!(
                "Cannot bind run {run_id} to inactive agent thread {agent_thread_id}."
            ));
        }
        if thread.plane != CadAgentPlane::Modeling || thread.owner_id != session_id {
            return Err(format!(
                "Agent run {run_id} can only bind to the session's modeling thread."
            ));
        }
        let run = require_agent_run_mut(&mut state, session_id, run_id)?;
        if matches!(
            run.status,
            CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
        ) {
            return Err(format!(
                "Cannot bind terminal agent run {run_id} to a Codex thread/turn."
            ));
        }
        if let Some(existing_thread_id) = &run.agent_thread_id {
            if existing_thread_id != agent_thread_id {
                return Err(format!(
                    "Agent run {run_id} is already bound to thread {existing_thread_id}."
                ));
            }
        }
        if let (Some(existing_turn_id), Some(turn_id)) =
            (&run.external_turn_id, external_turn_id.as_ref())
        {
            if existing_turn_id != turn_id {
                return Err(format!(
                    "Agent run {run_id} is already bound to turn {existing_turn_id}."
                ));
            }
        }
        run.agent_thread_id = Some(thread.id);
        run.external_agent = Some(thread.external_agent);
        run.external_thread_id = Some(thread.external_thread_id);
        if external_turn_id.is_some() {
            run.external_turn_id = external_turn_id;
        }
        run.connection_generation = connection_generation;
        run.recovery_status = recovery_status;
        run.updated_at = timestamp();
        let run = run.clone();
        self.repository.save_agent_run(&run)?;
        Ok(run)
    }

    pub fn upsert_agent_conversation_message(
        &self,
        message: CadConversationMessage,
    ) -> Result<CadConversationMessage, String> {
        if message
            .external_item_id
            .as_deref()
            .is_none_or(str::is_empty)
        {
            return Err("Agent conversation message requires external_item_id.".to_string());
        }
        if message
            .external_thread_id
            .as_deref()
            .is_none_or(str::is_empty)
            || message
                .external_turn_id
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(
                "Agent conversation message requires external_thread_id and external_turn_id."
                    .to_string(),
            );
        }
        let run_id = message
            .run_id
            .as_deref()
            .ok_or_else(|| "Agent conversation message requires run_id.".to_string())?;
        let session_id = message.session_id.clone();
        let (saved, snapshot) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, &session_id)?;
            let run = state
                .agent_runs
                .get(&message.session_id)
                .into_iter()
                .flatten()
                .find(|run| run.id == run_id)
                .ok_or_else(|| format!("Agent run not found: {run_id}"))?;
            if run.external_thread_id != message.external_thread_id
                || run.external_turn_id != message.external_turn_id
            {
                return Err(format!(
                    "Conversation message external thread/turn does not match agent run {run_id}."
                ));
            }
            let saved = self.repository.save_conversation_message(&message)?;
            let messages = state
                .conversation
                .entry(message.session_id.clone())
                .or_default();
            if let Some(existing) = messages.iter_mut().find(|candidate| {
                candidate.external_thread_id == saved.external_thread_id
                    && candidate.external_turn_id == saved.external_turn_id
                    && candidate.external_item_id == saved.external_item_id
            }) {
                *existing = saved.clone();
            } else {
                messages.push(saved.clone());
            }
            messages.sort_by(|left, right| {
                left.sequence
                    .cmp(&right.sequence)
                    .then_with(|| left.created_at.cmp(&right.created_at))
                    .then_with(|| left.id.cmp(&right.id))
            });
            let snapshot = build_state(&state, &session_id)?;
            (saved, snapshot)
        };
        self.emit(
            CadBridgeEventType::AgentMessageCreated,
            &session_id,
            snapshot,
        );
        Ok(saved)
    }

    pub fn save_agent_transport_event(
        &self,
        event: CadAgentTransportEvent,
    ) -> Result<CadAgentTransportEvent, String> {
        if event.method.trim().is_empty() {
            return Err("Agent transport event method cannot be empty.".to_string());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, &event.session_id)?;
        if let Some(run_id) = &event.run_id {
            let run = state
                .agent_runs
                .get(&event.session_id)
                .into_iter()
                .flatten()
                .find(|run| run.id == *run_id)
                .ok_or_else(|| format!("Agent run not found: {run_id}"))?;
            if event.agent_thread_id.is_some() && run.agent_thread_id != event.agent_thread_id {
                return Err(format!(
                    "Transport event thread does not match agent run {run_id}."
                ));
            }
            if event.external_turn_id.is_some() && run.external_turn_id != event.external_turn_id {
                return Err(format!(
                    "Transport event turn does not match agent run {run_id}."
                ));
            }
        }
        if let Some(agent_thread_id) = &event.agent_thread_id {
            let exists = state
                .agent_threads
                .get(&event.session_id)
                .into_iter()
                .flatten()
                .any(|thread| thread.id == *agent_thread_id);
            if !exists {
                return Err(format!("Agent thread not found: {agent_thread_id}"));
            }
        }
        let saved = self.repository.save_agent_transport_event(&event)?;
        let events = state
            .agent_transport_events
            .entry(event.session_id.clone())
            .or_default();
        if let Some(existing) = events.iter().find(|candidate| candidate.id == saved.id) {
            if existing != &saved {
                return Err(format!(
                    "Transport event id was replayed with different in-memory content: {}",
                    saved.id
                ));
            }
        } else {
            events.push(saved.clone());
            events.sort_by(|left, right| {
                left.sequence
                    .cmp(&right.sequence)
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
        Ok(saved)
    }
}

pub(super) fn validate_agent_thread_fields(thread: &CadAgentThread) -> Result<(), String> {
    if thread.id.trim().is_empty()
        || thread.session_id.trim().is_empty()
        || thread.external_agent.trim().is_empty()
        || thread.external_thread_id.trim().is_empty()
    {
        return Err("Agent thread identifiers cannot be empty.".to_string());
    }
    validate_thread_scope(&ThreadScope {
        session_id: thread.session_id.clone(),
        plane: thread.plane.clone(),
        owner_id: thread.owner_id.clone(),
    })?;
    if thread.replaced_by_id.as_deref() == Some(thread.id.as_str()) {
        return Err(format!("Agent thread cannot replace itself: {}", thread.id));
    }
    if thread.replaced_by_id.is_some() && thread.archived_at.is_none() {
        return Err(format!(
            "Replaced agent thread must have archived_at: {}",
            thread.id
        ));
    }
    Ok(())
}

fn validate_thread_scope(scope: &ThreadScope) -> Result<(), String> {
    if scope.session_id.trim().is_empty() || scope.owner_id.trim().is_empty() {
        return Err("Agent thread scope identifiers cannot be empty.".to_string());
    }
    if scope.plane == CadAgentPlane::Modeling && scope.owner_id != scope.session_id {
        return Err("Modeling agent thread owner_id must equal session_id.".to_string());
    }
    Ok(())
}
