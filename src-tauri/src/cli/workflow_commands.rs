use super::workflow_support::{
    append_outer_iteration, committed_plan_for_run, record_structural_completed,
    record_structural_started, require_exported_artifact, require_revision_in_session,
    with_tool_events,
};
use super::*;

pub(super) fn finalize(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = resolve_session_id(args, service)?;
    let run_id = resolve_active_run_id(args, service, &session_id)?;
    ensure_run_belongs_to_session(service, &session_id, &run_id)?;
    let revision_id = resolve_active_revision_id(args, service, &session_id)?;
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
            // Both validators must evaluate the same STL before deciding whether to
            // hand off to VLM. DFM operational errors are deliberately not converted
            // into a synthetic failed report.
            let dfm_evaluation = crate::dfm::evaluate(
                service,
                app_data_dir,
                &session_id,
                &run_id,
                &revision_id,
                &final_artifact,
                args.optional("prusaslicer-path"),
                args.optional("dfm-profile"),
            )
            .map_err(CliError::runtime)?;
            crate::dfm::validate_report(&dfm_evaluation.report, &run_id, &revision_id)
                .map_err(CliError::invalid_input)?;
            let final_artifact = dfm_evaluation.stl_artifact.clone();
            if !evaluation.passed || !dfm_evaluation.passed {
                let failure_report =
                    crate::dfm::failure_report(&evaluation.report, &dfm_evaluation.report);
                service
                    .clear_workflow_pending_vlm(&session_id, &run_id)
                    .map_err(CliError::storage)?;
                let workflow = append_outer_iteration(
                    service,
                    &session_id,
                    &run_id,
                    Some(revision_id.clone()),
                    evaluation.report.clone(),
                    dfm_evaluation.report.clone(),
                    None,
                    Some(failure_report.clone()),
                    false,
                )?;
                return Ok(CommandOutput {
                    revision_id: Some(revision_id.clone()),
                    event_payload: json!({
                        "runId": run_id,
                        "revisionId": revision_id,
                        "artifactId": final_artifact.id,
                        "structuralPassed": evaluation.passed,
                        "dfmPassed": dfm_evaluation.passed,
                        "nextAction": "outer_loop_refine_source"
                    }),
                    data: json!({
                        "contractType": "cadastrophe.finalization.v1",
                        "runId": run_id,
                        "revisionId": revision_id,
                        "artifactId": final_artifact.id,
                        "finalArtifact": final_artifact,
                        "gcodeArtifact": dfm_evaluation.gcode_artifact,
                        "dfmReportArtifact": dfm_evaluation.report_artifact,
                        "artifactPaths": artifact_paths([&final_artifact, &dfm_evaluation.gcode_artifact, &dfm_evaluation.report_artifact].into_iter()),
                        "locked": true,
                        "structuralReport": evaluation.report,
                        "dfmReport": dfm_evaluation.report,
                        "failureReport": failure_report.clone(),
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
            let contract = build_vlm_contract(&rendered_images)?;
            validate_vlm_contract(&contract)?;
            let pending_vlm = CadWorkflowPendingVlm {
                run_id: run_id.clone(),
                artifact_id: final_artifact.id.clone(),
                revision_id: Some(revision_id.clone()),
                contract: contract.clone(),
                pass_threshold,
                structural_report: Some(evaluation.report.clone()),
                dfm_report: Some(dfm_evaluation.report.clone()),
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
                    "dfmPassed": true,
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
                    "gcodeArtifact": dfm_evaluation.gcode_artifact,
                    "dfmReportArtifact": dfm_evaluation.report_artifact,
                    "artifactPaths": artifact_paths([&final_artifact, &dfm_evaluation.gcode_artifact, &dfm_evaluation.report_artifact, &rendered_images].into_iter()),
                    "locked": true,
                    "structuralReport": evaluation.report,
                    "dfmReport": dfm_evaluation.report,
                    "vlmContract": contract,
                    "workflow": workflow,
                    "nextAction": "vlm_judge",
                    "next_action": "vlm_judge"
                }),
            })
        },
    )
}
