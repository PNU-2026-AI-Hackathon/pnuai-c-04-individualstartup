use crate::protocol::{CadArtifact, CadArtifactKind, CadDiagnostics};
use serde_json::Value;
use std::path::Path;

pub(super) fn latest_stl_artifact(artifacts: &[CadArtifact]) -> Option<CadArtifact> {
    artifacts
        .iter()
        .rev()
        .find(|artifact| {
            artifact.kind == CadArtifactKind::Stl
                && artifact.format == "stl"
                && artifact.deleted_at.is_none()
                && artifact.missing_at.is_none()
        })
        .cloned()
}

pub(crate) fn artifact_filesystem_path(
    app_data_dir: &Path,
    artifact: &CadArtifact,
) -> Option<String> {
    let metadata = artifact.metadata.as_ref()?;
    metadata
        .get("path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            metadata
                .get("relativePath")
                .and_then(Value::as_str)
                .map(|path| app_data_dir.join(path).to_string_lossy().to_string())
        })
}

pub(super) fn artifact_paths<'a>(artifacts: impl Iterator<Item = &'a CadArtifact>) -> Vec<String> {
    artifacts
        .filter_map(|artifact| {
            artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("path"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(super) fn ok_cli_diagnostics(elapsed_ms: u64) -> CadDiagnostics {
    CadDiagnostics {
        ok: true,
        elapsed_ms,
        items: Vec::new(),
    }
}
