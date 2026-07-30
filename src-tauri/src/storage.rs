use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

mod migrations;
#[cfg(test)]
mod tests;

pub use migrations::run_migrations;

pub const DATABASE_FILE_NAME: &str = "cadastrophe.sqlite3";
pub const ARTIFACT_DIR_NAME: &str = "artifacts";

pub type StorageResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageLayout {
    app_data_dir: PathBuf,
    database_path: PathBuf,
    artifact_root: PathBuf,
}

impl StorageLayout {
    pub fn from_app_data_dir(app_data_dir: PathBuf) -> Self {
        Self {
            database_path: app_data_dir.join(DATABASE_FILE_NAME),
            artifact_root: app_data_dir.join(ARTIFACT_DIR_NAME),
            app_data_dir,
        }
    }

    #[cfg(test)]
    pub fn from_artifact_root(artifact_root: PathBuf) -> Self {
        let app_data_dir = artifact_root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| artifact_root.clone());
        Self {
            database_path: app_data_dir.join(DATABASE_FILE_NAME),
            artifact_root,
            app_data_dir,
        }
    }

    pub fn app_data_dir(&self) -> &Path {
        &self.app_data_dir
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn artifact_root(&self) -> &Path {
        &self.artifact_root
    }

    pub fn artifact_relative_path(
        &self,
        session_id: &str,
        revision_id: &str,
        artifact_id: &str,
        format: &str,
    ) -> StorageResult<PathBuf> {
        validate_path_segment("session_id", session_id)?;
        validate_path_segment("revision_id", revision_id)?;
        validate_path_segment("artifact_id", artifact_id)?;
        validate_path_segment("format", format)?;
        Ok(PathBuf::from(ARTIFACT_DIR_NAME)
            .join(session_id)
            .join(revision_id)
            .join(format!("{artifact_id}.{format}")))
    }

    pub fn artifact_path(
        &self,
        session_id: &str,
        revision_id: &str,
        artifact_id: &str,
        format: &str,
    ) -> StorageResult<PathBuf> {
        validate_path_segment("session_id", session_id)?;
        validate_path_segment("revision_id", revision_id)?;
        validate_path_segment("artifact_id", artifact_id)?;
        validate_path_segment("format", format)?;
        Ok(self
            .artifact_root
            .join(session_id)
            .join(revision_id)
            .join(format!("{artifact_id}.{format}")))
    }
}

pub fn initialize_storage(layout: &StorageLayout) -> StorageResult<()> {
    fs::create_dir_all(layout.app_data_dir())?;
    fs::create_dir_all(layout.artifact_root())?;
    let mut connection = Connection::open(layout.database_path())?;
    run_migrations(&mut connection)?;
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_path_segment(name: &str, value: &str) -> StorageResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(format!("Invalid artifact path {name}: {value:?}").into());
    }
    Ok(())
}
