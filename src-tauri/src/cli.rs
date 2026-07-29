use crate::protocol::{
    CadAgentRunEventType, CadAgentRunStatus, CadArtifact, CadArtifactKind, CadDiagnostics,
    CadExportResult, CadModelPlan, CadSourceLanguage, CadWorkflowOuterIteration,
    CadWorkflowPendingVlm, CadWorkflowPlan, ExportArtifactInput, PersistRuntimeArtifactInput,
    UpdateModelSourceInput,
};
use crate::session_repository::SqliteSessionRepository;
use crate::session_service::SessionService;
use crate::storage::{self, StorageLayout};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const APP_IDENTIFIER: &str = "dev.cadastrophe.desktop";
const PLAN_SCHEMA_VERSION: &str = "cad_model_plan.v1";

pub fn session_current_main() -> i32 {
    run("cadastrophe-session-current", session_current)
}

pub fn session_state_main() -> i32 {
    run("cadastrophe-session-state", session_state)
}

pub fn plan_commit_main() -> i32 {
    run("cadastrophe-plan-commit", plan_commit)
}

pub fn source_apply_main() -> i32 {
    run("cadastrophe-source-apply", source_apply)
}

pub fn preview_render_main() -> i32 {
    run("cadastrophe-preview-render", preview_render)
}

pub fn artifact_export_main() -> i32 {
    run("cadastrophe-artifact-export", artifact_export)
}

pub fn evaluate_structural_main() -> i32 {
    run("cadastrophe-evaluate-structural", evaluate_structural)
}

pub fn finalize_main() -> i32 {
    run("cadastrophe-finalize", finalize)
}

pub fn vlm_submit_main() -> i32 {
    run("cadastrophe-vlm-submit", vlm_submit)
}

fn run(
    command: &'static str,
    handler: fn(&ParsedArgs, &SessionService, &PathBuf) -> CliResult<CommandOutput>,
) -> i32 {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => return emit_error(command, false, error),
    };
    let pretty = parsed.pretty;
    let app_data_dir = match parsed.app_data_dir() {
        Ok(path) => path,
        Err(error) => return emit_error(command, pretty, error),
    };
    let service = match load_service(app_data_dir.clone()) {
        Ok(service) => service,
        Err(error) => return emit_error(command, pretty, error),
    };
    match handler(&parsed, &service, &app_data_dir) {
        Ok(output) => emit_success(command, pretty, output.data),
        Err(error) => emit_error(command, pretty, error),
    }
}

fn session_current(
    _args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let current = service.get_current_session().map_err(CliError::storage)?;
    let (active_revision_id, selected_runtime) = current
        .state
        .as_ref()
        .map(|state| {
            (
                state.session.active_revision_id.clone(),
                Some(state.session.selected_runtime.clone()),
            )
        })
        .unwrap_or((None, None));
    Ok(CommandOutput::new(json!({
        "appDataDir": app_data_dir,
        "sessionId": current.session_id,
        "uiUrl": current.ui_url,
        "activeRevisionId": active_revision_id,
        "selectedRuntime": selected_runtime,
        "state": current.state
    })))
}

fn session_state(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?;
    let state = service
        .get_session_state(session_id)
        .map_err(CliError::not_found)?;
    Ok(CommandOutput::new(json!({
        "appDataDir": app_data_dir,
        "sessionId": session_id,
        "state": state
    })))
}

