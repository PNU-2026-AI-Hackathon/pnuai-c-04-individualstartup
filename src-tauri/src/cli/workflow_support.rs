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
                "Run {run_id} has no committed CadModelPlan. Call cadastrophe-plan-commit first."
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

pub(super) fn record_structural_started(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    revision_id: &str,
) -> CliResult<()> {
    service
        .record_agent_tool_event(
            session_id,
            run_id,
            Some(revision_id.to_string()),
            CadAgentRunEventType::AgentToolStarted,
            json!({
                "command": "cadastrophe-evaluate-structural",
                "status": "started"
            }),
        )
        .map(|_| ())
        .map_err(CliError::storage)
}

pub(super) fn record_structural_completed(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    evaluation: Option<&StructuralEvaluation>,
    error: Option<&CliError>,
) -> CliResult<()> {
    let payload = if let Some(evaluation) = evaluation {
        json!({
            "command": "cadastrophe-evaluate-structural",
            "status": "completed",
            "ok": true,
            "passed": evaluation.passed,
            "artifactId": evaluation.artifact.as_ref().map(|artifact| artifact.id.clone()),
            "nextAction": if evaluation.passed { "vlm_judge" } else { "outer_loop_refine_source" }
        })
    } else {
        let error = error.expect("error provided when evaluation missing");
        json!({
            "command": "cadastrophe-evaluate-structural",
            "status": "failed",
            "ok": false,
            "error": {
                "code": error.code,
                "message": error.message
            }
        })
    };
    service
        .record_agent_tool_event(
            session_id,
            run_id,
            Some(revision_id.to_string()),
            CadAgentRunEventType::AgentToolCompleted,
            payload,
        )
        .map(|_| ())
        .map_err(CliError::storage)
}

pub(super) fn pending_vlm_for_run(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    artifact_id: &str,
) -> CliResult<CadWorkflowPendingVlm> {
    let pending = service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?
        .workflow
        .pending_vlm
        .into_iter()
        .find(|pending| pending.run_id == run_id)
        .ok_or_else(|| {
            CliError::precondition_failed(format!(
                "Run {run_id} has no pending cadastrophe.vlm_judge.v1 contract."
            ))
        })?;
    if pending.artifact_id != artifact_id {
        return Err(CliError::precondition_failed(format!(
            "Pending VLM artifact {} does not match submitted artifact {artifact_id}.",
            pending.artifact_id
        )));
    }
    Ok(pending)
}

pub(super) fn append_outer_iteration(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    revision_id: Option<String>,
    structural_report: Value,
    vlm_report: Option<Value>,
    failure_report: Option<Value>,
    passed: bool,
) -> CliResult<crate::protocol::CadWorkflowState> {
    let state = service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?;
    let iteration = state
        .workflow
        .outer_iterations
        .iter()
        .filter(|iteration| iteration.run_id == run_id)
        .map(|iteration| iteration.iteration)
        .max()
        .unwrap_or(0)
        + 1;
    service
        .save_workflow_outer_iteration(
            session_id,
            CadWorkflowOuterIteration {
                id: format!("workflow-outer-{run_id}-{iteration}"),
                run_id: run_id.to_string(),
                iteration,
                revision_id,
                structural_report,
                vlm_report,
                failure_report,
                passed,
                created_at: timestamp(),
            },
        )
        .map_err(CliError::storage)
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
            "Run {run_id} has no committed CadModelPlan. Call cadastrophe-plan-commit first."
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
