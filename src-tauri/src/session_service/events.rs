use super::*;

pub(super) fn add_user_event(
    revision: &mut CadRevision,
    event_type: &str,
    payload: Value,
) -> CadUserEvent {
    let event = CadUserEvent {
        id: uuid(),
        revision_id: revision.id.clone(),
        event_type: event_type.to_string(),
        created_at: timestamp(),
        payload: metadata_from_value(payload),
    };
    revision.user_events.push(event.clone());
    event
}

pub(super) fn append_agent_run_event(
    state: &mut ServiceState,
    session_id: &str,
    run_id: &str,
    revision_id: Option<String>,
    event_type: CadAgentRunEventType,
    payload: Value,
    metadata: Option<Metadata>,
) -> CadAgentRunEvent {
    let events = state
        .agent_run_events
        .entry(session_id.to_string())
        .or_default();
    let sequence = events
        .iter()
        .filter(|event| event.run_id == run_id)
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0)
        + 1;
    let event = CadAgentRunEvent {
        id: uuid(),
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        revision_id,
        event_type,
        sequence,
        created_at: timestamp(),
        payload: metadata_from_value(payload),
        metadata,
    };
    events.push(event.clone());
    event
}

pub(super) fn persist_agent_run_event(
    repository: &dyn SessionRepository,
    state: &mut ServiceState,
    session_id: &str,
    event: CadAgentRunEvent,
) -> Result<CadAgentRunEvent, String> {
    let saved = repository.save_agent_run_event(&event)?;
    if saved.sequence != event.sequence {
        if let Some(events) = state.agent_run_events.get_mut(session_id) {
            if let Some(existing) = events.iter_mut().find(|candidate| candidate.id == saved.id) {
                *existing = saved.clone();
            }
            events.sort_by(|left, right| {
                left.run_id
                    .cmp(&right.run_id)
                    .then_with(|| left.sequence.cmp(&right.sequence))
            });
        }
    }
    Ok(saved)
}

pub(super) fn event_type_for_run_status(status: &CadAgentRunStatus) -> CadBridgeEventType {
    match status {
        CadAgentRunStatus::Completed => CadBridgeEventType::AgentRunCompleted,
        CadAgentRunStatus::Failed => CadBridgeEventType::AgentRunFailed,
        CadAgentRunStatus::Queued => CadBridgeEventType::AgentRunCreated,
        _ => CadBridgeEventType::AgentRunUpdated,
    }
}

pub(super) fn run_event_type_for_update(
    bridge_event_type: Option<&CadBridgeEventType>,
    status: &CadAgentRunStatus,
) -> CadAgentRunEventType {
    match bridge_event_type {
        Some(CadBridgeEventType::AgentMessageCreated) => CadAgentRunEventType::AgentMessageCreated,
        Some(CadBridgeEventType::AgentToolStarted) => CadAgentRunEventType::AgentToolStarted,
        Some(CadBridgeEventType::AgentToolCompleted) => CadAgentRunEventType::AgentToolCompleted,
        Some(CadBridgeEventType::AgentRunCompleted) => CadAgentRunEventType::AgentRunCompleted,
        Some(CadBridgeEventType::AgentRunFailed) => CadAgentRunEventType::AgentRunFailed,
        _ => match status {
            CadAgentRunStatus::Completed => CadAgentRunEventType::AgentRunCompleted,
            CadAgentRunStatus::Failed => CadAgentRunEventType::AgentRunFailed,
            CadAgentRunStatus::Cancelled => CadAgentRunEventType::AgentRunCancelled,
            CadAgentRunStatus::Queued => CadAgentRunEventType::AgentRunCreated,
            _ => CadAgentRunEventType::AgentRunUpdated,
        },
    }
}

pub(super) fn is_terminal_run_status(status: &CadAgentRunStatus) -> bool {
    matches!(
        status,
        CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
    )
}
