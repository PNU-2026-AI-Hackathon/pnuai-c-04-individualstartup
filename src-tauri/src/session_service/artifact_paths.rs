use super::*;

pub(super) fn collect_artifact_files(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_artifact_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

pub(super) fn validate_artifact_relative_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || !path.starts_with(storage::ARTIFACT_DIR_NAME)
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Artifact relativePath escapes artifact root: {path:?}"
        ));
    }
    Ok(())
}

pub(super) fn validate_artifact_absolute_path(
    path: &Path,
    artifact_root: &Path,
) -> Result<(), String> {
    if !path.starts_with(artifact_root) {
        return Err(format!("Artifact path is outside artifact root: {path:?}"));
    }
    Ok(())
}
