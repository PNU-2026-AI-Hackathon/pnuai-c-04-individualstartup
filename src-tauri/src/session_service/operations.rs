use super::agent_persistence::validate_agent_thread_fields;
use super::*;

impl SessionService {
    pub fn prepare_agent_thread_replacement(
        &self,
        session_id: &str,
        external_agent: &str,
    ) -> Result<CadAgentThreadReplacementPreparation, String> {
        if external_agent.trim().is_empty() {
            return Err("External agent cannot be empty.".to_string());
        }
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        reject_all_active_agent_runs(&state, session_id)?;
        let mut active_threads = state
            .agent_threads
            .get(session_id)
            .into_iter()
            .flatten()
            .filter(|thread| {
                thread.external_agent == external_agent
                    && thread.archived_at.is_none()
                    && thread.replaced_by_id.is_none()
            });
        let active_thread = active_threads.next().cloned();
        if active_threads.next().is_some() {
            return Err(format!(
                "Multiple active agent threads exist for session {session_id} and agent {external_agent}."
            ));
        }
        Ok(CadAgentThreadReplacementPreparation {
            session_id: session_id.to_string(),
            external_agent: external_agent.to_string(),
            active_thread,
        })
    }

    pub fn replace_active_agent_thread(
        &self,
        old_thread_id: &str,
        replacement: CadAgentThread,
        reason: String,
        allowed_run_id: Option<&str>,
    ) -> Result<CadAgentThreadReplacementResult, String> {
        validate_agent_thread_fields(&replacement)?;
        if reason.trim().is_empty() {
            return Err("Agent thread replacement reason cannot be empty.".to_string());
        }
        if replacement.id == old_thread_id {
            return Err(format!(
                "Replacement agent thread must have a new id: {old_thread_id}"
            ));
        }
        if replacement.archived_at.is_some() || replacement.replaced_by_id.is_some() {
            return Err(format!(
                "Replacement agent thread must be active and unreplaced: {}",
                replacement.id
            ));
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        let old_thread = state
            .agent_threads
            .values()
            .flatten()
            .find(|thread| thread.id == old_thread_id)
            .cloned()
            .ok_or_else(|| format!("Agent thread not found: {old_thread_id}"))?;
        if old_thread.archived_at.is_some() || old_thread.replaced_by_id.is_some() {
            return Err(format!("Agent thread is not active: {old_thread_id}"));
        }
        if old_thread.session_id != replacement.session_id
            || old_thread.external_agent != replacement.external_agent
        {
            return Err(
                "Replacement agent thread must belong to the same session and external agent."
                    .to_string(),
            );
        }
        require_session(&state, &old_thread.session_id)?;
        validate_replacement_active_runs(&state, &old_thread, allowed_run_id)?;
        let session_threads = state
            .agent_threads
            .get(&old_thread.session_id)
            .ok_or_else(|| {
                format!(
                    "Agent thread state is missing for {}",
                    old_thread.session_id
                )
            })?;
        if session_threads
            .iter()
            .any(|thread| thread.id == replacement.id)
        {
            return Err(format!(
                "Replacement agent thread id already exists: {}",
                replacement.id
            ));
        }
        if session_threads.iter().any(|thread| {
            thread.id != old_thread.id
                && thread.external_agent == old_thread.external_agent
                && thread.archived_at.is_none()
                && thread.replaced_by_id.is_none()
        }) {
            return Err(format!(
                "Session {} has another active {} agent thread.",
                old_thread.session_id, old_thread.external_agent
            ));
        }

        let now = timestamp();
        let mut archived_thread = old_thread;
        archived_thread.status = CadAgentThreadStatus::Replaced;
        archived_thread.updated_at = now.clone();
        archived_thread.archived_at = Some(now.clone());
        archived_thread.replaced_by_id = Some(replacement.id.clone());
        insert_replacement_metadata(
            &mut archived_thread.metadata,
            &reason,
            &now,
            "replacedByReason",
        );
        let mut replacement = replacement;
        insert_replacement_metadata(
            &mut replacement.metadata,
            &reason,
            &now,
            "replacementReason",
        );
        self.repository
            .replace_agent_thread(&archived_thread, &replacement)?;

        let threads = state
            .agent_threads
            .get_mut(&archived_thread.session_id)
            .ok_or_else(|| {
                format!(
                    "Agent thread state is missing for {}",
                    archived_thread.session_id
                )
            })?;
        let existing = threads
            .iter_mut()
            .find(|thread| thread.id == archived_thread.id)
            .ok_or_else(|| format!("Agent thread disappeared from memory: {old_thread_id}"))?;
        *existing = archived_thread.clone();
        threads.push(replacement.clone());
        threads.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(CadAgentThreadReplacementResult {
            archived_thread,
            active_thread: replacement,
        })
    }

    pub fn install_first_agent_thread(
        &self,
        thread: CadAgentThread,
    ) -> Result<CadAgentThread, String> {
        validate_agent_thread_fields(&thread)?;
        if thread.archived_at.is_some() || thread.replaced_by_id.is_some() {
            return Err(format!(
                "New agent thread must be active and unreplaced: {}",
                thread.id
            ));
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, &thread.session_id)?;
        reject_all_active_agent_runs(&state, &thread.session_id)?;
        let threads = state
            .agent_threads
            .entry(thread.session_id.clone())
            .or_default();
        if threads.iter().any(|candidate| candidate.id == thread.id) {
            return Err(format!("Agent thread id already exists: {}", thread.id));
        }
        if threads.iter().any(|candidate| {
            candidate.external_agent == thread.external_agent
                && candidate.archived_at.is_none()
                && candidate.replaced_by_id.is_none()
        }) {
            return Err(format!(
                "Session {} already has an active {} agent thread.",
                thread.session_id, thread.external_agent
            ));
        }
        self.repository.save_agent_thread(&thread)?;
        threads.push(thread.clone());
        threads.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(thread)
    }

    pub fn agent_session_diagnostics(
        &self,
        session_id: &str,
    ) -> Result<CadAgentSessionDiagnostics, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let session = require_session(&state, session_id)?;
        let runs = state
            .agent_runs
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        let mut threads = state
            .agent_threads
            .get(session_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|thread| {
                let mut thread_runs = runs
                    .iter()
                    .filter(|run| run.agent_thread_id.as_deref() == Some(thread.id.as_str()))
                    .map(run_diagnostic)
                    .collect::<Vec<_>>();
                thread_runs.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
                CadAgentThreadDiagnostic {
                    thread,
                    runs: thread_runs,
                }
            })
            .collect::<Vec<_>>();
        threads.sort_by(|left, right| {
            left.thread
                .created_at
                .cmp(&right.thread.created_at)
                .then_with(|| left.thread.id.cmp(&right.thread.id))
        });
        let mut unbound_runs = runs
            .iter()
            .filter(|run| run.agent_thread_id.is_none())
            .map(run_diagnostic)
            .collect::<Vec<_>>();
        unbound_runs.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
        Ok(CadAgentSessionDiagnostics {
            session_id: session_id.to_string(),
            archived: session.archived_at.is_some(),
            threads,
            unbound_runs,
            transport_event_count: state
                .agent_transport_events
                .get(session_id)
                .map_or(0, Vec::len),
        })
    }

