use crate::protocol::{CadArtifact, CadArtifactKind, CadDiagnostics, PersistRuntimeArtifactInput};
use crate::session_service::SessionService;
use crate::storage;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

const SETTINGS_DIRECTORY: &str = "dfm";
const SETTINGS_FILE: &str = "settings.json";
const PROFILE_FILE: &str = "profile.ini";
const DEFAULT_PROFILE: &str = include_str!("../../profile.ini");
const REQUIRED_PROFILE_KEYS: &[&str] = &[
    "printer_technology",
    "nozzle_diameter",
    "filament_diameter",
    "layer_height",
    "gcode_flavor",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInput {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileContentsInput {
    pub contents: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProfileInput {
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableValidation {
    pub path: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileValidation {
    pub hash: String,
    pub key_settings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DfmProfileSettings {
    pub contents: String,
    pub hash: String,
    pub key_settings: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DfmSettings {
    pub prusaslicer_executable: Option<String>,
    pub executable_validation: Option<ExecutableValidation>,
    pub profile: DfmProfileSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DfmDesignContext {
    pub(crate) printer_technology: String,
    pub(crate) nozzle_diameter_mm: f64,
    pub(crate) support_material_enabled: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImportedProfile {
    pub contents: String,
    pub source_path: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportedProfile {
    pub path: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSettings {
    prusaslicer_executable: Option<String>,
    executable_validation: Option<ExecutableValidation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedDfmInputs {
    pub executable: ExecutableValidation,
    pub executable_sha256: String,
    pub profile_contents: String,
    pub profile_source: String,
    pub profile: ProfileValidation,
}

#[derive(Debug)]
pub(crate) struct DfmEvaluation {
    pub report: Value,
    pub passed: bool,
}

pub fn get_settings(app_data_dir: &Path) -> Result<DfmSettings, String> {
    ensure_profile_exists(app_data_dir)?;
    let stored = read_stored_settings(app_data_dir)?;
    let contents = fs::read_to_string(profile_path(app_data_dir))
        .map_err(|error| format!("Failed to read the saved DFM profile: {error}"))?;
    let validation = validate_profile(&contents)?;
    Ok(DfmSettings {
        prusaslicer_executable: stored.prusaslicer_executable,
        executable_validation: stored.executable_validation,
        profile: DfmProfileSettings {
            contents,
            hash: validation.hash,
            key_settings: validation.key_settings,
        },
    })
}

pub(crate) fn load_design_context(app_data_dir: &Path) -> Result<DfmDesignContext, String> {
    ensure_profile_exists(app_data_dir)?;
    let contents = fs::read_to_string(profile_path(app_data_dir))
        .map_err(|error| format!("Failed to read the saved DFM profile: {error}"))?;
    design_context_from_profile(&contents)
}

pub fn validate_executable(path: &str) -> Result<ExecutableValidation, String> {
    let resolved = resolve_executable_path(Path::new(path))?;
    if !resolved.is_absolute() {
        return Err("PrusaSlicer executable path must be absolute.".to_string());
    }
    let metadata = fs::metadata(&resolved).map_err(|error| {
        format!(
            "PrusaSlicer executable does not exist at {}: {error}",
            resolved.display()
        )
    })?;
    if !metadata.is_file() {
        return Err(format!(
            "PrusaSlicer executable path is not a file: {}",
            resolved.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!(
                "PrusaSlicer executable is not executable: {}",
                resolved.display()
            ));
        }
    }
    let output = Command::new(&resolved)
        .arg("--help")
        .output()
        .map_err(|error| format!("Failed to execute PrusaSlicer --help: {error}"))?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("PrusaSlicer --help stdout is not UTF-8: {error}"))?;
    let stderr = String::from_utf8(output.stderr)
        .map_err(|error| format!("PrusaSlicer --help stderr is not UTF-8: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "PrusaSlicer --help exited with status {}. stdout: {} stderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        ));
    }
    let version = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|line| line.to_ascii_lowercase().contains("prusaslicer"))
        .ok_or_else(|| {
            format!(
                "Configured executable did not identify itself as PrusaSlicer. stdout: {} stderr: {}",
                stdout.trim(),
                stderr.trim()
            )
        })?
        .to_string();
    Ok(ExecutableValidation {
        path: resolved.to_string_lossy().to_string(),
        version,
    })
}

pub fn save_executable(app_data_dir: &Path, path: &str) -> Result<ExecutableValidation, String> {
    let validation = validate_executable(path)?;
    let stored = StoredSettings {
        prusaslicer_executable: Some(validation.path.clone()),
        executable_validation: Some(validation.clone()),
    };
    write_stored_settings(app_data_dir, &stored)?;
    Ok(validation)
}

pub fn validate_profile(contents: &str) -> Result<ProfileValidation, String> {
    let entries = parse_profile_entries(contents)?;
    validate_profile_entries(contents, &entries)
}

fn parse_profile_entries(contents: &str) -> Result<BTreeMap<String, String>, String> {
    if contents.trim().is_empty() {
        return Err("DFM profile must not be empty.".to_string());
    }
    let mut entries = BTreeMap::new();
    for (index, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with('[')
        {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            format!(
                "Invalid DFM profile syntax at line {}: expected key = value.",
                index + 1
            )
        })?;
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "Invalid DFM profile syntax at line {}: key is empty.",
                index + 1
            ));
        }
        if entries
            .insert(key.to_string(), value.trim().to_string())
            .is_some()
        {
            return Err(format!("DFM profile contains duplicate key {key:?}."));
        }
    }
    Ok(entries)
}

fn validate_profile_entries(
    contents: &str,
    entries: &BTreeMap<String, String>,
) -> Result<ProfileValidation, String> {
    let mut key_settings = BTreeMap::new();
    for key in REQUIRED_PROFILE_KEYS {
        let value = entries
            .get(*key)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("DFM profile is missing required setting {key}."))?;
        key_settings.insert((*key).to_string(), value.clone());
    }
    if let Some(bed_shape) = entries.get("bed_shape").filter(|value| !value.is_empty()) {
        parse_bed_shape(bed_shape)?;
        key_settings.insert("bed_shape".to_string(), bed_shape.clone());
    }
    if key_settings.get("printer_technology").map(String::as_str) != Some("FFF") {
        return Err("DFM profile printer_technology must be FFF.".to_string());
    }
    for key in ["nozzle_diameter", "filament_diameter", "layer_height"] {
        let value = key_settings[key]
            .parse::<f64>()
            .map_err(|error| format!("DFM profile {key} must be numeric: {error}"))?;
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("DFM profile {key} must be greater than zero."));
        }
    }
    Ok(ProfileValidation {
        hash: storage::sha256_hex(contents.as_bytes()),
        key_settings,
    })
}

fn parse_bed_shape(value: &str) -> Result<Vec<[f64; 2]>, String> {
    let points = value
        .split(',')
        .map(|point| {
            let point = point.trim();
            let (x, y) = point.split_once('x').ok_or_else(|| {
                format!("DFM profile bed_shape point {point:?} must use x-by-y coordinates.")
            })?;
            let x = x.trim().parse::<f64>().map_err(|error| {
                format!("DFM profile bed_shape X coordinate {x:?} must be numeric: {error}")
            })?;
            let y = y.trim().parse::<f64>().map_err(|error| {
                format!("DFM profile bed_shape Y coordinate {y:?} must be numeric: {error}")
            })?;
            if !x.is_finite() || !y.is_finite() {
                return Err("DFM profile bed_shape coordinates must be finite.".to_string());
            }
            Ok([x, y])
        })
        .collect::<Result<Vec<_>, String>>()?;
    if points.len() < 3 {
        return Err("DFM profile bed_shape must contain at least three points.".to_string());
    }
    Ok(points)
}

fn design_context_from_profile(contents: &str) -> Result<DfmDesignContext, String> {
    let entries = parse_profile_entries(contents)?;
    validate_profile_entries(contents, &entries)?;
    let printer_technology = entries["printer_technology"].clone();
    let nozzle_diameter_mm = entries["nozzle_diameter"]
        .parse::<f64>()
        .map_err(|error| format!("DFM profile nozzle_diameter must be numeric: {error}"))?;
    let support_material_enabled = match entries
        .get("support_material")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "DFM profile is missing design setting support_material.".to_string())?
        .as_str()
    {
        "0" => false,
        "1" => true,
        _ => return Err("DFM profile support_material must be 0 or 1.".to_string()),
    };
    Ok(DfmDesignContext {
        printer_technology,
        nozzle_diameter_mm,
        support_material_enabled,
    })
}

pub fn save_profile(app_data_dir: &Path, contents: &str) -> Result<DfmProfileSettings, String> {
    let validation = validate_profile(contents)?;
    fs::create_dir_all(settings_dir(app_data_dir))
        .map_err(|error| format!("Failed to create DFM settings directory: {error}"))?;
    fs::write(profile_path(app_data_dir), contents)
        .map_err(|error| format!("Failed to save DFM profile: {error}"))?;
    Ok(DfmProfileSettings {
        contents: contents.to_string(),
        hash: validation.hash,
        key_settings: validation.key_settings,
    })
}

pub fn restore_default_profile(app_data_dir: &Path) -> Result<DfmProfileSettings, String> {
    save_profile(app_data_dir, DEFAULT_PROFILE)
}

pub fn import_profile(path: &str) -> Result<ImportedProfile, String> {
    let path = require_absolute_file(path, "DFM profile import")?;
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("Failed to import DFM profile {}: {error}", path.display()))?;
    validate_profile(&contents)?;
    Ok(ImportedProfile {
        contents,
        source_path: path.to_string_lossy().to_string(),
    })
}

