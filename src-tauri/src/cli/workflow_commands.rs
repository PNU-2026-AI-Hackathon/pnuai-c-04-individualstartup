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
