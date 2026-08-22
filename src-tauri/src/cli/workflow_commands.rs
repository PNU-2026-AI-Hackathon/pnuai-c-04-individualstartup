use super::workflow_support::{
    committed_plan_for_run, require_exported_artifact, require_revision_in_session,
    with_tool_events,
};
use super::*;
use crate::protocol::{CadValidationBatchCreate, CadValidationCheckCreate, CadValidationCheckKind};
use crate::validation_plane::contract::VLM_PASS_THRESHOLD;
use std::path::Path;

pub(super) fn finalize(
    args: &ParsedArgs,
    service: &SessionService,
    app_data_dir: &PathBuf,
) -> CliResult<CommandOutput> {
    let session_id = resolve_session_id(args, service)?;
    let run_id = resolve_active_run_id(args, service, &session_id)?;
    ensure_run_belongs_to_session(service, &session_id, &run_id)?;
    let revision_id = resolve_active_revision_id(args, service, &session_id)?;
    if args.optional("pass-threshold").is_some() {
        return Err(CliError::invalid_input(
            "--pass-threshold is no longer configurable; VLM passing requires at least 7/9 total and 2/3 in every subscore.",
        ));
    }
    let pass_threshold = VLM_PASS_THRESHOLD;
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
            let run = service
                .get_agent_run(&session_id, &run_id)
                .map_err(CliError::storage)?
                .ok_or_else(|| CliError::not_found(format!("Agent run not found: {run_id}")))?;
            if run.prompt.trim().is_empty() {
                return Err(CliError::invalid_input(
                    "Agent run prompt cannot be empty when finalization is queued.",
                ));
            }

            let (export, _state) = service
                .export_artifact(ExportArtifactInput {
                    session_id: session_id.clone(),
                    revision_id: Some(revision_id.clone()),
                    format: "stl".to_string(),
                })
                .map_err(CliError::storage)?;
            let final_artifact = require_exported_artifact(export)?;
            let stl_path =
                super::artifacts::artifact_filesystem_path(app_data_dir, &final_artifact)
                    .ok_or_else(|| {
                        CliError::invalid_input(
                            "Final STL artifact metadata is missing path or relativePath.",
                        )
                    })?;
            require_existing_absolute_file(Path::new(&stl_path), "Final STL artifact")?;
            let stl_sha256 = final_artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("sha256"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    CliError::invalid_input("Final STL artifact metadata is missing sha256.")
                })?;
            let actual_sha256 = storage::sha256_hex(
                &std::fs::read(&stl_path).map_err(|error| CliError::storage(error.to_string()))?,
            );
            if actual_sha256 != stl_sha256 {
                return Err(CliError::invalid_input(format!(
                    "Final STL artifact hash mismatch: expected {stl_sha256}, received {actual_sha256}."
                )));
            }

            let structural_sidecar = resolve_frozen_executable(
                args.optional("sidecar"),
                "CADASTROPHE_STRUCTURAL_ANCHOR_PATH",
                "cadastrophe-structural-anchor",
                "Structural sidecar",
            )?;
            let renderer_sidecar = resolve_frozen_executable(
                args.optional("renderer-sidecar"),
                "CADASTROPHE_VLM_RENDERER_PATH",
                "cadastrophe-vlm-renderer",
                "VLM renderer sidecar",
            )?;
            let dfm_inputs = crate::dfm::prepare_evaluation_inputs(
                app_data_dir,
                args.optional("prusaslicer-path"),
                args.optional("dfm-profile"),
            )
            .map_err(CliError::invalid_input)?;

            let common = json!({
                "sessionId": session_id,
                "runId": run_id,
                "revisionId": revision_id,
                "artifactId": final_artifact.id,
                "stl": {
                    "path": stl_path,
                    "sha256": stl_sha256,
                    "artifact": final_artifact,
                },
                "plan": workflow_plan.plan,
            });
            let structural_input = extend_contract(
                &common,
                "cadastrophe.structural_check_input.v1",
                json!({
                    "sidecarPath": structural_sidecar,
                    "sidecarSha256": executable_sha256(&structural_sidecar)?,
                }),
            )?;
            let dfm_input = extend_contract(
                &common,
                "cadastrophe.dfm_check_input.v1",
                json!({"prepared": dfm_inputs}),
            )?;
            let vlm_input = extend_contract(
                &common,
                "cadastrophe.vlm_check_input.v1",
                json!({
                    "userRequest": run.prompt,
                    "passThreshold": pass_threshold,
                    "rendererSidecarPath": renderer_sidecar,
                    "rendererSidecarSha256": executable_sha256(&renderer_sidecar)?,
                }),
            )?;

            let (batch, checks) = service
                .create_validation_batch(CadValidationBatchCreate {
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    revision_id: revision_id.clone(),
                    artifact_id: final_artifact.id.clone(),
                    checks: vec![
                        CadValidationCheckCreate {
                            kind: CadValidationCheckKind::Structural,
                            input_contract: structural_input,
                        },
                        CadValidationCheckCreate {
                            kind: CadValidationCheckKind::Dfm,
                            input_contract: dfm_input,
                        },
                        CadValidationCheckCreate {
                            kind: CadValidationCheckKind::Vlm,
                            input_contract: vlm_input,
                        },
                    ],
                })
                .map_err(CliError::storage)?;
            let check_ids = checks
                .iter()
                .map(|check| json!({"id": check.id, "kind": check.kind, "status": check.status}))
                .collect::<Vec<_>>();
            Ok(CommandOutput {
                revision_id: Some(revision_id.clone()),
                event_payload: json!({
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": final_artifact.id,
                    "batchId": batch.id,
                    "batchStatus": batch.status,
                    "checks": check_ids,
                    "attempt": batch.attempt,
                    "nextAction": "validation_queued"
                }),
                data: json!({
                    "contractType": "cadastrophe.finalization.v2",
                    "runId": run_id,
                    "revisionId": revision_id,
                    "artifactId": final_artifact.id,
                    "finalArtifact": final_artifact,
                    "artifactPaths": artifact_paths([&final_artifact].into_iter()),
                    "locked": true,
                    "validationBatch": batch,
                    "validationChecks": checks,
                    "nextAction": "validation_queued",
                    "next_action": "validation_queued"
                }),
            })
        },
    )
}