pub fn export_profile(path: &str, contents: &str) -> Result<ExportedProfile, String> {
    validate_profile(contents)?;
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("DFM profile export path must be absolute.".to_string());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create DFM profile export directory: {error}"))?;
    }
    fs::write(&path, contents)
        .map_err(|error| format!("Failed to export DFM profile {}: {error}", path.display()))?;
    Ok(ExportedProfile {
        path: path.to_string_lossy().to_string(),
    })
}

/// Resolves and validates every mutable DFM option before a validation batch is queued.
/// The returned value is persisted with the check so retries use the exact same executable
/// and profile bytes rather than whatever happens to be configured at execution time.
pub(crate) fn prepare_evaluation_inputs(
    app_data_dir: &Path,
    executable_override: Option<&str>,
    profile_override: Option<&str>,
) -> Result<PreparedDfmInputs, String> {
    let (executable, profile_contents, profile_source) =
        if executable_override.is_some() || profile_override.is_some() {
            let settings = get_settings(app_data_dir)?;
            let executable = executable_override
                .map(str::to_string)
                .or(settings.prusaslicer_executable)
                .ok_or_else(|| "PrusaSlicer executable is not configured.".to_string())?;
            let (contents, source) = if let Some(path) = profile_override {
                let imported = import_profile(path)?;
                (imported.contents, imported.source_path)
            } else {
                (
                    settings.profile.contents,
                    profile_path(app_data_dir).to_string_lossy().to_string(),
                )
            };
            (executable, contents, source)
        } else {
            let settings = get_settings(app_data_dir)?;
            let executable = settings
                .prusaslicer_executable
                .ok_or_else(|| "PrusaSlicer executable is not configured.".to_string())?;
            (
                executable,
                settings.profile.contents,
                profile_path(app_data_dir).to_string_lossy().to_string(),
            )
        };
    let executable = validate_executable(&executable)?;
    let profile = validate_profile(&profile_contents)?;
    let executable_sha256 = storage::sha256_hex(&fs::read(&executable.path).map_err(|error| {
        format!(
            "Failed to fingerprint PrusaSlicer executable {}: {error}",
            executable.path
        )
    })?);
    Ok(PreparedDfmInputs {
        executable,
        executable_sha256,
        profile_contents,
        profile_source,
        profile,
    })
}

