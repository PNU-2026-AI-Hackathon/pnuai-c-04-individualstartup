use super::workflow_support::{
    append_outer_iteration, committed_plan_for_run, pending_vlm_for_run,
    record_structural_completed, record_structural_started, require_exported_artifact,
    require_revision_in_session, with_tool_events,
};
use super::*;

pub(super) fn evaluate_structural(
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

pub(super) fn finalize(
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

            let rendered_images = render_vlm_images_for_artifact(
                service,
                app_data_dir,
                &session_id,
                &run_id,
                &revision_id,
                &workflow_plan.plan,
                &final_artifact,
                args.optional("renderer-sidecar"),
            )?;
            let contract = build_vlm_contract(
                &session_id,
                &run_id,
                &revision_id,
                &workflow_plan.plan,
                &final_artifact,
                &rendered_images,
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
                    "renderedImageArtifactId": rendered_images.id,
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
                    "renderedImageArtifact": rendered_images,
                    "artifactPaths": artifact_paths([&final_artifact, &rendered_images].into_iter()),
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

pub(super) fn vlm_submit(
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
