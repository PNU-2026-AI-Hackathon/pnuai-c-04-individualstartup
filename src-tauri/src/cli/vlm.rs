use crate::cli::artifacts::{artifact_filesystem_path, base64_encode, ok_cli_diagnostics};
use crate::cli::support::{require_contract_type, CliError, CliResult, ParsedArgs};
use crate::protocol::{CadArtifact, CadArtifactKind, CadModelPlan, PersistRuntimeArtifactInput};
use crate::session_service::SessionService;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(super) fn build_vlm_submission(args: &ParsedArgs) -> CliResult<Value> {
    args.require_only(&[
        "structure",
        "components",
        "proportions",
        "inconsistency",
        "diagnostic",
    ])?;
    let structure = parse_subscore(args, "structure")?;
    let components = parse_subscore(args, "components")?;
    let proportions = parse_subscore(args, "proportions")?;
    let mut submission = serde_json::Map::from_iter([
        (
            "contractType".to_string(),
            json!(crate::validation_plane::contract::SUBMISSION_CONTRACT_TYPE),
        ),
        (
            "scores".to_string(),
            json!({
                "structure": structure,
                "components": components,
                "proportions": proportions,
            }),
        ),
    ]);
    if let Some(inconsistency) = optional_non_empty(args, "inconsistency")? {
        submission.insert("inconsistencies".to_string(), json!([inconsistency]));
    }
    if let Some(diagnostic) = optional_non_empty(args, "diagnostic")? {
        submission.insert("diagnostic".to_string(), json!(diagnostic));
    }
    Ok(Value::Object(submission))
}

fn parse_subscore(args: &ParsedArgs, name: &str) -> CliResult<u64> {
    let raw = args.required(name)?;
    raw.parse::<u64>()
        .ok()
        .filter(|score| *score <= 3)
        .ok_or_else(|| {
            CliError::invalid_input(format!("--{name} must be an integer from 0 through 3."))
        })
}

fn optional_non_empty<'a>(args: &'a ParsedArgs, name: &str) -> CliResult<Option<&'a str>> {
    match args.optional(name) {
        Some(value) if value.trim().is_empty() => Err(CliError::invalid_input(format!(
            "--{name} cannot be empty."
        ))),
        value => Ok(value),
    }
}

pub(crate) fn render_vlm_images_for_artifact(
    service: &SessionService,
    app_data_dir: &Path,
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    plan: &CadModelPlan,
    artifact: &CadArtifact,
    renderer_override: Option<&str>,
) -> CliResult<CadArtifact> {
    if artifact.kind != CadArtifactKind::Stl || artifact.format != "stl" {
        return Err(CliError::invalid_input(format!(
            "CAD artifact {} is not an STL artifact.",
            artifact.id
        )));
    }
    let stl_path = artifact_filesystem_path(app_data_dir, artifact).ok_or_else(|| {
        CliError::invalid_input("Final STL artifact metadata is missing path or relativePath.")
    })?;
    let output_dir = app_data_dir
        .join("vlm-renders")
        .join(session_id)
        .join(revision_id)
        .join(run_id)
        .join(&artifact.id);
    fs::create_dir_all(&output_dir).map_err(|error| CliError::storage(error.to_string()))?;
    let source_artifact_sha256 = artifact
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("sha256"))
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::invalid_input("Final STL artifact metadata is missing sha256."))?
        .to_string();
    let source_hash = artifact
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("sourceHash"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let input = json!({
        "runId": run_id,
        "sessionId": session_id,
        "revisionId": revision_id,
        "artifactId": artifact.id,
        "sourceArtifactSha256": source_artifact_sha256,
        "sourceHash": source_hash,
        "stlPath": stl_path,
        "outputDirectory": output_dir,
        "viewMode": "9-view",
        "resolution": { "width": 512, "height": 512 },
        "artifactManifest": artifact,
        "plan": plan
    });
    let manifest = invoke_vlm_renderer_sidecar(&input, renderer_override)?;
    validate_vlm_renderer_manifest(&manifest, revision_id, &artifact.id)?;
    let image_path = manifest
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::runtime("VLM renderer manifest is missing path."))?;
    let png_bytes = fs::read(image_path).map_err(|error| {
        CliError::runtime(format!(
            "Failed to read rendered VLM image {image_path}: {error}"
        ))
    })?;
    if png_bytes.is_empty() {
        return Err(CliError::runtime(
            "VLM renderer produced an empty PNG grid artifact.",
        ));
    }
    let metadata = json!({
        "renderer": manifest.get("renderer").cloned().unwrap_or_else(|| json!("cadastrophe-vlm-renderer")),
        "rendererEngine": manifest.get("rendererEngine").cloned().unwrap_or_else(|| json!("unknown")),
        "viewMode": manifest.get("viewMode").cloned().unwrap_or_else(|| json!("9-view")),
        "views": manifest.get("views").cloned().unwrap_or_else(|| json!([])),
        "resolution": manifest.get("resolution").cloned().unwrap_or_else(|| json!({"width": 512, "height": 512})),
        "sourceArtifactId": artifact.id,
        "sourceArtifactSha256": source_artifact_sha256,
        "revisionId": revision_id,
        "sourceHash": source_hash,
        "bytes": png_bytes.len() as u64,
        "renderManifest": manifest
    });
    let persisted = service
        .persist_runtime_artifact(PersistRuntimeArtifactInput {
            session_id: session_id.to_string(),
            revision_id: revision_id.to_string(),
            kind: CadArtifactKind::RenderImage,
            format: "png".to_string(),
            contents_base64: base64_encode(&png_bytes),
            diagnostics: ok_cli_diagnostics(0),
            metadata: metadata
                .as_object()
                .cloned()
                .ok_or_else(|| CliError::runtime("VLM render metadata is not a JSON object."))?,
        })
        .map_err(CliError::storage)?;
    Ok(persisted.artifact)
}