pub(crate) fn evaluate_prepared(
    service: &SessionService,
    app_data_dir: &Path,
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    stl_artifact: &CadArtifact,
    prepared: &PreparedDfmInputs,
) -> Result<DfmEvaluation, String> {
    let executable = &prepared.executable;
    let profile_contents = &prepared.profile_contents;
    let profile_source = &prepared.profile_source;
    let profile = &prepared.profile;
    let bed_shape = profile
        .key_settings
        .get("bed_shape")
        .map(|value| parse_bed_shape(value))
        .transpose()?;
    let executable_sha256 = storage::sha256_hex(&fs::read(&executable.path).map_err(|error| {
        format!(
            "Failed to verify PrusaSlicer executable {}: {error}",
            executable.path
        )
    })?);
    if executable_sha256 != prepared.executable_sha256 {
        return Err(format!(
            "PrusaSlicer executable changed after the validation batch was queued: {}.",
            executable.path
        ));
    }
    let stl_path = stl_artifact
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "Final STL artifact metadata is missing an absolute path.".to_string())?;
    if !stl_path.is_absolute() || !stl_path.is_file() {
        return Err(format!(
            "Final STL artifact is unavailable at {}.",
            stl_path.display()
        ));
    }

    let work_dir = app_data_dir
        .join("dfm-work")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&work_dir)
        .map_err(|error| format!("Failed to create PrusaSlicer work directory: {error}"))?;
    let work_stl = work_dir.join("model.stl");
    let work_profile = work_dir.join(PROFILE_FILE);
    fs::copy(&stl_path, &work_stl)
        .map_err(|error| format!("Failed to stage STL for PrusaSlicer: {error}"))?;
    fs::write(&work_profile, profile_contents)
        .map_err(|error| format!("Failed to stage DFM profile for PrusaSlicer: {error}"))?;

    let output = Command::new(&executable.path)
        .arg("--load")
        .arg(&work_profile)
        .arg("--export-gcode")
        .arg(&work_stl)
        .current_dir(&work_dir)
        .output()
        .map_err(|error| format!("Failed to execute PrusaSlicer: {error}"))?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("PrusaSlicer stdout is not UTF-8: {error}"))?;
    let stderr = String::from_utf8(output.stderr)
        .map_err(|error| format!("PrusaSlicer stderr is not UTF-8: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "PrusaSlicer exited with status {}. stdout: {} stderr: {}",
            output.status,
            stdout.trim(),
            stderr.trim()
        ));
    }
    let gcode_path = work_stl.with_extension("gcode");
    let gcode = fs::read(&gcode_path).map_err(|error| {
        format!(
            "PrusaSlicer completed without producing G-code at {}: {error}",
            gcode_path.display()
        )
    })?;
    if gcode.is_empty() {
        return Err("PrusaSlicer produced an empty G-code file.".to_string());
    }

    let diagnostics = parse_diagnostics(&stdout, &stderr);
    let passed = !diagnostics.iter().any(|item| item["severity"] == "error");
    let mut gcode_metadata = json!({
        "contractType": "cadastrophe.gcode_artifact.v1",
        "runId": run_id,
        "revisionId": revision_id,
        "sourceArtifactId": stl_artifact.id,
        "profileHash": profile.hash,
        "profileSource": profile_source,
        "prusaslicerVersion": executable.version
    });
    if let Some(bed_shape) = bed_shape {
        gcode_metadata["bedShape"] = json!(bed_shape);
    }
    let gcode_artifact = service
        .persist_runtime_artifact(PersistRuntimeArtifactInput {
            session_id: session_id.to_string(),
            revision_id: revision_id.to_string(),
            kind: CadArtifactKind::Gcode,
            format: "gcode".to_string(),
            contents_base64: base64::engine::general_purpose::STANDARD.encode(&gcode),
            diagnostics: CadDiagnostics {
                ok: true,
                elapsed_ms: 0,
                items: Vec::new(),
            },
            metadata: object(gcode_metadata)?,
        })
        .map_err(|error| format!("Failed to persist generated G-code: {error}"))?
        .artifact;
    // Do not mutate the STL manifest here. Structural/DFM/render consume the same immutable
    // snapshot concurrently; the coordinator applies profileHash only while settling the batch.
    let stl_artifact = stl_artifact.clone();

    let checks = vec![
        json!({"name":"prusaslicer_execution","passed":true,"severity":"info","message":"PrusaSlicer exited successfully."}),
        json!({"name":"gcode_generated","passed":true,"severity":"info","message":format!("Generated {} bytes of G-code.", gcode.len())}),
        json!({"name":"slicer_diagnostics","passed":passed,"severity":if passed {"info"} else {"error"},"message":if passed {"No error diagnostics were emitted."} else {"PrusaSlicer emitted error diagnostics."}}),
    ];
    let report = json!({
        "contractType": "cadastrophe.dfm_report.v1",
        "runId": run_id,
        "revisionId": revision_id,
        "artifactId": stl_artifact.id,
        "passed": passed,
        "checks": checks,
        "diagnostics": diagnostics,
        "profileHash": profile.hash,
        "keySettings": profile.key_settings,
        "gcodeArtifactId": gcode_artifact.id,
        "process": {"exitCode": output.status.code(), "stdout": stdout, "stderr": stderr},
        "prusaslicer": {"path": executable.path, "version": executable.version}
    });
    validate_report(&report, run_id, revision_id)?;
    let report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("Failed to serialize DFM report: {error}"))?;
    let _ = service
        .persist_runtime_artifact(PersistRuntimeArtifactInput {
            session_id: session_id.to_string(),
            revision_id: revision_id.to_string(),
            kind: CadArtifactKind::Metadata,
            format: "json".to_string(),
            contents_base64: base64::engine::general_purpose::STANDARD.encode(report_bytes),
            diagnostics: CadDiagnostics {
                ok: true,
                elapsed_ms: 0,
                items: Vec::new(),
            },
            metadata: object(json!({
                "contractType": "cadastrophe.dfm_report_artifact.v1",
                "runId": run_id,
                "sourceArtifactId": stl_artifact.id,
                "gcodeArtifactId": gcode_artifact.id,
                "profileHash": profile.hash
            }))?,
        })
        .map_err(|error| format!("Failed to persist DFM report: {error}"))?
        .artifact;
    Ok(DfmEvaluation { report, passed })
}