fn plan_commit(
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

fn source_apply(
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

fn preview_render(
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

fn artifact_export(
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

fn evaluate_structural(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let revision_id = args.required("revision")?.to_string();
    let run_id = args.optional("run").map(str::to_string);
    let plan_path = args.required_path("plan")?;
    require_revision_in_session(service, &session_id, &revision_id)?;

    with_tool_events(
        service,
        "cadastrophe-evaluate-structural",
        &session_id,
        run_id.as_deref(),
        Some(revision_id.clone()),
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
            let evaluation = evaluate_structural_for_revision(
                service,
                app_data_dir,
                &session_id,
                run_id.as_deref(),
                &revision_id,
                &plan,
                args.optional("artifact"),
                args.optional("sidecar"),
            )?;
            let failure_report = structural_failure_report(&evaluation.report);
            Ok(CommandOutput {
                revision_id: Some(revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": evaluation.artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "passed": evaluation.passed,
                    "nextAction": if evaluation.passed { "finalize_or_vlm_judge" } else { "outer_loop_refine_source" }
                }),
                data: json!({
                    "contractType": "cadastrophe.structural_evaluation.v1",
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": evaluation.artifact.as_ref().map(|artifact| artifact.id.clone()),
                    "artifact": evaluation.artifact,
                    "artifactPaths": artifact_paths(evaluation.artifact.as_ref().into_iter()),
                    "structuralReport": evaluation.report,
                    "failureReport": failure_report,
                    "failure_report": failure_report,
                    "nextAction": if evaluation.passed { "finalize_or_vlm_judge" } else { "outer_loop_refine_source" },
                    "next_action": if evaluation.passed { "finalize_or_vlm_judge" } else { "outer_loop_refine_source" }
                }),
            })
        },
    )
}

fn finalize(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let run_id = args.required("run")?.to_string();
    let revision_id = args.required("revision")?.to_string();
    let pass_threshold =
        parse_optional_f64(args.optional("pass-threshold"), 0.8, "pass-threshold")?;
    if !(0.0..=1.0).contains(&pass_threshold) {
        return Err(CliError::invalid_input(
            "--pass-threshold must be between 0.0 and 1.0.",
        ));
    }
    require_revision_in_session(service, &session_id, &revision_id)?;

    with_tool_events(
        service,
        "cadastrophe-finalize",
        &session_id,
        Some(&run_id),
        Some(revision_id.clone()),
        || {
            let workflow_plan = committed_plan_for_run(service, &session_id, &run_id)?;
            validate_plan(&workflow_plan.plan)?;
            let (export, _state) = service
                .export_artifact(ExportArtifactInput {
                    session_id: session_id.clone(),
                    revision_id: Some(revision_id.clone()),
                    format: "stl".to_string(),
                })
                .map_err(CliError::storage)?;
            let final_artifact = require_exported_artifact(export)?;
            record_structural_started(service, &session_id, &run_id, &revision_id)?;
            let evaluation = evaluate_structural_for_revision(
                service,
                app_data_dir,
                &session_id,
                Some(&run_id),
                &revision_id,
                &workflow_plan.plan,
                Some(&final_artifact.id),
                args.optional("sidecar"),
            );
            record_structural_completed(
                service,
                &session_id,
                &run_id,
                &revision_id,
                evaluation.as_ref().ok(),
                evaluation.as_ref().err(),
            )?;
            let evaluation = evaluation?;
            validate_structural_report(&evaluation.report, &run_id, &revision_id)?;
            let failure_report = structural_failure_report(&evaluation.report);
            if !evaluation.passed {
                service
                    .clear_workflow_pending_vlm(&session_id, &run_id)
                    .map_err(CliError::storage)?;
                let workflow = append_outer_iteration(
                    service,
                    &session_id,
                    &run_id,
                    Some(revision_id.clone()),
                    evaluation.report.clone(),
                    None,
                    failure_report.clone(),
                    false,
                )?;
                return Ok(CommandOutput {
                    revision_id: Some(revision_id.clone()),
                    event_payload: json!({
                        "runId": run_id,
                        "revisionId": revision_id,
                        "artifactId": final_artifact.id,
                        "structuralPassed": false,
                        "nextAction": "outer_loop_refine_source"
                    }),
                    data: json!({
                        "contractType": "cadastrophe.finalization.v1",
                        "runId": run_id,
                        "revisionId": revision_id,
                        "artifactId": final_artifact.id,
                        "finalArtifact": final_artifact,
                        "artifactPaths": artifact_paths(std::iter::once(&final_artifact)),
                        "locked": true,
                        "structuralReport": evaluation.report,
                        "failureReport": failure_report,
                        "failure_report": failure_report,
                        "workflow": workflow,
                        "nextAction": "outer_loop_refine_source",
                        "next_action": "outer_loop_refine_source"
                    }),
                });
            }

            let contract = build_vlm_contract(
                &session_id,
                &run_id,
                &revision_id,
                &workflow_plan.plan,
                &final_artifact,
                pass_threshold,
                &evaluation.report,
            )?;
            validate_vlm_contract(
                &contract,
                &session_id,
                &run_id,
                &revision_id,
                &final_artifact.id,
            )?;
            let pending_vlm = CadWorkflowPendingVlm {
                run_id: run_id.clone(),
                artifact_id: final_artifact.id.clone(),
                contract: contract.clone(),
                pass_threshold,
                created_at: timestamp(),
            };
            let workflow = service
                .save_workflow_pending_vlm(&session_id, pending_vlm)
                .map_err(CliError::storage)?;
            Ok(CommandOutput {
                revision_id: Some(revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": final_artifact.id,
                    "structuralPassed": true,
                    "contractType": "cadastrophe.vlm_judge.v1",
                    "nextAction": "vlm_judge"
                }),
                data: json!({
                    "contractType": "cadastrophe.finalization.v1",
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": final_artifact.id,
                    "finalArtifact": final_artifact,
                    "artifactPaths": artifact_paths(std::iter::once(&final_artifact)),
                    "locked": true,
                    "structuralReport": evaluation.report,
                    "vlmContract": contract,
                    "workflow": workflow,
                    "nextAction": "vlm_judge",
                    "next_action": "vlm_judge"
                }),
            })
        },
    )
}

fn vlm_submit(
    args: &ParsedArgs,
    service: &SessionService,
    _app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = args.required("session")?.to_string();
    let run_id = args.required("run")?.to_string();
    let artifact_id = args.required("artifact")?.to_string();
    let report_path = args.required_path("report")?;

    with_tool_events(
        service,
        "cadastrophe-vlm-submit",
        &session_id,
        Some(&run_id),
        None,
        || {
            let report_json = fs::read_to_string(&report_path).map_err(|error| {
                CliError::invalid_input(format!(
                    "Failed to read VLM judge report file {}: {error}",
                    report_path.display()
                ))
            })?;
            let report: Value = serde_json::from_str(&report_json).map_err(|error| {
                CliError::invalid_input(format!(
                    "VLM judge report file {} is not valid JSON: {error}",
                    report_path.display()
                ))
            })?;
            let pending = pending_vlm_for_run(service, &session_id, &run_id, &artifact_id)?;
            validate_vlm_contract_value(&pending.contract)?;
            let revision_id = pending
                .contract
                .get("revisionId")
                .and_then(Value::as_str)
                .map(str::to_string);
            validate_vlm_judge_report(&report, &run_id, &artifact_id)?;
            let score = report.get("score").and_then(Value::as_f64).unwrap_or(0.0);
            let judge_passed = report
                .get("passed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let passed = judge_passed && score >= pending.pass_threshold;
            let failure_report = if passed {
                None
            } else {
                Some(vlm_failure_report(&report, score, pending.pass_threshold))
            };
            let structural_report = pending
                .contract
                .get("structuralReport")
                .cloned()
                .unwrap_or_else(|| {
                    json!({
                        "contractType": "cadastrophe.structural_report.v1",
                        "runId": run_id,
                        "artifactId": artifact_id,
                        "passed": true,
                        "checks": []
                    })
                });
            append_outer_iteration(
                service,
                &session_id,
                &run_id,
                revision_id.clone(),
                structural_report,
                Some(report.clone()),
                failure_report.clone(),
                passed,
            )?;
            let workflow = service
                .clear_workflow_pending_vlm(&session_id, &run_id)
                .map_err(CliError::storage)?;
            if passed {
                let _ = service
                    .update_agent_run(
                        &session_id,
                        &run_id,
                        Some(CadAgentRunStatus::Completed),
                        Some(None),
                        None,
                        None,
                        Some(json!({
                            "artifactId": artifact_id,
                            "nextAction": "complete",
                            "vlmPassed": true
                        })),
                    )
                    .map_err(CliError::storage)?;
            }
            Ok(CommandOutput {
                revision_id,
                event_payload: json!({
                    "runId": run_id,
                    "artifactId": artifact_id,
                    "passed": passed,
                    "score": score,
                    "passThreshold": pending.pass_threshold,
                    "nextAction": if passed { "complete" } else { "outer_loop_refine_source" }
                }),
                data: json!({
                    "contractType": "cadastrophe.vlm_submission.v1",
                    "runId": run_id,
                    "artifactId": artifact_id,
                    "passed": passed,
                    "score": score,
                    "passThreshold": pending.pass_threshold,
                    "vlmReport": report,
                    "failureReport": failure_report,
                    "failure_report": failure_report,
                    "workflow": workflow,
                    "nextAction": if passed { "complete" } else { "outer_loop_refine_source" },
                    "next_action": if passed { "complete" } else { "outer_loop_refine_source" }
                }),
            })
        },
    )
}

#[derive(Debug)]
struct StructuralEvaluation {
    artifact: Option<CadArtifact>,
    report: Value,
    passed: bool,
}

fn evaluate_structural_for_revision(
    service: &SessionService,
    app_data_dir: &PathBuf,
    session_id: &str,
    run_id: Option<&str>,
    revision_id: &str,
    plan: &CadModelPlan,
    artifact_id: Option<&str>,
    sidecar_override: Option<&str>,
) -> CliResult<StructuralEvaluation> {
    let revision = service
        .get_revision(session_id, revision_id)
        .map_err(CliError::not_found)?;
    let artifact = artifact_id
        .map(|artifact_id| {
            revision
                .artifacts
                .iter()
                .find(|artifact| artifact.id == artifact_id)
                .cloned()
                .ok_or_else(|| {
                    CliError::not_found(format!(
                        "CAD artifact {artifact_id} does not belong to revision {revision_id}."
                    ))
                })
        })
        .transpose()?
        .or_else(|| latest_stl_artifact(&revision.artifacts));
    let Some(artifact) = artifact else {
        let report = structural_fallback_report(
            run_id,
            revision_id,
            None,
            "artifact_missing",
            "No STL artifact is available for structural evaluation.",
        );
        return Ok(StructuralEvaluation {
            artifact: None,
            report,
            passed: false,
        });
    };
    if artifact.kind != CadArtifactKind::Stl || artifact.format != "stl" {
        return Err(CliError::invalid_input(format!(
            "CAD artifact {} is not an STL artifact.",
            artifact.id
        )));
    }
    let Some(stl_path) = artifact_filesystem_path(app_data_dir, &artifact) else {
        let report = structural_fallback_report(
            run_id,
            revision_id,
            Some(&artifact.id),
            "artifact_path_missing",
            "STL artifact manifest does not contain path or relativePath metadata.",
        );
        return Ok(StructuralEvaluation {
            artifact: Some(artifact),
            report,
            passed: false,
        });
    };
    let input = json!({
        "runId": run_id.unwrap_or(""),
        "revisionId": revision_id,
        "artifactId": artifact.id,
        "plan": plan,
        "stlPath": stl_path,
        "artifactManifest": artifact,
        "runtimeDiagnostics": revision.diagnostics,
        "sourceText": revision.source
    });
    let report = invoke_structural_sidecar(
        &input,
        run_id,
        revision_id,
        Some(&artifact.id),
        sidecar_override,
    )?;
    let passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(StructuralEvaluation {
        artifact: Some(artifact),
        report,
        passed,
    })
}

fn latest_stl_artifact(artifacts: &[CadArtifact]) -> Option<CadArtifact> {
    artifacts
        .iter()
        .rev()
        .find(|artifact| {
            artifact.kind == CadArtifactKind::Stl
                && artifact.format == "stl"
                && artifact.deleted_at.is_none()
                && artifact.missing_at.is_none()
        })
        .cloned()
}

fn artifact_filesystem_path(app_data_dir: &PathBuf, artifact: &CadArtifact) -> Option<String> {
    let metadata = artifact.metadata.as_ref()?;
    metadata
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            metadata
                .get("relativePath")
                .and_then(Value::as_str)
                .map(|path| app_data_dir.join(path).to_string_lossy().to_string())
        })
}