fn invoke_vlm_renderer_sidecar(input: &Value, renderer_override: Option<&str>) -> CliResult<Value> {
    let sidecar = resolve_vlm_renderer_sidecar(renderer_override);
    if let Some(path) = renderer_override.map(PathBuf::from) {
        if !path.exists() {
            return Err(CliError::runtime(
                "cadastrophe-vlm-renderer sidecar is not available.",
            ));
        }
    }
    let mut child = Command::new(&sidecar)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            CliError::runtime(format!(
                "Failed to start cadastrophe-vlm-renderer {}: {error}",
                sidecar.display()
            ))
        })?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CliError::runtime("Failed to open cadastrophe-vlm-renderer stdin."))?;
        stdin
            .write_all(
                serde_json::to_string(input)
                    .map_err(|error| CliError::runtime(error.to_string()))?
                    .as_bytes(),
            )
            .map_err(|error| CliError::runtime(error.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| CliError::runtime(error.to_string()))?;
    if !output.status.success() {
        return Err(CliError::runtime(format!(
            "cadastrophe-vlm-renderer exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CliError::runtime(format!(
            "cadastrophe-vlm-renderer emitted invalid JSON: {error}"
        ))
    })
}

fn resolve_vlm_renderer_sidecar(renderer_override: Option<&str>) -> PathBuf {
    if let Some(path) = renderer_override {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var("CADASTROPHE_VLM_RENDERER_PATH") {
        return PathBuf::from(path);
    }
    let executable = if cfg!(target_os = "windows") {
        "cadastrophe-vlm-renderer.exe"
    } else {
        "cadastrophe-vlm-renderer"
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

fn validate_vlm_renderer_manifest(
    manifest: &Value,
    revision_id: &str,
    source_artifact_id: &str,
) -> CliResult<()> {
    require_contract_type(
        manifest,
        "cadastrophe.vlm_render_manifest.v1",
        "VLM render manifest",
    )?;
    if manifest.get("revisionId").and_then(Value::as_str) != Some(revision_id) {
        return Err(CliError::runtime(
            "VLM render manifest revisionId does not match finalization revision.",
        ));
    }
    if manifest.get("sourceArtifactId").and_then(Value::as_str) != Some(source_artifact_id) {
        return Err(CliError::runtime(
            "VLM render manifest sourceArtifactId does not match final STL artifact.",
        ));
    }
    if manifest.get("format").and_then(Value::as_str) != Some("png") {
        return Err(CliError::runtime("VLM render manifest format must be png."));
    }
    if manifest
        .get("views")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err(CliError::runtime(
            "VLM render manifest must include at least one view.",
        ));
    }
    for field in ["path", "sha256"] {
        if manifest
            .get(field)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(CliError::runtime(format!(
                "VLM render manifest missing non-empty {field}."
            )));
        }
    }
    if !is_positive_json_number(manifest.get("bytes")) {
        return Err(CliError::runtime(
            "VLM render manifest reports an empty PNG artifact.",
        ));
    }
    Ok(())
}

fn is_positive_json_number(value: Option<&Value>) -> bool {
    let Some(Value::Number(number)) = value else {
        return false;
    };
    if let Some(value) = number.as_u64() {
        return value > 0;
    }
    if let Some(value) = number.as_i64() {
        return value > 0;
    }
    number
        .as_f64()
        .is_some_and(|value| value.is_finite() && value > 0.0)
}

pub(crate) fn build_vlm_contract(rendered_image: &CadArtifact) -> CliResult<Value> {
    let image_metadata = rendered_image.metadata.as_ref();
    let image_path = image_metadata
        .and_then(|metadata| metadata.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::invalid_input("Rendered image artifact metadata is missing path.")
        })?;
    Ok(json!({
        "contractType": "cadastrophe.vlm_judge.v1",
        "handoff": "VLM Judge Handoff needed.",
        "renderedImages": {
            "available": true,
            "path": image_path
        },
    }))
}