pub(crate) fn validate_report(
    report: &Value,
    run_id: &str,
    revision_id: &str,
) -> Result<(), String> {
    if report.get("contractType").and_then(Value::as_str) != Some("cadastrophe.dfm_report.v1") {
        return Err("DFM report contractType must be cadastrophe.dfm_report.v1.".to_string());
    }
    if report.get("runId").and_then(Value::as_str) != Some(run_id)
        || report.get("revisionId").and_then(Value::as_str) != Some(revision_id)
    {
        return Err("DFM report runId or revisionId does not match finalization.".to_string());
    }
    if report.get("passed").and_then(Value::as_bool).is_none()
        || report.get("checks").and_then(Value::as_array).is_none()
        || report
            .get("diagnostics")
            .and_then(Value::as_array)
            .is_none()
        || report.get("profileHash").and_then(Value::as_str).is_none()
        || report
            .get("gcodeArtifactId")
            .and_then(Value::as_str)
            .is_none()
    {
        return Err("DFM report is missing required fields.".to_string());
    }
    Ok(())
}

fn parse_diagnostics(stdout: &str, stderr: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| ("stdout", line))
        .chain(stderr.lines().map(|line| ("stderr", line)))
        .filter_map(|(stream, line)| {
            let message = line.trim();
            if message.is_empty() {
                return None;
            }
            let lower = message.to_ascii_lowercase();
            let severity = if lower.contains("error") || lower.contains("fatal") {
                "error"
            } else if lower.contains("warning") || lower.contains("warn:") {
                "warning"
            } else {
                return None;
            };
            Some(json!({"severity":severity,"stream":stream,"message":message}))
        })
        .collect()
}

