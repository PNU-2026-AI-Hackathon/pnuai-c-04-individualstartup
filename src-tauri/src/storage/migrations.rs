use super::StorageResult;
use rusqlite::{params, Connection};

#[cfg(test)]
pub(super) const SCHEMA_VERSION: i64 = 4;

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
pub(super) struct Migration {
    pub(super) version: i64,
    pub(super) name: &'static str,
    pub(super) sql: &'static str,
}

pub(super) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
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
    },
    Migration {
        version: 2,
        name: "milestone_3_0_workflow_state_spine",
        sql: r#"
      CREATE TABLE workflow_plans (
        run_id TEXT PRIMARY KEY,
        revision_id TEXT,
        plan_json TEXT NOT NULL,
        source_language TEXT NOT NULL,
        created_at TEXT NOT NULL,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE SET NULL
      );

      CREATE TABLE workflow_outer_iterations (
        id TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        iteration INTEGER NOT NULL,
        revision_id TEXT,
        structural_report_json TEXT NOT NULL,
        vlm_report_json TEXT,
        failure_report_json TEXT,
        passed INTEGER NOT NULL CHECK(passed IN (0, 1)),
        created_at TEXT NOT NULL,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE SET NULL,
        UNIQUE(run_id, iteration)
      );

      CREATE TABLE workflow_pending_vlm (
        run_id TEXT PRIMARY KEY,
        artifact_id TEXT NOT NULL,
        contract_json TEXT NOT NULL,
        pass_threshold REAL NOT NULL,
        created_at TEXT NOT NULL,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE
      );

      CREATE INDEX idx_workflow_plans_revision
        ON workflow_plans(revision_id);
      CREATE INDEX idx_workflow_outer_iterations_run_iteration
        ON workflow_outer_iterations(run_id, iteration);
      CREATE INDEX idx_workflow_outer_iterations_revision
        ON workflow_outer_iterations(revision_id);
      CREATE INDEX idx_workflow_pending_vlm_artifact
        ON workflow_pending_vlm(artifact_id);
    "#,
    },
    Migration {
        version: 3,
        name: "milestone_4_0_first_run_and_title_source",
        sql: r#"
      ALTER TABLE sessions
        ADD COLUMN title_source TEXT NOT NULL DEFAULT 'system';

      CREATE TABLE app_kv (
        key TEXT PRIMARY KEY,
        value_json TEXT NOT NULL,
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
      );
    "#,
    },
    Migration {
        version: 4,
        name: "agent_owned_vlm_handoff_context",
        sql: r#"
      ALTER TABLE workflow_pending_vlm
        ADD COLUMN revision_id TEXT REFERENCES revisions(id) ON DELETE SET NULL;
      ALTER TABLE workflow_pending_vlm
        ADD COLUMN structural_report_json TEXT;
      CREATE INDEX idx_workflow_pending_vlm_revision
        ON workflow_pending_vlm(revision_id);
    "#,
    },
];