    pub fn cleanup_agent_transport_events(
        &self,
        input: CadAgentTransportCleanupInput,
    ) -> Result<CadAgentTransportCleanupResult, String> {
        if input.created_before.is_none() && input.max_events_per_session.is_none() {
            return Err(
                "Transport cleanup requires created_before or max_events_per_session.".to_string(),
            );
        }
        if input
            .created_before
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err("Transport cleanup created_before cannot be empty.".to_string());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        if let Some(session_id) = &input.session_id {
            require_session(&state, session_id)?;
        }
        let mut deleted_ids = HashSet::new();
        for (session_id, events) in &state.agent_transport_events {
            if input
                .session_id
                .as_deref()
                .is_some_and(|selected| selected != session_id)
            {
                continue;
            }
            if let Some(created_before) = &input.created_before {
                deleted_ids.extend(
                    events
                        .iter()
                        .filter(|event| event.created_at < *created_before)
                        .map(|event| event.id.clone()),
                );
            }
            if let Some(cap) = input.max_events_per_session {
                let mut newest = events.iter().collect::<Vec<_>>();
                newest.sort_by(|left, right| {
                    right
                        .created_at
                        .cmp(&left.created_at)
                        .then_with(|| right.sequence.cmp(&left.sequence))
                        .then_with(|| right.id.cmp(&left.id))
                });
                deleted_ids.extend(newest.into_iter().skip(cap).map(|event| event.id.clone()));
            }
        }
        let mut deleted_event_ids = deleted_ids.into_iter().collect::<Vec<_>>();
        deleted_event_ids.sort();
        let deleted_count = self
            .repository
            .delete_agent_transport_events(&deleted_event_ids)?;
        if deleted_count != deleted_event_ids.len() {
            return Err(format!(
                "Transport cleanup expected to delete {} events, deleted {deleted_count}.",
                deleted_event_ids.len()
            ));
        }
        for events in state.agent_transport_events.values_mut() {
            events.retain(|event| deleted_event_ids.binary_search(&event.id).is_err());
        }
        Ok(CadAgentTransportCleanupResult {
            deleted_count,
            deleted_event_ids,
        })
    }
}