fn object(value: Value) -> Result<Map<String, Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "Artifact metadata must be a JSON object.".to_string())
}

fn resolve_executable_path(path: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    if path.extension().and_then(|extension| extension.to_str()) == Some("app") {
        let candidate = path.join("Contents/MacOS/PrusaSlicer");
        if !candidate.is_file() {
            return Err(format!(
                "Selected .app does not contain Contents/MacOS/PrusaSlicer: {}",
                path.display()
            ));
        }
        return Ok(candidate);
    }
    Ok(path.to_path_buf())
}

fn require_absolute_file(path: &str, label: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("{label} path must be absolute."));
    }
    if !path.is_file() {
        return Err(format!("{label} file does not exist: {}", path.display()));
    }
    Ok(path)
}

fn settings_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(SETTINGS_DIRECTORY)
}
fn settings_path(app_data_dir: &Path) -> PathBuf {
    settings_dir(app_data_dir).join(SETTINGS_FILE)
}
fn profile_path(app_data_dir: &Path) -> PathBuf {
    settings_dir(app_data_dir).join(PROFILE_FILE)
}

fn ensure_profile_exists(app_data_dir: &Path) -> Result<(), String> {
    if !profile_path(app_data_dir).exists() {
        restore_default_profile(app_data_dir)?;
    }
    Ok(())
}

