use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

pub const DATABASE_FILE_NAME: &str = "cadastrophe.sqlite3";
pub const ARTIFACT_DIR_NAME: &str = "artifacts";
pub const SCHEMA_VERSION: i64 = 1;

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

pub fn run_migrations(connection: &mut Connection) -> StorageResult<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
          version INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );
        "#,
    )?;

    for migration in MIGRATIONS {
        let already_applied: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            params![migration.version],
            |row| row.get(0),
        )?;
        if already_applied {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )?;
        transaction.commit()?;
    }

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

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: SCHEMA_VERSION,
    name: "milestone_2_1_initial_persistence_schema",
    sql: r#"
      CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        title TEXT,
        selected_runtime TEXT NOT NULL,
        status TEXT NOT NULL,
        active_revision_id TEXT,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        last_viewed_at TEXT,
        connected_ui_clients INTEGER NOT NULL DEFAULT 0,
        archived_at TEXT,
        deleted_at TEXT,
        metadata_json TEXT,
        FOREIGN KEY(active_revision_id) REFERENCES revisions(id)
          ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED
      );

      CREATE TABLE revisions (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        parent_revision_id TEXT,
        restored_from_revision_id TEXT,
        source_language TEXT NOT NULL,
        source TEXT NOT NULL,
        parameters_json TEXT NOT NULL DEFAULT '[]',
        diagnostics_json TEXT NOT NULL DEFAULT '{"ok":true,"elapsedMs":0,"items":[]}',
        created_at TEXT NOT NULL,
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(parent_revision_id) REFERENCES revisions(id) ON DELETE SET NULL,
        FOREIGN KEY(restored_from_revision_id) REFERENCES revisions(id) ON DELETE SET NULL
      );

      CREATE TABLE artifacts (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        revision_id TEXT NOT NULL,
        kind TEXT NOT NULL,
        format TEXT NOT NULL,
        relative_path TEXT NOT NULL UNIQUE,
        uri TEXT NOT NULL,
        sha256 TEXT NOT NULL,
        bytes INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        deleted_at TEXT,
        missing_at TEXT,
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE CASCADE
      );

      CREATE TABLE conversation_messages (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        revision_id TEXT,
        run_id TEXT,
        role TEXT NOT NULL,
        content TEXT NOT NULL,
        created_at TEXT NOT NULL,
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE SET NULL,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE SET NULL
      );

      CREATE TABLE agent_runs (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        input_revision_id TEXT,
        output_revision_id TEXT,
        status TEXT NOT NULL,
        prompt TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        started_at TEXT,
        completed_at TEXT,
        error TEXT,
        active_step TEXT,
        external_agent TEXT,
        external_thread_id TEXT,
        external_turn_id TEXT,
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(input_revision_id) REFERENCES revisions(id) ON DELETE SET NULL,
        FOREIGN KEY(output_revision_id) REFERENCES revisions(id) ON DELETE SET NULL
      );

      CREATE TABLE agent_run_events (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        run_id TEXT NOT NULL,
        revision_id TEXT,
        event_type TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        payload_json TEXT NOT NULL DEFAULT '{}',
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE SET NULL,
        UNIQUE(run_id, sequence)
      );

      CREATE INDEX idx_revisions_session_created_at
        ON revisions(session_id, created_at);
      CREATE INDEX idx_artifacts_session_revision
        ON artifacts(session_id, revision_id);
      CREATE INDEX idx_conversation_messages_session_created_at
        ON conversation_messages(session_id, created_at);
      CREATE INDEX idx_agent_runs_session_created_at
        ON agent_runs(session_id, created_at);
      CREATE INDEX idx_agent_run_events_run_sequence
        ON agent_run_events(run_id, sequence);
    "#,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_places_database_and_artifacts_under_app_data() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-storage-test-{}", uuid::Uuid::new_v4()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir.clone());

        assert_eq!(
            layout.database_path(),
            app_data_dir.join(DATABASE_FILE_NAME).as_path()
        );
        assert_eq!(
            layout.artifact_root(),
            app_data_dir.join(ARTIFACT_DIR_NAME).as_path()
        );
        assert_eq!(
            layout
                .artifact_relative_path("session-1", "revision-1", "artifact-1", "stl")
                .unwrap(),
            PathBuf::from("artifacts")
                .join("session-1")
                .join("revision-1")
                .join("artifact-1.stl")
        );
        assert_eq!(
            layout
                .artifact_path("session-1", "revision-1", "artifact-1", "stl")
                .unwrap(),
            app_data_dir
                .join("artifacts")
                .join("session-1")
                .join("revision-1")
                .join("artifact-1.stl")
        );
    }

    #[test]
    fn migration_runner_creates_milestone_2_1_schema_once() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-storage-test-{}", uuid::Uuid::new_v4()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);

        initialize_storage(&layout).unwrap();
        initialize_storage(&layout).unwrap();

        let connection = Connection::open(layout.database_path()).unwrap();
        for table in [
            "schema_migrations",
            "sessions",
            "revisions",
            "artifacts",
            "conversation_messages",
            "agent_runs",
            "agent_run_events",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {table}");
        }

        let artifact_columns = table_columns(&connection, "artifacts");
        for column in [
            "sha256",
            "bytes",
            "created_at",
            "deleted_at",
            "missing_at",
            "metadata_json",
        ] {
            assert!(
                artifact_columns.iter().any(|candidate| candidate == column),
                "missing artifacts.{column}"
            );
        }

        let migration_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(migration_count, 1);
        let (version, name): (i64, String) = connection
            .query_row(
                "SELECT version, name FROM schema_migrations ORDER BY version",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(name, "milestone_2_1_initial_persistence_schema");

        let applied_versions = applied_schema_versions(&connection);
        assert_eq!(applied_versions, vec![SCHEMA_VERSION]);
        let mut connection = Connection::open(layout.database_path()).unwrap();
        run_migrations(&mut connection).unwrap();
        assert_eq!(applied_schema_versions(&connection), applied_versions);
    }

    fn table_columns(connection: &Connection, table: &str) -> Vec<String> {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        statement
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<Vec<String>, _>>()
            .unwrap()
    }

    fn applied_schema_versions(connection: &Connection) -> Vec<i64> {
        let mut statement = connection
            .prepare("SELECT version FROM schema_migrations ORDER BY version")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<i64>, _>>()
            .unwrap()
    }
}
