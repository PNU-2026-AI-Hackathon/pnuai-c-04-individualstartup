use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OpenscadWasmNodeOutput {
    pub(super) diagnostics: CadDiagnostics,
    #[serde(default)]
    pub(super) mesh: Option<CadMesh>,
    #[serde(default)]
    pub(super) stl_base64: Option<String>,
    #[serde(default)]
    pub(super) stl_sha256: Option<String>,
    #[serde(default)]
    pub(super) stl_bytes: Option<u64>,
}

pub(super) fn render_open_scad_wasm_node(
    source: &str,
    app_data_dir: &Path,
) -> Result<OpenscadWasmNodeOutput, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let source_path = app_data_dir.join(format!("openscad-render-{}.scad", Uuid::new_v4()));
    fs::write(&source_path, source).map_err(|error| error.to_string())?;
    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "Could not resolve repository root for OpenSCAD WASM helper.".to_string())?
        .join("scripts")
        .join("openscad-render.mjs");
    let output = Command::new("node")
        .arg(&script_path)
        .arg(&source_path)
        .output()
        .map_err(|error| {
            format!(
                "Failed to execute OpenSCAD WASM helper {}: {error}",
                script_path.display()
            )
        })?;
    let _ = fs::remove_file(&source_path);
    if !output.status.success() {
        return Err(format!(
            "OpenSCAD WASM helper exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "OpenSCAD WASM helper returned invalid JSON: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub(super) fn runtime_artifact_metadata(
    source: &str,
    parameters: &[CadParameter],
    rendered: &OpenscadWasmNodeOutput,
    phase: &str,
) -> Result<Value, String> {
    Ok(json!({
        "runtime": "openscad-wasm",
        "sourceLanguage": "openscad",
        "sourceHash": source_hash(source),
        "parameterHash": storage::sha256_hex(
            serde_json::to_string(parameters)
                .map_err(|error| error.to_string())?
                .as_bytes()
        ),
        "stlSha256": rendered.stl_sha256,
        "stlBytes": rendered.stl_bytes,
        "renderDurationMs": rendered.diagnostics.elapsed_ms,
        "diagnosticsSource": "openscad-wasm",
        "phase": phase
    }))
}