fn reject_all_active_agent_runs(state: &ServiceState, session_id: &str) -> Result<(), String> {
    if let Some(run) = state
        .agent_runs
        .get(session_id)
        .into_iter()
        .flatten()
        .find(|run| !is_terminal_run_status(&run.status))
    {
        return Err(format!(
            "Agent thread cannot be replaced while run {} is active.",
            run.id
        ));
    }
    Ok(())
}

fn validate_replacement_active_runs(
    state: &ServiceState,
    old_thread: &CadAgentThread,
    allowed_run_id: Option<&str>,
) -> Result<(), String> {
    let active_runs = state
        .agent_runs
        .get(&old_thread.session_id)
        .into_iter()
        .flatten()
        .filter(|run| !is_terminal_run_status(&run.status))
        .collect::<Vec<_>>();
    for run in active_runs {
        if allowed_run_id != Some(run.id.as_str()) {
            return Err(format!(
                "Agent thread cannot be replaced while run {} is active.",
                run.id
            ));
        }
        if run.session_id != old_thread.session_id
            || run
                .external_agent
                .as_deref()
                .is_some_and(|agent| agent != old_thread.external_agent)
            || run.agent_thread_id.is_some()
            || run.external_turn_id.is_some()
        {
            return Err(format!(
                "Allowed replacement run {} must be unbound and belong to the replacement session/agent.",
                run.id
            ));
        }
    }
    if let Some(allowed_run_id) = allowed_run_id {
        let allowed_exists = state
            .agent_runs
            .get(&old_thread.session_id)
            .into_iter()
            .flatten()
            .any(|run| run.id == allowed_run_id && !is_terminal_run_status(&run.status));
        if !allowed_exists {
            return Err(format!(
                "Allowed replacement run is not active in session {}: {allowed_run_id}",
                old_thread.session_id
            ));
        }
    }
    Ok(())
}

fn insert_replacement_metadata(
    metadata: &mut Option<Metadata>,
    reason: &str,
    replaced_at: &str,
    reason_key: &str,
) {
    let metadata = metadata.get_or_insert_with(Map::new);
    metadata.insert(reason_key.to_string(), Value::String(reason.to_string()));
    metadata.insert(
        "replacementRecordedAt".to_string(),
        Value::String(replaced_at.to_string()),
    );
}

fn run_diagnostic(run: &CadAgentRun) -> CadAgentRunDiagnostic {
    CadAgentRunDiagnostic {
        run_id: run.id.clone(),
        status: run.status.clone(),
        recovery_status: run.recovery_status.clone(),
        agent_thread_id: run.agent_thread_id.clone(),
        external_thread_id: run.external_thread_id.clone(),
        external_turn_id: run.external_turn_id.clone(),
        connection_generation: run.connection_generation,
        last_error: run.error.clone(),
        updated_at: run.updated_at.clone(),
    }
}
