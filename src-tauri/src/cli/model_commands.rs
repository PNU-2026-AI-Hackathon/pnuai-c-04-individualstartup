use super::workflow_support::{
    require_committed_plan, require_revision_in_session, with_tool_events,
};
use super::*;

pub(super) fn plan_commit(
    args: &ParsedArgs,
    service: &SessionService,
    _app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = resolve_session_id(args, service)?;
    let run_id = resolve_active_run_id(args, service, &session_id)?;
    ensure_run_belongs_to_session(service, &session_id, &run_id)?;
    let plan_path = args.required_path("plan")?;
    let active_revision_id = service
        .get_session_state(&session_id)
        .map_err(CliError::not_found)?
        .session
        .active_revision_id;
    let active_revision_id = match args.optional("revision") {
        Some(_) => Some(resolve_active_revision_id(args, service, &session_id)?),
        None => active_revision_id,
    };
    if let Some(revision_id) = &active_revision_id {
        require_revision_in_session(service, &session_id, revision_id)?;
    }
    validate_run_revision_scope(service, &session_id, &run_id, active_revision_id.as_deref())?;

    with_tool_events(
        service,
        "cadgen-ax-plan-commit",
        &session_id,
        Some(&run_id),
        active_revision_id.clone(),
        || {
            let plan_json = fs::read_to_string(&plan_path).map_err(|error| {
                CliError::invalid_input(format!(
                    "Failed to read plan file {}: {error}",
                    plan_path.display()
                ))
            })?;
            let plan = parse_plan_draft_json(&plan_json, &plan_path.display().to_string())?;
            let workflow_plan = CadWorkflowPlan {
                run_id: run_id.clone(),
                revision_id: active_revision_id.clone(),
                source_language: plan.source_language.clone(),
                plan,
                created_at: timestamp(),
            };
            let workflow = service
                .save_workflow_plan(&session_id, workflow_plan.clone())
                .map_err(CliError::storage)?;
            Ok(CommandOutput {
                revision_id: active_revision_id.clone(),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": active_revision_id,
                    "schemaVersion": workflow_plan.plan.schema_version,
                    "sourceLanguage": workflow_plan.source_language,
                    "nextAction": "source_apply"
                }),
                data: json!({
                    "runId": run_id,
                    "revisionId": workflow_plan.revision_id,
                    "plan": workflow_plan.plan,
                    "workflow": workflow,
                    "nextAction": "source_apply"
                }),
            })
        },
    )
}

pub(super) fn source_apply(
    args: &ParsedArgs,
    service: &SessionService,
    _app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = resolve_session_id(args, service)?;
    let run_id = resolve_active_run_id(args, service, &session_id)?;
    ensure_run_belongs_to_session(service, &session_id, &run_id)?;
    let source_path = args.required_path("source")?;
    let language = resolve_source_language(args, service, &session_id)?;
    if language != CadSourceLanguage::Openscad {
        return Err(CliError::invalid_input(
            "cadgen-ax-source-apply currently supports openscad source only.",
        ));
    }
    let parent_revision_id = service
        .get_session_state(&session_id)
        .map_err(CliError::not_found)?
        .session
        .active_revision_id;
    let parent_revision_id = match args.optional("revision") {
        Some(_) => Some(resolve_active_revision_id(args, service, &session_id)?),
        None => parent_revision_id,
    };
    if let Some(revision_id) = &parent_revision_id {
        require_revision_in_session(service, &session_id, revision_id)?;
    }
    validate_run_revision_scope(service, &session_id, &run_id, parent_revision_id.as_deref())?;

    with_tool_events(
        service,
        "cadgen-ax-source-apply",
        &session_id,
        Some(&run_id),
        parent_revision_id.clone(),
        || {
            require_committed_plan(service, &session_id, &run_id)?;
            let source = fs::read_to_string(&source_path).map_err(|error| {
                CliError::invalid_input(format!(
                    "Failed to read source file {}: {error}",
                    source_path.display()
                ))
            })?;
            let result = service
                .update_model_source(UpdateModelSourceInput {
                    session_id: session_id.clone(),
                    source_language: language.clone(),
                    source,
                    parent_revision_id: parent_revision_id.clone(),
                    parameters: None,
                })
                .map_err(CliError::storage)?;
            service
                .link_agent_run_output_revision(&session_id, &run_id, result.revision_id.clone())
                .map_err(CliError::storage)?;
            let (preview_result, rendered_state) = service
                .render_preview(RenderPreviewInput {
                    session_id: session_id.clone(),
                    revision_id: Some(result.revision_id.clone()),
                })
                .map_err(CliError::runtime)?;
            let artifact_paths = artifact_paths(preview_result.artifacts.iter());
            let diagnostics_ok = preview_result.diagnostics.ok;
            let next_action = if diagnostics_ok {
                "finalize"
            } else {
                "source_repair"
            };
            Ok(CommandOutput {
                revision_id: Some(result.revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": result.revision_id,
                    "parentRevisionId": parent_revision_id,
                    "sourceLanguage": language,
                    "diagnosticsOk": diagnostics_ok,
                    "diagnostics": preview_result.diagnostics,
                    "previewArtifactId": preview_result.artifacts.iter().find(|artifact| artifact.kind == CadArtifactKind::PreviewMesh).map(|artifact| artifact.id.clone()),
                    "stlArtifactId": preview_result.artifacts.iter().find(|artifact| artifact.kind == CadArtifactKind::Stl).map(|artifact| artifact.id.clone()),
                    "nextAction": next_action
                }),
                data: json!({
                    "runId": run_id,
                    "revisionId": result.revision_id,
                    "parentRevisionId": parent_revision_id,
                    "sourceLanguage": language,
                    "diagnostics": preview_result.diagnostics,
                    "artifacts": preview_result.artifacts,
                    "artifactPaths": artifact_paths,
                    "state": rendered_state,
                    "nextAction": next_action
                }),
            })
        },
    )
}

fn validate_run_revision_scope(
    service: &SessionService,
    session_id: &str,
    run_id: &str,
    revision_id: Option<&str>,
) -> CliResult<()> {
    let run = service
        .get_agent_run(session_id, run_id)
        .map_err(CliError::storage)?
        .ok_or_else(|| CliError::not_found(format!("Agent run not found: {run_id}")))?;
    if let Some(revision_id) = revision_id {
        let belongs_to_run = run.input_revision_id.as_deref() == Some(revision_id)
            || run.output_revision_id.as_deref() == Some(revision_id);
        if !belongs_to_run {
            return Err(CliError::precondition_failed(format!(
                "CAD revision {revision_id} is not an input or output revision of agent run {run_id}."
            )));
        }
    }
    Ok(())
}