fn invoke_structural_sidecar(
    input: &Value,
    run_id: Option<&str>,
    revision_id: &str,
    artifact_id: Option<&str>,
    sidecar_override: Option<&str>,
) -> CliResult<Value> {
    let sidecar = resolve_structural_sidecar(sidecar_override);
    if let Some(path) = sidecar_override.map(PathBuf::from) {
        if !path.exists() {
            return Ok(structural_fallback_report(
                run_id,
                revision_id,
                artifact_id,
                "structural_anchor_unavailable",
                "cadastrophe-structural-anchor sidecar is not available.",
            ));
        }
    }
    let mut child = match Command::new(&sidecar)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(structural_fallback_report(
                run_id,
                revision_id,
                artifact_id,
                "structural_anchor_unavailable",
                "cadastrophe-structural-anchor sidecar is not available.",
            ));
        }
        Err(error) => {
            return Ok(structural_fallback_report(
                run_id,
                revision_id,
                artifact_id,
                "structural_anchor_spawn_failed",
                &format!("Failed to start cadastrophe-structural-anchor: {error}"),
            ));
        }
    };
    {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            CliError::storage("Failed to open cadastrophe-structural-anchor stdin.")
        })?;
        stdin
            .write_all(
                serde_json::to_string(input)
                    .map_err(|error| CliError::storage(error.to_string()))?
                    .as_bytes(),
            )
            .map_err(|error| CliError::storage(error.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| CliError::storage(error.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(structural_fallback_report(
            run_id,
            revision_id,
            artifact_id,
            "structural_anchor_failed",
            &format!(
                "cadastrophe-structural-anchor exited with status {}: {}",
                output.status,
                stderr.trim()
            ),
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CliError::invalid_input(format!(
            "cadastrophe-structural-anchor emitted invalid JSON: {error}"
        ))
    })
}

fn resolve_structural_sidecar(sidecar_override: Option<&str>) -> PathBuf {
    if let Some(path) = sidecar_override {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("CADASTROPHE_STRUCTURAL_ANCHOR_PATH") {
        return PathBuf::from(path);
    }
    let executable = if cfg!(target_os = "windows") {
        "cadastrophe-structural-anchor.exe"
    } else {
        "cadastrophe-structural-anchor"
    };
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let adjacent = parent.join(executable);
            if adjacent.exists() {
                return adjacent;
            }
        }
    }
    PathBuf::from(executable)
}

fn structural_fallback_report(
    run_id: Option<&str>,
    revision_id: &str,
    artifact_id: Option<&str>,
    reason: &str,
    message: &str,
) -> Value {
    json!({
        "contractType": "cadastrophe.structural_report.v1",
        "runId": run_id.unwrap_or(""),
        "revisionId": revision_id,
        "artifactId": artifact_id,
        "passed": false,
        "checks": [
            {
                "name": reason,
                "passed": false,
                "severity": "error",
                "message": message
            }
        ],
        "failureReport": {
            "contractType": "cadastrophe.failure_report.v1",
            "reason": reason,
            "nextAction": "refine_plan_or_source"
        }
    })
}

fn validate_structural_report(report: &Value, run_id: &str, revision_id: &str) -> CliResult<()> {
    require_contract_type(
        report,
        "cadastrophe.structural_report.v1",
        "structural report",
    )?;
    if report.get("runId").and_then(Value::as_str).unwrap_or("") != run_id {
        return Err(CliError::invalid_input(
            "Structural report runId does not match finalization run.",
        ));
    }
    if report
        .get("revisionId")
        .and_then(Value::as_str)
        .unwrap_or("")
        != revision_id
    {
        return Err(CliError::invalid_input(
            "Structural report revisionId does not match finalization revision.",
        ));
    }
    if report.get("passed").and_then(Value::as_bool).is_none() {
        return Err(CliError::invalid_input(
            "Structural report must contain a boolean passed field.",
        ));
    }
    Ok(())
}

fn structural_failure_report(report: &Value) -> Option<Value> {
    report.get("failureReport").cloned().or_else(|| {
        let passed = report
            .get("passed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (!passed).then(|| {
            json!({
                "contractType": "cadastrophe.failure_report.v1",
                "reason": "structural_anchor_failed",
                "nextAction": "refine_plan_or_source"
            })
        })
    })
}

