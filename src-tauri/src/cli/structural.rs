use crate::cli::artifacts::{artifact_filesystem_path, latest_stl_artifact};
use crate::cli::support::{require_contract_type, CliError, CliResult};
use crate::protocol::{CadArtifactKind, CadModelPlan};
use crate::session_service::SessionService;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub(crate) struct StructuralEvaluation {
    pub(crate) report: Value,
    pub(crate) passed: bool,
}

pub(crate) fn evaluate_structural_for_revision(
    service: &SessionService,
    app_data_dir: &Path,
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
        return Err(CliError::runtime(
            "No STL artifact is available for structural evaluation.",
        ));
    };
    if artifact.kind != CadArtifactKind::Stl || artifact.format != "stl" {
        return Err(CliError::invalid_input(format!(
            "CAD artifact {} is not an STL artifact.",
            artifact.id
        )));
    }
    let Some(stl_path) = artifact_filesystem_path(app_data_dir, &artifact) else {
        return Err(CliError::runtime(
            "STL artifact manifest does not contain path or relativePath metadata.",
        ));
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
    let report = invoke_structural_sidecar(&input, sidecar_override)?;
    let passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| CliError::invalid_input("Structural report is missing boolean passed."))?;
    Ok(StructuralEvaluation { report, passed })
}

fn invoke_structural_sidecar(input: &Value, sidecar_override: Option<&str>) -> CliResult<Value> {
    let sidecar = resolve_structural_sidecar(sidecar_override);
    if let Some(path) = sidecar_override.map(PathBuf::from) {
        if !path.exists() {
            return Err(CliError::runtime(
                "cadgen-ax-structural-anchor sidecar is not available.",
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
            return Err(CliError::runtime(
                "cadgen-ax-structural-anchor sidecar is not available.",
            ));
        }
        Err(error) => {
            return Err(CliError::runtime(format!(
                "Failed to start cadgen-ax-structural-anchor: {error}"
            )));
        }
    };
    {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            CliError::storage("Failed to open cadgen-ax-structural-anchor stdin.")
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
        return Err(CliError::runtime(format!(
            "cadgen-ax-structural-anchor exited with status {}: {}",
            output.status,
            stderr.trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CliError::invalid_input(format!(
            "cadgen-ax-structural-anchor emitted invalid JSON: {error}"
        ))
    })
}

fn resolve_structural_sidecar(sidecar_override: Option<&str>) -> PathBuf {
    if let Some(path) = sidecar_override {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("CADGEN_AX_STRUCTURAL_ANCHOR_PATH") {
        return PathBuf::from(path);
    }
    let executable = if cfg!(target_os = "windows") {
        "cadgen-ax-structural-anchor.exe"
    } else {
        "cadgen-ax-structural-anchor"
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

pub(crate) fn validate_structural_report(
    report: &Value,
    run_id: &str,
    revision_id: &str,
) -> CliResult<()> {
    require_contract_type(
        report,
        "cadgen-ax.structural_report.v1",
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
    let passed = report
        .get("passed")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            CliError::invalid_input("Structural report must contain a boolean passed field.")
        })?;
    if !passed {
        structural_failure_report(report)?;
    }
    Ok(())
}

pub(super) fn structural_failure_report(report: &Value) -> CliResult<Value> {
    let failure_report = report
        .get("failureReport")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CliError::invalid_input("Failed structural report must contain a failureReport object.")
        })?;
    require_contract_type(
        &Value::Object(failure_report.clone()),
        "cadgen-ax.failure_report.v1",
        "structural failure report",
    )?;
    if failure_report
        .get("reason")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err(CliError::invalid_input(
            "Structural failure report must contain a non-empty reason.",
        ));
    }
    let mut failure_report = failure_report.clone();
    failure_report.insert("structuralPassed".to_string(), Value::Bool(false));
    failure_report.insert("structuralReport".to_string(), report.clone());
    failure_report.insert(
        "nextAction".to_string(),
        Value::String("outer_loop_refine_source".to_string()),
    );
    failure_report.insert(
        "next_action".to_string(),
        Value::String("outer_loop_refine_source".to_string()),
    );
    Ok(Value::Object(failure_report))
}
