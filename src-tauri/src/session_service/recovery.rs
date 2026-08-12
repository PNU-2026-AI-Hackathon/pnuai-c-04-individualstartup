use super::*;

impl SessionService {
    pub fn list_startup_agent_run_recovery_candidates(
        &self,
    ) -> Result<Vec<CadAgentRunRecoveryCandidate>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let mut candidates = state
            .agent_runs
            .values()
            .flatten()
            .filter(|run| {
                matches!(
                    run.status,
                    CadAgentRunStatus::Queued
                        | CadAgentRunStatus::Running
                        | CadAgentRunStatus::WaitingForUser
                )
            })
            .map(|run| {
                let action = if run.external_turn_id.is_some() {
                    CadAgentRunRecoveryAction::QueryHistory
                } else if run.status == CadAgentRunStatus::Queued {
                    CadAgentRunRecoveryAction::Reenqueue
                } else {
                    CadAgentRunRecoveryAction::MarkUnknownOutcome
                };
                CadAgentRunRecoveryCandidate {
                    session_id: run.session_id.clone(),
                    run_id: run.id.clone(),
                    status: run.status.clone(),
                    recovery_status: run.recovery_status.clone(),
                    action,
                    agent_thread_id: run.agent_thread_id.clone(),
                    external_thread_id: run.external_thread_id.clone(),
                    external_turn_id: run.external_turn_id.clone(),
                }
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then_with(|| left.run_id.cmp(&right.run_id))
        });
        Ok(candidates)
    }

    pub fn mark_agent_run_reconciling(
        &self,
        session_id: &str,
        run_id: &str,
        reason: String,
    ) -> Result<CadAgentRun, String> {
        if reason.trim().is_empty() {
            return Err("Agent run reconciliation reason cannot be empty.".to_string());
        }
        let (run, snapshot) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            if is_terminal_run_status(&run.status) {
                return Err(format!("Terminal agent run cannot be reconciled: {run_id}"));
            }
            if run.external_turn_id.is_none() {
                return Err(format!(
                    "Agent run without an external turn cannot query history: {run_id}"
                ));
            }
            if run.recovery_status == CadAgentRecoveryStatus::Reconciling {
                return Ok(run.clone());
            }
            let previous_status = run.status.clone();
            run.recovery_status = CadAgentRecoveryStatus::Reconciling;
            run.error = Some(reason.clone());
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
                    "previousStatus": previous_status,
                    "status": run.status,
                    "recoveryStatus": run.recovery_status,
                    "reason": reason
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let snapshot = build_state(&state, session_id)?;
            (run, snapshot)
        };
        self.emit(CadBridgeEventType::AgentRunUpdated, session_id, snapshot);
        Ok(run)
    }

    pub fn mark_agent_run_unknown_outcome(
        &self,
        session_id: &str,
        run_id: &str,
        reason: String,
    ) -> Result<CadAgentRun, String> {
        if reason.trim().is_empty() {
            return Err("Unknown agent run outcome requires a reason.".to_string());
        }
        let (run, snapshot) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            if run.recovery_status == CadAgentRecoveryStatus::UnknownOutcome
                && is_terminal_run_status(&run.status)
            {
                return Ok(run.clone());
            }
            if is_terminal_run_status(&run.status) {
                return Err(format!(
                    "Terminal agent run cannot be changed to unknown outcome: {run_id}"
                ));
            }
            if run.external_turn_id.is_some()
                && run.recovery_status != CadAgentRecoveryStatus::Reconciling
            {
                return Err(format!(
                    "Agent run with an external turn must query history before unknown outcome: {run_id}"
                ));
            }
            let now = timestamp();
            run.status = CadAgentRunStatus::Failed;
            run.recovery_status = CadAgentRecoveryStatus::UnknownOutcome;
            run.error = Some(reason.clone());
            run.active_step = None;
            run.completed_at = Some(now.clone());
            run.updated_at = now;
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                run.input_revision_id.clone(),
                CadAgentRunEventType::AgentRunFailed,
                json!({
                    "status": run.status,
                    "recoveryStatus": run.recovery_status,
                    "reason": reason,
                    "automaticRetry": false
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let snapshot = build_state(&state, session_id)?;
            (run, snapshot)
        };
        self.emit(CadBridgeEventType::AgentRunFailed, session_id, snapshot);
        Ok(run)
    }

    pub fn apply_agent_run_history_recovery(
        &self,
        input: CadAgentRunHistoryRecoveryInput,
    ) -> Result<CadAgentRunRecoveryResult, String> {
        let expected_status = terminal_status_for_history(&input.outcome);
        let (run, terminal_already_applied) = {
            let state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, &input.session_id)?;
            let run = state
                .agent_runs
                .get(&input.session_id)
                .into_iter()
                .flatten()
                .find(|run| run.id == input.run_id)
                .cloned()
                .ok_or_else(|| format!("Agent run not found: {}", input.run_id))?;
            if is_terminal_run_status(&run.status) {
                if run.status != expected_status {
                    return Err(format!(
                        "History outcome conflicts with terminal agent run {}: {:?} versus {:?}.",
                        run.id, run.status, expected_status
                    ));
                }
            }
            if run.external_turn_id.is_none() {
                return Err(format!(
                    "History recovery requires an external turn id: {}",
                    run.id
                ));
            }
            if run.external_thread_id.is_none() {
                return Err(format!(
                    "History recovery requires an external thread id: {}",
                    run.id
                ));
            }
            let terminal_already_applied = is_terminal_run_status(&run.status);
            (run, terminal_already_applied)
        };

        let mut inserted_message_count = 0;
        let mut updated_message_count = 0;
        let mut suppressed_message_count = 0;
        if let CadAgentRunHistoryOutcome::Completed { messages } = &input.outcome {
            let mut item_ids = HashSet::new();
            for recovered in messages {
                if recovered.external_item_id.trim().is_empty() {
                    return Err("Recovered agent message requires external_item_id.".to_string());
                }
                if recovered.content.trim().is_empty() {
                    return Err(format!(
                        "Recovered agent message content cannot be empty: {}",
                        recovered.external_item_id
                    ));
                }
                if recovered.created_at.trim().is_empty() {
                    return Err(format!(
                        "Recovered agent message requires created_at: {}",
                        recovered.external_item_id
                    ));
                }
                if !recovered.is_final {
                    return Err(format!(
                        "Recovered agent message must be a completed item: {}",
                        recovered.external_item_id
                    ));
                }
                if !item_ids.insert(recovered.external_item_id.as_str()) {
                    return Err(format!(
                        "History contains duplicate agent message item id: {}",
                        recovered.external_item_id
                    ));
                }
            }
            for recovered in messages {
                let existing = {
                    let state = self.inner.lock().map_err(lock_error)?;
                    state
                        .conversation
                        .get(&input.session_id)
                        .into_iter()
                        .flatten()
                        .find(|message| {
                            message.external_thread_id == run.external_thread_id
                                && message.external_turn_id == run.external_turn_id
                                && message.external_item_id.as_deref()
                                    == Some(recovered.external_item_id.as_str())
                        })
                        .cloned()
                };
                if existing.as_ref().is_some_and(|message| {
                    message.content == recovered.content
                        && recovered
                            .phase
                            .as_ref()
                            .is_none_or(|phase| message.phase.as_ref() == Some(phase))
                        && recovered
                            .sequence
                            .is_none_or(|sequence| message.sequence == Some(sequence))
                        && message.is_final
                        && recovered
                            .metadata
                            .as_ref()
                            .is_none_or(|metadata| message.metadata.as_ref() == Some(metadata))
                }) {
                    suppressed_message_count += 1;
                    continue;
                }
                let message = CadConversationMessage {
                    id: existing
                        .as_ref()
                        .map(|message| message.id.clone())
                        .unwrap_or_else(uuid),
                    session_id: input.session_id.clone(),
                    revision_id: run
                        .output_revision_id
                        .clone()
                        .or(run.input_revision_id.clone()),
                    role: CadConversationRole::Assistant,
                    content: recovered.content.clone(),
                    created_at: recovered.created_at.clone(),
                    run_id: Some(run.id.clone()),
                    external_thread_id: run.external_thread_id.clone(),
                    external_turn_id: run.external_turn_id.clone(),
                    external_item_id: Some(recovered.external_item_id.clone()),
                    phase: recovered.phase.clone(),
                    sequence: recovered.sequence,
                    is_final: true,
                    metadata: recovered.metadata.clone(),
                };
                let _ = self.upsert_agent_conversation_message(message)?;
                if existing.is_some() {
                    updated_message_count += 1;
                } else {
                    inserted_message_count += 1;
                }
            }
        }

        if terminal_already_applied {
            return Ok(CadAgentRunRecoveryResult {
                run,
                inserted_message_count,
                updated_message_count,
                suppressed_message_count,
                terminal_event_created: false,
            });
        }

        let (run, snapshot, bridge_event_type) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let run = require_agent_run_mut(&mut state, &input.session_id, &input.run_id)?;
            if is_terminal_run_status(&run.status) {
                if run.status != expected_status {
                    return Err(format!(
                        "History outcome raced with conflicting terminal state for run {}.",
                        run.id
                    ));
                }
                return Ok(CadAgentRunRecoveryResult {
                    run: run.clone(),
                    inserted_message_count,
                    updated_message_count,
                    suppressed_message_count,
                    terminal_event_created: false,
                });
            }
            let now = timestamp();
            run.status = expected_status.clone();
            run.active_step = None;
            run.completed_at = Some(now.clone());
            run.updated_at = now;
            match &input.outcome {
                CadAgentRunHistoryOutcome::Completed { .. } => {
                    run.error = None;
                    run.recovery_status = CadAgentRecoveryStatus::RecoveredFromHistory;
                }
                CadAgentRunHistoryOutcome::Failed { error } => {
                    if error.trim().is_empty() {
                        return Err("Recovered failed turn requires an error.".to_string());
                    }
                    run.error = Some(error.clone());
                    run.recovery_status = CadAgentRecoveryStatus::RecoveredFromHistory;
                }
                CadAgentRunHistoryOutcome::Interrupted { reason } => {
                    if reason.trim().is_empty() {
                        return Err("Recovered interrupted turn requires a reason.".to_string());
                    }
                    run.error = Some(reason.clone());
                    run.recovery_status = CadAgentRecoveryStatus::RecoveredFromHistory;
                }
                CadAgentRunHistoryOutcome::NotFound => {
                    run.error = Some(format!(
                        "External turn {} was not found in thread history; outcome is unknown and the run was not retried.",
                        run.external_turn_id.as_deref().unwrap_or_default()
                    ));
                    run.recovery_status = CadAgentRecoveryStatus::UnknownOutcome;
                }
            }
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event_type = match run.status {
                CadAgentRunStatus::Completed => CadAgentRunEventType::AgentRunCompleted,
                CadAgentRunStatus::Failed => CadAgentRunEventType::AgentRunFailed,
                CadAgentRunStatus::Cancelled => CadAgentRunEventType::AgentRunCancelled,
                _ => unreachable!("history outcome always maps to a terminal run status"),
            };
            let event = append_agent_run_event(
                &mut state,
                &input.session_id,
                &input.run_id,
                run.output_revision_id
                    .clone()
                    .or(run.input_revision_id.clone()),
                event_type,
                json!({
                    "status": run.status,
                    "recoveryStatus": run.recovery_status,
                    "recoveredFromHistory": true,
                    "error": run.error
                }),
                None,
            );
            persist_agent_run_event(
                self.repository.as_ref(),
                &mut state,
                &input.session_id,
                event,
            )?;
            let bridge_event_type = event_type_for_run_status(&run.status);
            let snapshot = build_state(&state, &input.session_id)?;
            (run, snapshot, bridge_event_type)
        };
        self.emit(bridge_event_type, &input.session_id, snapshot);
        Ok(CadAgentRunRecoveryResult {
            run,
            inserted_message_count,
            updated_message_count,
            suppressed_message_count,
            terminal_event_created: true,
        })
    }
}

fn terminal_status_for_history(outcome: &CadAgentRunHistoryOutcome) -> CadAgentRunStatus {
    match outcome {
        CadAgentRunHistoryOutcome::Completed { .. } => CadAgentRunStatus::Completed,
        CadAgentRunHistoryOutcome::Failed { .. } | CadAgentRunHistoryOutcome::NotFound => {
            CadAgentRunStatus::Failed
        }
        CadAgentRunHistoryOutcome::Interrupted { .. } => CadAgentRunStatus::Cancelled,
    }
}