fn committed_plan_for_run(
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

fn require_exported_artifact(export: CadExportResult) -> CliResult<CadArtifact> {
    if !export.diagnostics.ok {
        return Err(CliError::precondition_failed(
            "Final STL export diagnostics failed.",
        ));
    }
    export
        .artifact
        .ok_or_else(|| CliError::precondition_failed("Final STL export produced no artifact."))
}

fn record_structural_started(
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

fn record_structural_completed(
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

fn build_vlm_contract(
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    plan: &CadModelPlan,
    artifact: &CadArtifact,
    pass_threshold: f64,
    structural_report: &Value,
) -> CliResult<Value> {
    let metadata = artifact.metadata.as_ref();
    let relative_path = metadata
        .and_then(|metadata| metadata.get("relativePath"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::invalid_input("Final artifact metadata is missing relativePath.")
        })?;
    let sha256 = metadata
        .and_then(|metadata| metadata.get("sha256"))
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::invalid_input("Final artifact metadata is missing sha256."))?;
    Ok(json!({
        "contractType": "cadastrophe.vlm_judge.v1",
        "sessionId": session_id,
        "runId": run_id,
        "revisionId": revision_id,
        "artifactId": artifact.id,
        "passThreshold": pass_threshold,
        "prompt": "Judge whether the final CAD artifact visually satisfies the committed CadModelPlan.",
        "plan": plan,
        "artifact": {
            "format": artifact.format,
            "relativePath": relative_path,
            "sha256": sha256,
            "bytes": artifact.bytes.unwrap_or(0),
            "uri": artifact.uri
        },
        "structuralReport": structural_report
    }))
}

fn validate_vlm_contract(
    contract: &Value,
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    artifact_id: &str,
) -> CliResult<()> {
    validate_vlm_contract_value(contract)?;
    for (field, expected) in [
        ("sessionId", session_id),
        ("runId", run_id),
        ("revisionId", revision_id),
        ("artifactId", artifact_id),
    ] {
        if contract.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(CliError::invalid_input(format!(
                "VLM judge contract {field} does not match expected value."
            )));
        }
    }
    Ok(())
}

fn validate_vlm_contract_value(contract: &Value) -> CliResult<()> {
    require_contract_type(contract, "cadastrophe.vlm_judge.v1", "VLM judge contract")?;
    for field in ["sessionId", "runId", "revisionId", "artifactId", "prompt"] {
        if contract
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(CliError::invalid_input(format!(
                "VLM judge contract missing non-empty {field}."
            )));
        }
    }
    let threshold = contract
        .get("passThreshold")
        .and_then(Value::as_f64)
        .ok_or_else(|| CliError::invalid_input("VLM judge contract missing passThreshold."))?;
    if !(0.0..=1.0).contains(&threshold) {
        return Err(CliError::invalid_input(
            "VLM judge contract passThreshold must be between 0.0 and 1.0.",
        ));
    }
    let plan: CadModelPlan = serde_json::from_value(
        contract
            .get("plan")
            .cloned()
            .ok_or_else(|| CliError::invalid_input("VLM judge contract missing plan."))?,
    )
    .map_err(|error| {
        CliError::invalid_input(format!("VLM judge contract plan is invalid: {error}"))
    })?;
    validate_plan(&plan)?;
    let artifact = contract
        .get("artifact")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::invalid_input("VLM judge contract missing artifact object."))?;
    if artifact.get("format").and_then(Value::as_str) != Some("stl") {
        return Err(CliError::invalid_input(
            "VLM judge contract artifact.format must be stl.",
        ));
    }
    for field in ["relativePath", "sha256"] {
        if artifact
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(CliError::invalid_input(format!(
                "VLM judge contract artifact missing non-empty {field}."
            )));
        }
    }
    Ok(())
}

fn validate_vlm_judge_report(report: &Value, run_id: &str, artifact_id: &str) -> CliResult<()> {
    require_contract_type(
        report,
        "cadastrophe.vlm_judge_report.v1",
        "VLM judge report",
    )?;
    if report.get("runId").and_then(Value::as_str) != Some(run_id) {
        return Err(CliError::invalid_input(
            "VLM judge report runId does not match pending VLM.",
        ));
    }
    if report.get("artifactId").and_then(Value::as_str) != Some(artifact_id) {
        return Err(CliError::invalid_input(
            "VLM judge report artifactId does not match pending VLM.",
        ));
    }
    let score = report
        .get("score")
        .and_then(Value::as_f64)
        .ok_or_else(|| CliError::invalid_input("VLM judge report missing numeric score."))?;
    if !(0.0..=1.0).contains(&score) {
        return Err(CliError::invalid_input(
            "VLM judge report score must be between 0.0 and 1.0.",
        ));
    }
    if report.get("passed").and_then(Value::as_bool).is_none() {
        return Err(CliError::invalid_input(
            "VLM judge report missing boolean passed field.",
        ));
    }
    Ok(())
}

