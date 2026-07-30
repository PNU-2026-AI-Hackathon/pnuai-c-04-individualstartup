use super::*;

pub(super) fn require_session<'a>(
    state: &'a ServiceState,
    session_id: &str,
) -> Result<&'a CadSession, String> {
    state
        .sessions
        .get(session_id)
        .ok_or_else(|| format!("CAD session is missing or has been deleted: {session_id}"))
}

pub(super) fn require_session_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
) -> Result<&'a mut CadSession, String> {
    state
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("CAD session is missing or has been deleted: {session_id}"))
}

pub(super) fn require_revision<'a>(
    state: &'a ServiceState,
    revision_id: &str,
) -> Result<&'a CadRevision, String> {
    state
        .revisions
        .get(revision_id)
        .ok_or_else(|| format!("CAD revision not found: {revision_id}"))
}

pub(super) fn require_revision_mut<'a>(
    state: &'a mut ServiceState,
    revision_id: &str,
) -> Result<&'a mut CadRevision, String> {
    state
        .revisions
        .get_mut(revision_id)
        .ok_or_else(|| format!("CAD revision not found: {revision_id}"))
}

pub(super) fn require_agent_run_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
    run_id: &str,
) -> Result<&'a mut CadAgentRun, String> {
    state
        .agent_runs
        .get_mut(session_id)
        .and_then(|runs| runs.iter_mut().find(|run| run.id == run_id))
        .ok_or_else(|| format!("Agent run not found: {run_id}"))
}

pub(super) fn validate_workflow_run(
    state: &ServiceState,
    session_id: &str,
    run_id: &str,
) -> Result<(), String> {
    require_session(state, session_id)?;
    state
        .agent_runs
        .get(session_id)
        .into_iter()
        .flatten()
        .find(|run| run.id == run_id)
        .map(|_| ())
        .ok_or_else(|| format!("Agent run not found: {run_id}"))
}

pub(super) fn validate_revision_session(
    state: &ServiceState,
    session_id: &str,
    revision_id: &str,
) -> Result<(), String> {
    let revision = require_revision(state, revision_id)?;
    if revision.session_id != session_id {
        return Err(format!(
            "CAD revision {revision_id} does not belong to session {session_id}."
        ));
    }
    Ok(())
}

pub(super) fn validate_artifact_session(
    state: &ServiceState,
    session_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    let artifact = state
        .artifacts
        .get(artifact_id)
        .ok_or_else(|| format!("CAD artifact not found: {artifact_id}"))?;
    validate_revision_session(state, session_id, &artifact.revision_id)
}
