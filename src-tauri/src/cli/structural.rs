use crate::cli::artifacts::{artifact_filesystem_path, latest_stl_artifact};
use crate::cli::support::{require_contract_type, CliError, CliResult};
use crate::protocol::{CadArtifact, CadArtifactKind, CadModelPlan};
use crate::session_service::SessionService;
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug)]
pub(super) struct StructuralEvaluation {
    pub(super) artifact: Option<CadArtifact>,
    pub(super) report: Value,
    pub(super) passed: bool,
}

pub(super) fn evaluate_structural_for_revision(
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
        let report = structural_error_report(
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
        let report = structural_error_report(
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
            return Ok(structural_error_report(
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
            return Ok(structural_error_report(
                run_id,
                revision_id,
                artifact_id,
                "structural_anchor_unavailable",
                "cadastrophe-structural-anchor sidecar is not available.",
            ));
        }
        Err(error) => {
            return Ok(structural_error_report(
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
        return Ok(structural_error_report(
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

fn structural_error_report(
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

pub(super) fn validate_structural_report(
    report: &Value,
    run_id: &str,
    revision_id: &str,
) -> CliResult<()> {
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

pub(super) fn structural_failure_report(report: &Value) -> Option<Value> {
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