fn vlm_failure_report(report: &Value, score: f64, pass_threshold: f64) -> Value {
    report.get("failureReport").cloned().unwrap_or_else(|| {
        json!({
            "contractType": "cadastrophe.failure_report.v1",
            "reason": "vlm_judge_failed",
            "nextAction": "outer_loop_refine_source",
            "score": score,
            "passThreshold": pass_threshold
        })
    })
}

fn pending_vlm_for_run(
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

fn append_outer_iteration(
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

fn require_contract_type(value: &Value, expected: &'static str, label: &str) -> CliResult<()> {
    let actual = value.get("contractType").and_then(Value::as_str);
    if actual != Some(expected) {
        return Err(CliError::invalid_input(format!(
            "{label} contractType must be {expected}."
        )));
    }
    Ok(())
}

fn parse_optional_f64(value: Option<&str>, fallback: f64, name: &str) -> CliResult<f64> {
    value
        .map(|value| {
            value.parse::<f64>().map_err(|error| {
                CliError::invalid_input(format!("--{name} must be a number: {error}"))
            })
        })
        .unwrap_or(Ok(fallback))
}

fn with_tool_events(
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

fn validate_plan(plan: &CadModelPlan) -> CliResult<()> {
    if plan.schema_version != PLAN_SCHEMA_VERSION {
        return Err(CliError::invalid_input(format!(
            "Unsupported plan schemaVersion {:?}; expected {PLAN_SCHEMA_VERSION}.",
            plan.schema_version
        )));
    }
    if plan.summary.trim().is_empty() {
        return Err(CliError::invalid_input("Plan summary must not be empty."));
    }
    if plan.main_component.name.trim().is_empty() {
        return Err(CliError::invalid_input(
            "Plan mainComponent.name must not be empty.",
        ));
    }
    let ratio = &plan.expected_aspect_ratio;
    for (name, value) in [
        ("expectedAspectRatio.x", ratio.x),
        ("expectedAspectRatio.y", ratio.y),
        ("expectedAspectRatio.z", ratio.z),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(CliError::invalid_input(format!("{name} must be positive.")));
        }
    }
    if !ratio.tolerance.is_finite() || ratio.tolerance < 0.0 {
        return Err(CliError::invalid_input(
            "expectedAspectRatio.tolerance must be zero or positive.",
        ));
    }
    if plan.runtime_constraints.runtime != crate::protocol::CadRuntimeKind::OpenscadWasm {
        return Err(CliError::invalid_input(
            "Track A currently supports CadModelPlan runtimeConstraints.runtime openscad-wasm only.",
        ));
    }
    Ok(())
}

fn require_committed_plan(
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

fn require_revision_in_session(
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

fn artifact_paths<'a>(
    artifacts: impl Iterator<Item = &'a crate::protocol::CadArtifact>,
) -> Vec<String> {
    artifacts
        .filter_map(|artifact| {
            artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("path"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenscadWasmCliOutput {
    diagnostics: CadDiagnostics,
    #[serde(default)]
    mesh: Option<Value>,
    #[serde(default)]
    stl_base64: Option<String>,
    #[serde(default)]
    stl_sha256: Option<String>,
    #[serde(default)]
    stl_bytes: Option<u64>,
}

fn render_open_scad_wasm_cli(
    source: &str,
    app_data_dir: &PathBuf,
) -> CliResult<OpenscadWasmCliOutput> {
    fs::create_dir_all(app_data_dir).map_err(|error| CliError::storage(error.to_string()))?;
    let source_path = app_data_dir.join(format!("openscad-render-{}.scad", uuid::Uuid::new_v4()));
    fs::write(&source_path, source).map_err(|error| CliError::storage(error.to_string()))?;
    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| {
            CliError::runtime("Could not resolve repository root for OpenSCAD WASM helper.")
        })?
        .join("scripts")
        .join("openscad-render.mjs");
    let output = Command::new("node")
        .arg(&script_path)
        .arg(&source_path)
        .output()
        .map_err(|error| {
            CliError::runtime(format!(
                "Failed to execute OpenSCAD WASM helper {}: {error}",
                script_path.display()
            ))
        })?;
    let _ = fs::remove_file(&source_path);
    if !output.status.success() {
        return Err(CliError::runtime(format!(
            "OpenSCAD WASM helper exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CliError::runtime(format!(
            "OpenSCAD WASM helper returned invalid JSON: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    })
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn load_service(app_data_dir: PathBuf) -> CliResult<SessionService> {
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).map_err(|error| {
        CliError::storage(format!(
            "Failed to initialize Cadastrophe storage at {}: {error}",
            layout.app_data_dir().display()
        ))
    })?;
    SessionService::with_repository_without_startup_verification(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .map_err(CliError::storage)
}

fn parse_source_language(value: &str) -> CliResult<CadSourceLanguage> {
    match value {
        "openscad" => Ok(CadSourceLanguage::Openscad),
        "cadquery" => Ok(CadSourceLanguage::Cadquery),
        "freecad-python" | "freecad_python" => Ok(CadSourceLanguage::FreecadPython),
        "cadastrophe-ir" | "cadastrophe_ir" => Ok(CadSourceLanguage::CadastropheIr),
        other => Err(CliError::invalid_input(format!(
            "Unsupported source language {other:?}."
        ))),
    }
}

#[derive(Debug)]
struct ParsedArgs {
    pretty: bool,
    values: BTreeMap<String, String>,
}

impl ParsedArgs {
    fn app_data_dir(&self) -> CliResult<PathBuf> {
        self.optional("app-data-dir")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("CADASTROPHE_APP_DATA_DIR")
                    .ok()
                    .map(PathBuf::from)
            })
            .or_else(default_app_data_dir)
            .ok_or_else(|| {
                CliError::invalid_input(
                    "Could not determine app data directory. Pass --app-data-dir <path>.",
                )
            })
    }

    fn required(&self, name: &str) -> CliResult<&str> {
        self.values
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| CliError::invalid_input(format!("Missing required --{name} option.")))
    }

    fn required_path(&self, name: &str) -> CliResult<PathBuf> {
        Ok(PathBuf::from(self.required(name)?))
    }

    fn optional(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> CliResult<ParsedArgs> {
    let mut pretty = false;
    let mut values = BTreeMap::new();
    let mut pending_key: Option<String> = None;
    for arg in args {
        if let Some(key) = pending_key.take() {
            if arg.starts_with("--") {
                return Err(CliError::invalid_input(format!(
                    "Missing value for --{key}."
                )));
            }
            values.insert(key, arg);
            continue;
        }
        if arg == "--pretty" {
            pretty = true;
        } else if arg == "--json" {
            // JSON is the default. Accept the flag so callers can be explicit.
        } else if let Some(rest) = arg.strip_prefix("--") {
            if let Some((key, value)) = rest.split_once('=') {
                if key.is_empty() || value.is_empty() {
                    return Err(CliError::invalid_input(format!("Invalid option {arg:?}.")));
                }
                values.insert(key.to_string(), value.to_string());
            } else if rest.is_empty() {
                return Err(CliError::invalid_input("Invalid empty option --."));
            } else {
                pending_key = Some(rest.to_string());
            }
        } else {
            return Err(CliError::invalid_input(format!(
                "Unexpected positional argument {arg:?}."
            )));
        }
    }
    if let Some(key) = pending_key {
        return Err(CliError::invalid_input(format!(
            "Missing value for --{key}."
        )));
    }
    Ok(ParsedArgs { pretty, values })
}

#[derive(Debug)]
struct CommandOutput {
    data: Value,
    revision_id: Option<String>,
    event_payload: Value,
}

impl CommandOutput {
    fn new(data: Value) -> Self {
        Self {
            data,
            revision_id: None,
            event_payload: json!({}),
        }
    }
}

type CliResult<T> = Result<T, CliError>;

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    exit_code: i32,
}

impl CliError {
    fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_input",
            message: message.into(),
            exit_code: 2,
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: "not_found",
            message: message.into(),
            exit_code: 3,
        }
    }

    fn precondition_failed(message: impl Into<String>) -> Self {
        Self {
            code: "precondition_failed",
            message: message.into(),
            exit_code: 4,
        }
    }

    fn storage(message: impl Into<String>) -> Self {
        Self {
            code: "storage_error",
            message: message.into(),
            exit_code: 1,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            code: "runtime_error",
            message: message.into(),
            exit_code: 5,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SuccessEnvelope {
    ok: bool,
    command: &'static str,
    data: Value,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    ok: bool,
    command: &'static str,
    error: ErrorBody,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: &'static str,
    message: String,
}

fn emit_success(command: &'static str, pretty: bool, data: Value) -> i32 {
    let envelope = SuccessEnvelope {
        ok: true,
        command,
        data,
    };
    print_json_stdout(&envelope, pretty);
    0
}

fn emit_error(command: &'static str, pretty: bool, error: CliError) -> i32 {
    let exit_code = error.exit_code;
    let envelope = ErrorEnvelope {
        ok: false,
        command,
        error: ErrorBody {
            code: error.code,
            message: error.message,
        },
    };
    print_json_stderr(&envelope, pretty);
    exit_code
}

fn print_json_stdout(value: &impl Serialize, pretty: bool) {
    if pretty {
        println!(
            "{}",
            serde_json::to_string_pretty(value).expect("envelope serializes")
        );
    } else {
        println!(
            "{}",
            serde_json::to_string(value).expect("envelope serializes")
        );
    }
}

fn print_json_stderr(value: &impl Serialize, pretty: bool) {
    if pretty {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(value).expect("envelope serializes")
        );
    } else {
        eprintln!(
            "{}",
            serde_json::to_string(value).expect("envelope serializes")
        );
    }
}

fn merge_event_payload(base: Value, extra: Value) -> Value {
    let mut base = match base {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Value::Object(extra) = extra {
        base.extend(extra);
    }
    Value::Object(base)
}

fn default_app_data_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(APP_IDENTIFIER)
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|root| root.join(APP_IDENTIFIER))
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(".local").join("share")))
            .map(|root| root.join(APP_IDENTIFIER))
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}.{:03}Z", chrono_like_seconds(millis), millis % 1000)
}

fn chrono_like_seconds(millis: u128) -> String {
    let seconds = millis / 1000;
    let tm = time_from_unix(seconds as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second
    )
}

struct SimpleUtcTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn time_from_unix(seconds: i64) -> SimpleUtcTime {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    SimpleUtcTime {
        year,
        month,
        day,
        hour: seconds_of_day / 3600,
        minute: seconds_of_day % 3600 / 60,
        second: seconds_of_day % 60,
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CreateCadSessionInput;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    #[test]
    fn finalize_requires_committed_plan() {
        let app_data_dir = temp_app_data_dir("finalize-requires-plan");
        let service = sqlite_service(&app_data_dir);
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "create model".to_string(),
                None,
                Some("test".to_string()),
                None,
            )
            .unwrap();
        let revision_id = created.state.session.active_revision_id.clone().unwrap();

        let error = finalize(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("revision", &revision_id),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap_err();

        assert_eq!(error.code, "precondition_failed");
        assert!(error.message.contains("cadastrophe-plan-commit"));
    }

    #[cfg(unix)]
    #[test]
    fn finalize_structural_fail_appends_outer_iteration_without_pending_vlm() {
        let app_data_dir = temp_app_data_dir("finalize-structural-fail");
        let service = sqlite_service(&app_data_dir);
        let setup = setup_run_with_plan(&service);
        let sidecar = fake_sidecar(
            &app_data_dir,
            "structural-fail",
            &structural_report_json(&setup.run_id, &setup.revision_id, false),
        );

        let output = finalize(
            &args([
                ("session", &setup.session_id),
                ("run", &setup.run_id),
                ("revision", &setup.revision_id),
                ("sidecar", sidecar.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        assert_eq!(
            output.data["next_action"].as_str(),
            Some("outer_loop_refine_source")
        );
        assert_eq!(
            output.data["failure_report"]["contractType"].as_str(),
            Some("cadastrophe.failure_report.v1")
        );
        let state = service.get_session_state(&setup.session_id).unwrap();
        assert_eq!(state.workflow.pending_vlm.len(), 0);
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert!(!state.workflow.outer_iterations[0].passed);
        assert!(state.workflow.outer_iterations[0].vlm_report.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn finalize_structural_pass_creates_pending_vlm_contract() {
        let app_data_dir = temp_app_data_dir("finalize-structural-pass");
        let service = sqlite_service(&app_data_dir);
        let setup = setup_run_with_plan(&service);
        let sidecar = fake_sidecar(
            &app_data_dir,
            "structural-pass",
            &structural_report_json(&setup.run_id, &setup.revision_id, true),
        );

        let output = finalize(
            &args([
                ("session", &setup.session_id),
                ("run", &setup.run_id),
                ("revision", &setup.revision_id),
                ("sidecar", sidecar.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        assert_eq!(output.data["next_action"].as_str(), Some("vlm_judge"));
        assert_eq!(
            output.data["vlmContract"]["contractType"].as_str(),
            Some("cadastrophe.vlm_judge.v1")
        );
        let state = service.get_session_state(&setup.session_id).unwrap();
        assert_eq!(state.workflow.pending_vlm.len(), 1);
        assert_eq!(state.workflow.pending_vlm[0].run_id, setup.run_id);
        assert_eq!(state.workflow.outer_iterations.len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn cli_workflow_persists_required_tool_event_order() {
        let app_data_dir = temp_app_data_dir("workflow-event-order");
        let service = sqlite_service(&app_data_dir);
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let input_revision_id = created.state.session.active_revision_id.clone();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Create a workflow event order fixture.".to_string(),
                input_revision_id,
                Some("test".to_string()),
                None,
            )
            .unwrap();
        let plan_path = write_json_file(
            &app_data_dir,
            "plan.json",
            &serde_json::from_str(include_str!(
                "../../fixtures/contracts/cad_model_plan.v1.json"
            ))
            .unwrap(),
        );
        let source_path = app_data_dir.join("source.scad");
        fs::write(
            &source_path,
            "// @main_component wall_bracket\ncube([3, 1, 2]);\n",
        )
        .unwrap();

        plan_commit(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("plan", plan_path.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();
        let source_output = source_apply(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("source", source_path.to_str().unwrap()),
                ("language", "openscad"),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();
        let revision_id = source_output.data["revisionId"]
            .as_str()
            .unwrap()
            .to_string();
        preview_render(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("revision", &revision_id),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();
        let sidecar = fake_sidecar(
            &app_data_dir,
            "structural-event-order-pass",
            &structural_report_json(&run.id, &revision_id, true),
        );
        finalize(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("revision", &revision_id),
                ("sidecar", sidecar.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        let state = service.get_session_state(&created.session_id).unwrap();
        let completed_commands = state
            .agent_run_events
            .iter()
            .filter(|event| event.run_id == run.id)
            .filter(|event| event.event_type == CadAgentRunEventType::AgentToolCompleted)
            .filter_map(|event| event.payload.get("command").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(
            completed_commands,
            vec![
                "cadastrophe-plan-commit",
                "cadastrophe-source-apply",
                "cadastrophe-preview-render",
                "cadastrophe-evaluate-structural",
                "cadastrophe-finalize",
            ]
        );
        assert_eq!(state.workflow.plans.len(), 1);
        assert_eq!(state.workflow.pending_vlm.len(), 1);
    }

    #[test]
    fn preview_runtime_failure_records_source_repair_event_diagnostics() {
        let app_data_dir = temp_app_data_dir("preview-source-repair");
        let service = sqlite_service(&app_data_dir);
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let input_revision_id = created.state.session.active_revision_id.clone();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Create source that needs repair.".to_string(),
                input_revision_id,
                Some("test".to_string()),
                None,
            )
            .unwrap();
        let plan_path = write_json_file(
            &app_data_dir,
            "repair-plan.json",
            &serde_json::from_str(include_str!(
                "../../fixtures/contracts/cad_model_plan.v1.json"
            ))
            .unwrap(),
        );
        let source_path = app_data_dir.join("invalid.scad");
        fs::write(
            &source_path,
            "// @main_component wall_bracket\nunsupported();\n",
        )
        .unwrap();

        plan_commit(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("plan", plan_path.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();
        let source_output = source_apply(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("source", source_path.to_str().unwrap()),
                ("language", "openscad"),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();
        let revision_id = source_output.data["revisionId"].as_str().unwrap();
        let preview_output = preview_render(
            &args([
                ("session", &created.session_id),
                ("run", &run.id),
                ("revision", revision_id),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        assert_eq!(preview_output.data["next_action"].as_str(), None);
        assert_eq!(
            preview_output.data["nextAction"].as_str(),
            Some("source_repair")
        );
        let state = service.get_session_state(&created.session_id).unwrap();
        let preview_event = state
            .agent_run_events
            .iter()
            .rev()
            .find(|event| {
                event.run_id == run.id
                    && event.event_type == CadAgentRunEventType::AgentToolCompleted
                    && event.payload.get("command").and_then(Value::as_str)
                        == Some("cadastrophe-preview-render")
            })
            .unwrap();
        assert_eq!(
            preview_event
                .payload
                .get("nextAction")
                .and_then(Value::as_str),
            Some("source_repair")
        );
        assert!(preview_event
            .payload
            .get("diagnostics")
            .and_then(|diagnostics| diagnostics.get("items"))
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| {
                item.get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|message| message.contains("Current top level object is empty"))
            })));
    }

    #[cfg(unix)]
    #[test]
    fn vlm_submit_fail_and_pass_consume_pending_and_append_outer_iterations() {
        let app_data_dir = temp_app_data_dir("vlm-submit");
        let service = sqlite_service(&app_data_dir);
        let failed = setup_pending_vlm(&service, &app_data_dir, "fail");
        let fail_report = write_json_file(
            &app_data_dir,
            "vlm-fail.json",
            &json!({
                "contractType": "cadastrophe.vlm_judge_report.v1",
                "runId": failed.run_id,
                "artifactId": failed.artifact_id,
                "score": 0.4,
                "passed": false,
                "findings": [{"severity": "error", "message": "Missing feature."}],
                "failureReport": {
                    "contractType": "cadastrophe.failure_report.v1",
                    "reason": "missing_feature",
                    "nextAction": "outer_loop_refine_source"
                }
            }),
        );

        let fail_output = vlm_submit(
            &args([
                ("session", &failed.session_id),
                ("run", &failed.run_id),
                ("artifact", &failed.artifact_id),
                ("report", fail_report.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        assert_eq!(
            fail_output.data["next_action"].as_str(),
            Some("outer_loop_refine_source")
        );
        let failed_state = service.get_session_state(&failed.session_id).unwrap();
        assert_eq!(failed_state.workflow.pending_vlm.len(), 0);
        assert_eq!(failed_state.workflow.outer_iterations.len(), 1);
        assert!(!failed_state.workflow.outer_iterations[0].passed);

        let passed = setup_pending_vlm(&service, &app_data_dir, "pass");
        let pass_report = write_json_file(
            &app_data_dir,
            "vlm-pass.json",
            &json!({
                "contractType": "cadastrophe.vlm_judge_report.v1",
                "runId": passed.run_id,
                "artifactId": passed.artifact_id,
                "score": 0.95,
                "passed": true,
                "findings": []
            }),
        );

        let pass_output = vlm_submit(
            &args([
                ("session", &passed.session_id),
                ("run", &passed.run_id),
                ("artifact", &passed.artifact_id),
                ("report", pass_report.to_str().unwrap()),
            ]),
            &service,
            &app_data_dir,
        )
        .unwrap();

        assert_eq!(pass_output.data["next_action"].as_str(), Some("complete"));
        let passed_state = service.get_session_state(&passed.session_id).unwrap();
        let passed_iterations = passed_state
            .workflow
            .outer_iterations
            .iter()
            .filter(|iteration| iteration.run_id == passed.run_id)
            .collect::<Vec<_>>();
        assert_eq!(passed_iterations.len(), 1);
        assert!(passed_iterations[0].passed);
        let run = passed_state
            .agent_runs
            .iter()
            .find(|run| run.id == passed.run_id)
            .unwrap();
        assert_eq!(run.status, CadAgentRunStatus::Completed);
    }

    #[derive(Debug)]
    struct Setup {
        session_id: String,
        run_id: String,
        revision_id: String,
    }

    #[derive(Debug)]
    struct PendingSetup {
        session_id: String,
        run_id: String,
        artifact_id: String,
    }

    #[cfg(unix)]
    fn setup_pending_vlm(
        service: &SessionService,
        app_data_dir: &PathBuf,
        suffix: &str,
    ) -> PendingSetup {
        let setup = setup_run_with_plan(service);
        let sidecar = fake_sidecar(
            app_data_dir,
            &format!("structural-pass-{suffix}"),
            &structural_report_json(&setup.run_id, &setup.revision_id, true),
        );
        finalize(
            &args([
                ("session", &setup.session_id),
                ("run", &setup.run_id),
                ("revision", &setup.revision_id),
                ("sidecar", sidecar.to_str().unwrap()),
            ]),
            service,
            app_data_dir,
        )
        .unwrap();
        let state = service.get_session_state(&setup.session_id).unwrap();
        let pending = state
            .workflow
            .pending_vlm
            .iter()
            .find(|pending| pending.run_id == setup.run_id)
            .unwrap();
        PendingSetup {
            session_id: setup.session_id,
            run_id: setup.run_id,
            artifact_id: pending.artifact_id.clone(),
        }
    }

    fn setup_run_with_plan(service: &SessionService) -> Setup {
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let input_revision_id = created.state.session.active_revision_id.clone();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Create a wall bracket.".to_string(),
                input_revision_id.clone(),
                Some("test".to_string()),
                None,
            )
            .unwrap();
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap();
        service
            .save_workflow_plan(
                &created.session_id,
                CadWorkflowPlan {
                    run_id: run.id.clone(),
                    revision_id: input_revision_id.clone(),
                    plan: plan.clone(),
                    source_language: plan.source_language.clone(),
                    created_at: timestamp(),
                },
            )
            .unwrap();
        let source_result = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "// @main_component wall_bracket\ncube([3, 1, 2]);".to_string(),
                parent_revision_id: input_revision_id,
                parameters: None,
            })
            .unwrap();
        service
            .link_agent_run_output_revision(
                &created.session_id,
                &run.id,
                source_result.revision_id.clone(),
            )
            .unwrap();
        Setup {
            session_id: created.session_id,
            run_id: run.id,
            revision_id: source_result.revision_id,
        }
    }

    fn sqlite_service(app_data_dir: &PathBuf) -> SessionService {
        let layout = StorageLayout::from_app_data_dir(app_data_dir.clone());
        storage::initialize_storage(&layout).unwrap();
        SessionService::with_repository_without_startup_verification(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap()
    }

    #[cfg(unix)]
    fn fake_sidecar(app_data_dir: &PathBuf, name: &str, report: &Value) -> PathBuf {
        let path = app_data_dir.join(name);
        fs::create_dir_all(app_data_dir).unwrap();
        fs::write(
            &path,
            format!(
                "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{}'\n",
                serde_json::to_string(report)
                    .unwrap()
                    .replace('\'', "'\\''")
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn structural_report_json(run_id: &str, revision_id: &str, passed: bool) -> Value {
        let mut report = json!({
            "contractType": "cadastrophe.structural_report.v1",
            "runId": run_id,
            "revisionId": revision_id,
            "artifactId": "artifact-from-fake-sidecar",
            "passed": passed,
            "checks": [
                {
                    "name": "fake_structural_anchor",
                    "passed": passed,
                    "severity": if passed { "info" } else { "error" },
                    "message": if passed { "Structural fixture passed." } else { "Structural fixture failed." }
                }
            ]
        });
        if !passed {
            report["failureReport"] = json!({
                "contractType": "cadastrophe.failure_report.v1",
                "reason": "fake_structural_anchor_failed",
                "nextAction": "refine_plan_or_source"
            });
        }
        report
    }

    fn write_json_file(app_data_dir: &PathBuf, name: &str, value: &Value) -> PathBuf {
        let path = app_data_dir.join(name);
        fs::write(&path, serde_json::to_string(value).unwrap()).unwrap();
        path
    }

    fn args<const N: usize>(values: [(&str, &str); N]) -> ParsedArgs {
        ParsedArgs {
            pretty: false,
            values: values
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    fn temp_app_data_dir(name: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        std::env::temp_dir().join(format!(
            "cadastrophe-cli-{name}-{}-{millis}",
            std::process::id()
        ))
    }
}
