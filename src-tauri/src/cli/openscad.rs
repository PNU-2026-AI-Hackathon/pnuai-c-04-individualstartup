use crate::cli::support::{CliError, CliResult};
use crate::protocol::CadDiagnostics;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpenscadWasmCliOutput {
    pub(super) diagnostics: CadDiagnostics,
    #[serde(default)]
    pub(super) mesh: Option<Value>,
    #[serde(default)]
    pub(super) stl_base64: Option<String>,
    #[serde(default)]
    pub(super) stl_sha256: Option<String>,
    #[serde(default)]
    pub(super) stl_bytes: Option<u64>,
}

pub(super) fn render_open_scad_wasm_cli(
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
