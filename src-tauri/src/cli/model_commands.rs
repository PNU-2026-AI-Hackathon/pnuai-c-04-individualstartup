use super::workflow_support::{
    require_committed_plan, require_revision_in_session, with_tool_events,
};
use super::*;

pub(super) fn plan_commit(
    args: &ParsedArgs,
    service: &SessionService,
    _app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let run_id = args.required("run")?.to_string();
    let plan_path = args.required_path("plan")?;
    let active_revision_id = service
        .get_session_state(&session_id)
        .map_err(CliError::not_found)?
        .session
        .active_revision_id;

    with_tool_events(
        service,
        "cadastrophe-plan-commit",
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
            let plan: CadModelPlan = serde_json::from_str(&plan_json).map_err(|error| {
                CliError::invalid_input(format!(
                    "Plan file {} is not a valid CadModelPlan JSON document: {error}",
                    plan_path.display()
                ))
            })?;
            validate_plan(&plan)?;
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
    let session_id = args.required("session")?.to_string();
    let run_id = args.required("run")?.to_string();
    let source_path = args.required_path("source")?;
    let language = parse_source_language(args.required("language")?)?;
    if language != CadSourceLanguage::Openscad {
        return Err(CliError::invalid_input(
            "cadastrophe-source-apply currently supports --language openscad only.",
        ));
    }
    let parent_revision_id = service
        .get_session_state(&session_id)
        .map_err(CliError::not_found)?
        .session
        .active_revision_id;

    with_tool_events(
        service,
        "cadastrophe-source-apply",
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
            let state = service
                .link_agent_run_output_revision(&session_id, &run_id, result.revision_id.clone())
                .map_err(CliError::storage)?;
            let diagnostics = state
                .active_revision
                .as_ref()
                .map(|revision| revision.diagnostics.clone());
            Ok(CommandOutput {
                revision_id: Some(result.revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": result.revision_id,
                    "parentRevisionId": parent_revision_id,
                    "sourceLanguage": language,
                    "diagnosticsOk": diagnostics.as_ref().is_some_and(|diagnostics| diagnostics.ok),
                    "nextAction": "preview_render"
                }),
                data: json!({
                    "runId": run_id,
                    "revisionId": result.revision_id,
                    "parentRevisionId": parent_revision_id,
                    "sourceLanguage": language,
                    "diagnostics": diagnostics,
                    "state": state,
                    "nextAction": "preview_render"
                }),
            })
        },
    )
}