fn read_stored_settings(app_data_dir: &Path) -> Result<StoredSettings, String> {
    let path = settings_path(app_data_dir);
    if !path.exists() {
        return Ok(StoredSettings::default());
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("Failed to read DFM settings {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("DFM settings file is invalid JSON: {error}"))
}

fn write_stored_settings(app_data_dir: &Path, settings: &StoredSettings) -> Result<(), String> {
    fs::create_dir_all(settings_dir(app_data_dir))
        .map_err(|error| format!("Failed to create DFM settings directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("Failed to serialize DFM settings: {error}"))?;
    fs::write(settings_path(app_data_dir), bytes)
        .map_err(|error| format!("Failed to save DFM settings: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_validation_is_stable_and_extracts_key_settings() {
        let first = validate_profile(DEFAULT_PROFILE).unwrap();
        let second = validate_profile(DEFAULT_PROFILE).unwrap();
        assert_eq!(first.hash, second.hash);
        assert_eq!(first.hash.len(), 64);
        assert_eq!(first.key_settings["printer_technology"], "FFF");
        assert_eq!(first.key_settings["bed_shape"], "0x0,200x0,200x200,0x200");
    }

    #[test]
    fn bed_shape_parser_accepts_rectangular_prusa_coordinates() {
        assert_eq!(
            parse_bed_shape("0x0,50x0,50x50,0x50").unwrap(),
            vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0], [0.0, 50.0]]
        );
    }

    #[test]
    fn bed_shape_parser_rejects_invalid_coordinates() {
        let error = parse_bed_shape("0x0,50xnope,0x50").unwrap_err();
        assert!(error.contains("must be numeric"));
    }

    #[test]
    fn profile_validation_rejects_missing_required_setting() {
        let error = validate_profile("printer_technology = FFF\n").unwrap_err();
        assert!(error.contains("nozzle_diameter"));
    }

    #[test]
    fn profile_validation_rejects_malformed_line() {
        let error = validate_profile("not-an-assignment\n").unwrap_err();
        assert!(error.contains("line 1"));
    }

    #[test]
    fn design_context_extracts_only_modeling_constraints() {
        let context = design_context_from_profile(DEFAULT_PROFILE).unwrap();
        assert_eq!(context.printer_technology, "FFF");
        assert_eq!(context.nozzle_diameter_mm, 0.4);
        assert!(!context.support_material_enabled);
    }

    #[test]
    fn design_context_requires_valid_support_material_setting() {
        let missing = DEFAULT_PROFILE
            .lines()
            .filter(|line| !line.starts_with("support_material ="))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(design_context_from_profile(&missing)
            .unwrap_err()
            .contains("support_material"));

        let invalid = DEFAULT_PROFILE.replace("support_material = 0", "support_material = auto");
        assert!(design_context_from_profile(&invalid)
            .unwrap_err()
            .contains("must be 0 or 1"));
    }
}