fn require_existing_absolute_file(path: &Path, label: &str) -> CliResult<()> {
    if !path.is_absolute() {
        return Err(CliError::invalid_input(format!(
            "{label} path must be absolute."
        )));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|error| CliError::invalid_input(format!("{label} is unavailable: {error}")))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(CliError::invalid_input(format!(
            "{label} must be a non-empty file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn resolve_frozen_executable(
    explicit: Option<&str>,
    environment_key: &str,
    executable_name: &str,
    label: &str,
) -> CliResult<String> {
    let configured = explicit
        .map(PathBuf::from)
        .or_else(|| std::env::var(environment_key).ok().map(PathBuf::from));
    let path = if let Some(path) = configured {
        path
    } else {
        let platform_name = if cfg!(target_os = "windows") {
            format!("{executable_name}.exe")
        } else {
            executable_name.to_string()
        };
        let adjacent = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.join(&platform_name)))
            .filter(|path| path.is_file());
        adjacent
            .or_else(|| {
                std::env::var_os("PATH").and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|directory| directory.join(&platform_name))
                        .find(|candidate| candidate.is_file())
                })
            })
            .ok_or_else(|| {
                CliError::invalid_input(format!(
                    "{label} could not be resolved to an existing executable before queueing."
                ))
            })?
    };
    let path = std::fs::canonicalize(&path).map_err(|error| {
        CliError::invalid_input(format!(
            "Failed to resolve {label} {}: {error}",
            path.display()
        ))
    })?;
    require_existing_absolute_file(&path, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&path)
            .map_err(|error| CliError::invalid_input(error.to_string()))?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(CliError::invalid_input(format!(
                "{label} is not executable: {}",
                path.display()
            )));
        }
    }
    Ok(path.to_string_lossy().to_string())
}

fn extend_contract(common: &Value, contract_type: &str, extra: Value) -> CliResult<Value> {
    let mut contract = common
        .as_object()
        .cloned()
        .ok_or_else(|| CliError::storage("Validation common input is not an object."))?;
    contract.insert("contractType".into(), Value::String(contract_type.into()));
    let extra = extra
        .as_object()
        .ok_or_else(|| CliError::storage("Validation input extension is not an object."))?;
    for (key, value) in extra {
        contract.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(contract))
}

fn executable_sha256(path: &str) -> CliResult<String> {
    Ok(storage::sha256_hex(&std::fs::read(path).map_err(
        |error| {
            CliError::invalid_input(format!("Failed to fingerprint executable {path}: {error}"))
        },
    )?))
}
