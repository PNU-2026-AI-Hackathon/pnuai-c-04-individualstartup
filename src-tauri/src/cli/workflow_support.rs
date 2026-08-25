use super::*;

pub(super) fn committed_plan_for_run(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
) -> CliResult<CadWorkflowPlan> {
    require_committed_plan(service, session_id, run_id)?;
    service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?
        .workflow
        .plans
        .into_iter()
        .find(|plan| plan.run_id == run_id)
        .ok_or_else(|| {
            CliError::precondition_failed(format!(
                "Run {run_id} has no committed CadModelPlan. Call cadgen-ax-plan-commit first."
            ))
        })
}

pub(super) fn require_exported_artifact(export: CadExportResult) -> CliResult<CadArtifact> {
    if !export.diagnostics.ok {
        return Err(CliError::precondition_failed(
            "Final STL export diagnostics failed.",
        ));
    }
    export
        .artifact
        .ok_or_else(|| CliError::precondition_failed("Final STL export produced no artifact."))
}

pub(super) fn with_tool_events(
    service: &SessionService,
    command: &'static str,
    session_id: &str,
    run_id: Option<&str>,
    start_revision_id: Option<String>,
    action: impl FnOnce() -> CliResult<CommandOutput>,
) -> CliResult<CommandOutput> {
    if let Some(run_id) = run_id {
        service
            .record_agent_tool_event(
                session_id,
                run_id,
                start_revision_id.clone(),
                CadAgentRunEventType::AgentToolStarted,
                json!({
                    "command": command,
                    "status": "started"
                }),
            )
            .map_err(CliError::storage)?;
        match action() {
            Ok(output) => {
                let completed_payload = merge_event_payload(
                    json!({
                        "command": command,
                        "status": "completed",
                        "ok": true
                    }),
                    output.event_payload.clone(),
                );
                service
                    .record_agent_tool_event(
                        session_id,
                        run_id,
                        output.revision_id.clone().or(start_revision_id),
                        CadAgentRunEventType::AgentToolCompleted,
                        completed_payload,
                    )
                    .map_err(CliError::storage)?;
                Ok(output)
            }
            Err(error) => {
                let _ = service.record_agent_tool_event(
                    session_id,
                    run_id,
                    start_revision_id,
                    CadAgentRunEventType::AgentToolCompleted,
                    json!({
                        "command": command,
                        "status": "failed",
                        "ok": false,
                        "error": {
                            "code": error.code,
                            "message": error.message
                        }
                    }),
                );
                Err(error)
            }
        }
    } else {
        action()
    }
}

pub(super) fn require_committed_plan(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
) -> CliResult<()> {
    let state = service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?;
    if !state.agent_runs.iter().any(|run| run.id == run_id) {
        return Err(CliError::not_found(format!(
            "Agent run not found: {run_id}"
        )));
    }
    if !state
        .workflow
        .plans
        .iter()
        .any(|plan| plan.run_id == run_id)
    {
        return Err(CliError::precondition_failed(format!(
            "Run {run_id} has no committed CadModelPlan. Call cadgen-ax-plan-commit first."
        )));
    }
    Ok(())
}

pub(super) fn require_revision_in_session(
    service: &SessionService,
    session_id: &str,
    revision_id: &str,
) -> CliResult<()> {
    let state = service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?;
    if state
        .session
        .revisions
        .iter()
        .any(|revision| revision.id == revision_id)
    {
        return Ok(());
    }
    Err(CliError::not_found(format!(
        "CAD revision {revision_id} does not belong to session {session_id}."
    )))
}