pub(super) fn preview_render(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let revision_id = args.required("revision")?.to_string();
    require_revision_in_session(service, &session_id, &revision_id)?;
    let run_id = args.optional("run").map(str::to_string);
    with_tool_events(
        service,
        "cadastrophe-preview-render",
        &session_id,
        run_id.as_deref(),
        Some(revision_id.clone()),
        || {
            let revision = service
                .get_revision(&session_id, &revision_id)
                .map_err(CliError::storage)?;
            let rendered = render_open_scad_wasm_cli(&revision.source, app_data_dir)?;
            let diagnostics = rendered.diagnostics.clone();
            let mut preview_artifact = None;
            let mut stl_artifact = None;
            let state = if diagnostics.ok {
                let mesh = rendered.mesh.clone().ok_or_else(|| {
                    CliError::runtime("OpenSCAD WASM render did not return preview mesh.")
                })?;
                let stl_base64 = rendered.stl_base64.clone().ok_or_else(|| {
                    CliError::runtime("OpenSCAD WASM render did not return STL bytes.")
                })?;
                let source_hash = storage::sha256_hex(revision.source.as_bytes());
                let parameter_hash = storage::sha256_hex(
                    serde_json::to_string(&revision.parameters)
                        .map_err(|error| CliError::runtime(error.to_string()))?
                        .as_bytes(),
                );
                let metadata = json!({
                    "runtime": "openscad-wasm",
                    "sourceLanguage": "openscad",
                    "sourceHash": source_hash,
                    "parameterHash": parameter_hash,
                    "stlSha256": rendered.stl_sha256,
                    "stlBytes": rendered.stl_bytes,
                    "renderDurationMs": diagnostics.elapsed_ms,
                    "diagnosticsSource": "openscad-wasm",
                    "phase": "cli-preview"
                });
                let preview = service
                    .persist_runtime_artifact(PersistRuntimeArtifactInput {
                        session_id: session_id.clone(),
                        revision_id: revision_id.clone(),
                        kind: CadArtifactKind::PreviewMesh,
                        format: "json".to_string(),
                        contents_base64: base64_encode(
                            serde_json::to_string(&mesh)
                                .map_err(|error| CliError::runtime(error.to_string()))?
                                .as_bytes(),
                        ),
                        diagnostics: diagnostics.clone(),
                        metadata: metadata.as_object().cloned().ok_or_else(|| {
                            CliError::runtime("Runtime metadata is not an object.")
                        })?,
                    })
                    .map_err(CliError::storage)?;
                let stl = service
                    .persist_runtime_artifact(PersistRuntimeArtifactInput {
                        session_id: session_id.clone(),
                        revision_id: revision_id.clone(),
                        kind: CadArtifactKind::Stl,
                        format: "stl".to_string(),
                        contents_base64: stl_base64,
                        diagnostics: diagnostics.clone(),
                        metadata: metadata.as_object().cloned().ok_or_else(|| {
                            CliError::runtime("Runtime metadata is not an object.")
                        })?,
                    })
                    .map_err(CliError::storage)?;
                preview_artifact = Some(preview.artifact);
                stl_artifact = Some(stl.artifact);
                stl.state
            } else {
                service
                    .record_runtime_diagnostics(&session_id, &revision_id, diagnostics.clone())
                    .map_err(CliError::storage)?
            };
            let artifacts = preview_artifact
                .iter()
                .chain(stl_artifact.iter())
                .cloned()
                .collect::<Vec<_>>();
            let artifact_paths = artifact_paths(artifacts.iter());
            Ok(CommandOutput {
                revision_id: Some(revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "diagnosticsOk": diagnostics.ok,
                    "diagnostics": diagnostics,
                    "previewArtifactId": preview_artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "stlArtifactId": stl_artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "nextAction": if diagnostics.ok { "artifact_export" } else { "source_repair" }
                }),
                data: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "diagnostics": diagnostics,
                    "previewArtifactId": preview_artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "stlArtifactId": stl_artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "artifacts": artifacts,
                    "artifactPaths": artifact_paths,
                    "state": state,
                    "nextAction": if diagnostics.ok { "artifact_export" } else { "source_repair" }
                }),
            })
        },
    )
}

pub(super) fn artifact_export(
    args: &ParsedArgs,
    service: &SessionService,
    _app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let revision_id = args.required("revision")?.to_string();
    require_revision_in_session(service, &session_id, &revision_id)?;
    let format = args.required("format")?.to_string();
    if format != "stl" && format != "metadata" {
        return Err(CliError::invalid_input(
            "cadastrophe-artifact-export supports --format stl or --format metadata.",
        ));
    }
    let run_id = args.optional("run").map(str::to_string);
    with_tool_events(
        service,
        "cadastrophe-artifact-export",
        &session_id,
        run_id.as_deref(),
        Some(revision_id.clone()),
        || {
            let (result, state) = service
                .export_artifact(ExportArtifactInput {
                    session_id: session_id.clone(),
                    revision_id: Some(revision_id.clone()),
                    format: format.clone(),
                })
                .map_err(CliError::storage)?;
            let artifact_paths = artifact_paths(result.artifact.as_ref().into_iter());
            Ok(CommandOutput {
                revision_id: Some(revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "diagnosticsOk": result.diagnostics.ok,
                    "artifactId": result.artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "format": format,
                    "nextAction": if result.diagnostics.ok { "finalize" } else { "source_repair" }
                }),
                data: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "format": format,
                    "diagnostics": result.diagnostics,
                    "artifact": result.artifact,
                    "artifactPaths": artifact_paths,
                    "state": state,
                    "nextAction": if result.diagnostics.ok { "finalize" } else { "source_repair" }
                }),
            })
        },
    )
}
