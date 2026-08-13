use super::{sha256_hex, StorageResult};
use rusqlite::{params, Connection};

#[cfg(test)]
pub(super) const SCHEMA_VERSION: i64 = 7;

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
        if migration.version == 6 {
            backfill_artifact_revision_hashes(&transaction)?;
        }
        transaction.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![migration.version, migration.name],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

fn backfill_artifact_revision_hashes(transaction: &rusqlite::Transaction<'_>) -> StorageResult<()> {
    let artifacts = {
        let mut statement = transaction.prepare(
            r#"
            SELECT artifacts.id, revisions.source
            FROM artifacts
            INNER JOIN revisions ON revisions.id = artifacts.revision_id
            WHERE artifacts.revision_hash IS NULL OR artifacts.revision_hash = ''
            "#,
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (artifact_id, source) in artifacts {
        let changed = transaction.execute(
            "UPDATE artifacts SET revision_hash = ?1 WHERE id = ?2",
            params![sha256_hex(source.as_bytes()), artifact_id],
        )?;
        if changed != 1 {
            return Err(format!(
                "DFM migration could not backfill artifact revision hash: {artifact_id}"
            )
            .into());
        }
    }
    let missing_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM artifacts WHERE revision_hash IS NULL OR length(revision_hash) <> 64 OR revision_hash GLOB '*[^0-9a-f]*'",
        [],
        |row| row.get(0),
    )?;
    if missing_count != 0 {
        return Err(format!(
            "DFM migration left {missing_count} artifacts without a valid revision hash"
        )
        .into());
    }
    let mismatched_count: i64 = transaction.query_row(
        r#"
        SELECT COUNT(*)
        FROM artifacts
        LEFT JOIN revisions ON revisions.id = artifacts.revision_id
        WHERE revisions.id IS NULL
        "#,
        [],
        |row| row.get(0),
    )?;
    if mismatched_count != 0 {
        return Err(format!(
            "DFM migration found {mismatched_count} artifacts without a matching revision"
        )
        .into());
    }
    transaction.execute_batch(
        r#"
        CREATE TRIGGER artifacts_hashes_insert
        BEFORE INSERT ON artifacts
        WHEN NEW.revision_hash IS NULL
          OR length(NEW.revision_hash) <> 64
          OR NEW.revision_hash GLOB '*[^0-9a-f]*'
          OR (NEW.profile_hash IS NOT NULL AND (
            length(NEW.profile_hash) <> 64 OR NEW.profile_hash GLOB '*[^0-9a-f]*'
          ))
        BEGIN
          SELECT RAISE(ABORT, 'artifact revision/profile hash is invalid');
        END;

        CREATE TRIGGER artifacts_hashes_update
        BEFORE UPDATE OF revision_hash, profile_hash ON artifacts
        WHEN NEW.revision_hash IS NULL
          OR length(NEW.revision_hash) <> 64
          OR NEW.revision_hash GLOB '*[^0-9a-f]*'
          OR (NEW.profile_hash IS NOT NULL AND (
            length(NEW.profile_hash) <> 64 OR NEW.profile_hash GLOB '*[^0-9a-f]*'
          ))
        BEGIN
          SELECT RAISE(ABORT, 'artifact revision/profile hash is invalid');
        END;
        "#,
    )?;
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
    Migration {
        version: 5,
        name: "persistent_agent_thread_graph",
        sql: r#"
      CREATE TABLE agent_threads (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        external_agent TEXT NOT NULL,
        external_thread_id TEXT NOT NULL,
        status TEXT NOT NULL,
        connection_generation INTEGER,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        last_resumed_at TEXT,
        archived_at TEXT,
        replaced_by_id TEXT,
        metadata_json TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(replaced_by_id) REFERENCES agent_threads(id) ON DELETE SET NULL
      );

      ALTER TABLE agent_runs
        ADD COLUMN agent_thread_id TEXT REFERENCES agent_threads(id) ON DELETE SET NULL;
      ALTER TABLE agent_runs
        ADD COLUMN connection_generation INTEGER;
      ALTER TABLE agent_runs
        ADD COLUMN recovery_status TEXT NOT NULL DEFAULT 'none';

      ALTER TABLE conversation_messages ADD COLUMN external_thread_id TEXT;
      ALTER TABLE conversation_messages ADD COLUMN external_turn_id TEXT;
      ALTER TABLE conversation_messages ADD COLUMN external_item_id TEXT;
      ALTER TABLE conversation_messages ADD COLUMN phase TEXT;
      ALTER TABLE conversation_messages ADD COLUMN sequence INTEGER;
      ALTER TABLE conversation_messages
        ADD COLUMN is_final INTEGER NOT NULL DEFAULT 1 CHECK(is_final IN (0, 1));

      CREATE TABLE agent_transport_events (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        run_id TEXT,
        agent_thread_id TEXT,
        external_turn_id TEXT,
        external_item_id TEXT,
        method TEXT NOT NULL,
        sequence INTEGER NOT NULL,
        payload_json TEXT NOT NULL,
        created_at TEXT NOT NULL,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE SET NULL
      );

      CREATE TEMP TABLE legacy_thread_candidates AS
      WITH grouped AS (
        SELECT
          session_id,
          external_agent,
          external_thread_id,
          MIN(id) AS representative_run_id,
          MAX(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS has_completed,
          MAX(CASE WHEN status = 'completed' THEN updated_at ELSE '' END) AS completed_updated_at,
          MAX(updated_at) AS latest_updated_at,
          MIN(created_at) AS created_at
        FROM agent_runs
        WHERE external_agent IS NOT NULL AND external_agent <> ''
          AND external_thread_id IS NOT NULL AND external_thread_id <> ''
        GROUP BY session_id, external_agent, external_thread_id
      ), external_ranked AS (
        SELECT grouped.*,
          ROW_NUMBER() OVER (
            PARTITION BY external_agent, external_thread_id
            ORDER BY has_completed DESC, completed_updated_at DESC,
                     latest_updated_at DESC, session_id DESC
          ) AS external_rank
        FROM grouped
      ), session_ranked AS (
        SELECT external_ranked.*,
          ROW_NUMBER() OVER (
            PARTITION BY session_id, external_agent
            ORDER BY has_completed DESC, completed_updated_at DESC,
                     latest_updated_at DESC, external_thread_id DESC
          ) AS session_rank
        FROM external_ranked
        WHERE external_rank = 1
      )
      SELECT
        'legacy-thread:' || representative_run_id AS id,
        session_id,
        external_agent,
        external_thread_id,
        has_completed,
        latest_updated_at,
        created_at,
        session_rank
      FROM session_ranked;

      INSERT INTO agent_threads (
        id, session_id, external_agent, external_thread_id, status,
        created_at, updated_at, archived_at, metadata_json
      )
      SELECT
        id, session_id, external_agent, external_thread_id,
        CASE WHEN session_rank = 1 THEN 'ready' ELSE 'legacy' END,
        created_at, latest_updated_at,
        CASE WHEN session_rank = 1 THEN NULL ELSE latest_updated_at END,
        '{"migrationStatus":"legacy-backfill"}'
      FROM legacy_thread_candidates;

      UPDATE agent_threads AS legacy
      SET replaced_by_id = (
        SELECT active.id
        FROM legacy_thread_candidates AS active
        WHERE active.session_id = legacy.session_id
          AND active.external_agent = legacy.external_agent
          AND active.session_rank = 1
      )
      WHERE legacy.id IN (
        SELECT id FROM legacy_thread_candidates WHERE session_rank <> 1
      );

      UPDATE agent_runs
      SET agent_thread_id = (
        SELECT candidate.id
        FROM legacy_thread_candidates AS candidate
        WHERE candidate.session_id = agent_runs.session_id
          AND candidate.external_agent = agent_runs.external_agent
          AND candidate.external_thread_id = agent_runs.external_thread_id
      )
      WHERE external_thread_id IS NOT NULL AND external_thread_id <> '';

      UPDATE agent_runs
      SET metadata_json = CASE
        WHEN json_valid(metadata_json) THEN
          json_set(metadata_json, '$.migrationStatus', 'unmapped')
        ELSE json_object(
          'migrationStatus', 'unmapped',
          'legacyMetadataJson', metadata_json
        )
      END
      WHERE external_thread_id IS NOT NULL AND external_thread_id <> ''
        AND agent_thread_id IS NULL;

      UPDATE conversation_messages
      SET external_thread_id = (
            SELECT external_thread_id FROM agent_runs
            WHERE agent_runs.id = conversation_messages.run_id
          ),
          external_turn_id = (
            SELECT external_turn_id FROM agent_runs
            WHERE agent_runs.id = conversation_messages.run_id
          )
      WHERE run_id IS NOT NULL;

      UPDATE conversation_messages
      SET metadata_json = CASE
        WHEN json_valid(metadata_json) THEN
          json_set(metadata_json, '$.migrationStatus', 'unmapped')
        ELSE json_object(
          'migrationStatus', 'unmapped',
          'legacyMetadataJson', metadata_json
        )
      END
      WHERE run_id IS NOT NULL
        AND (
          external_thread_id IS NULL OR external_turn_id IS NULL
          OR NOT EXISTS (
            SELECT 1 FROM agent_runs
            WHERE agent_runs.id = conversation_messages.run_id
              AND agent_runs.agent_thread_id IS NOT NULL
          )
        );

      DROP TABLE legacy_thread_candidates;

      CREATE TRIGGER agent_runs_thread_session_insert
      BEFORE INSERT ON agent_runs
      WHEN NEW.agent_thread_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads
        WHERE id = NEW.agent_thread_id
          AND session_id = NEW.session_id
          AND external_agent IS NEW.external_agent
          AND external_thread_id IS NEW.external_thread_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'agent run thread/session/external mapping mismatch');
      END;

      CREATE TRIGGER agent_runs_thread_session_update
      BEFORE UPDATE OF agent_thread_id, session_id, external_agent, external_thread_id
      ON agent_runs
      WHEN NEW.agent_thread_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads
        WHERE id = NEW.agent_thread_id
          AND session_id = NEW.session_id
          AND external_agent IS NEW.external_agent
          AND external_thread_id IS NEW.external_thread_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'agent run thread/session/external mapping mismatch');
      END;

      CREATE TRIGGER conversation_run_session_insert
      BEFORE INSERT ON conversation_messages
      WHEN NEW.run_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_runs
        WHERE id = NEW.run_id AND session_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'conversation run/session mismatch');
      END;

      CREATE TRIGGER conversation_run_session_update
      BEFORE UPDATE OF run_id, session_id ON conversation_messages
      WHEN NEW.run_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_runs
        WHERE id = NEW.run_id AND session_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'conversation run/session mismatch');
      END;

      CREATE TRIGGER agent_transport_graph_insert
      BEFORE INSERT ON agent_transport_events
      WHEN (NEW.run_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM agent_runs
              WHERE id = NEW.run_id AND session_id = NEW.session_id
            ))
        OR (NEW.agent_thread_id IS NOT NULL AND NOT EXISTS (
              SELECT 1 FROM agent_threads
              WHERE id = NEW.agent_thread_id AND session_id = NEW.session_id
            ))
      BEGIN
        SELECT RAISE(ABORT, 'agent transport event graph mismatch');
      END;

      CREATE UNIQUE INDEX agent_threads_external_id_uq
        ON agent_threads(external_agent, external_thread_id);
      CREATE UNIQUE INDEX agent_threads_active_session_agent_uq
        ON agent_threads(session_id, external_agent)
        WHERE archived_at IS NULL AND replaced_by_id IS NULL;
      CREATE INDEX idx_agent_threads_session_updated_at
        ON agent_threads(session_id, updated_at);
      CREATE INDEX idx_agent_runs_agent_thread
        ON agent_runs(agent_thread_id);
      CREATE INDEX idx_agent_runs_external_turn
        ON agent_runs(external_thread_id, external_turn_id);
      CREATE UNIQUE INDEX conversation_external_item_uq
        ON conversation_messages(external_thread_id, external_turn_id, external_item_id)
        WHERE external_item_id IS NOT NULL;
      CREATE INDEX idx_conversation_run_sequence
        ON conversation_messages(run_id, sequence);
      CREATE UNIQUE INDEX agent_transport_events_run_sequence_uq
        ON agent_transport_events(run_id, sequence)
        WHERE run_id IS NOT NULL;
      CREATE INDEX idx_agent_transport_events_thread_turn
        ON agent_transport_events(agent_thread_id, external_turn_id, sequence);
    "#,
    },
    Migration {
        version: 6,
        name: "dfm_artifact_lineage_and_workflow_reports",
        sql: r#"
      ALTER TABLE artifacts ADD COLUMN revision_hash TEXT;
      ALTER TABLE artifacts ADD COLUMN profile_hash TEXT;
      ALTER TABLE workflow_outer_iterations ADD COLUMN dfm_report_json TEXT;
      ALTER TABLE workflow_pending_vlm ADD COLUMN dfm_report_json TEXT;
      CREATE INDEX idx_artifacts_revision_hash ON artifacts(revision_hash);
      CREATE INDEX idx_artifacts_profile_hash ON artifacts(profile_hash);
    "#,
    },
    Migration {
        version: 7,
        name: "separate_validation_plane_persistence",
        sql: r#"
      ALTER TABLE agent_threads
        ADD COLUMN plane TEXT NOT NULL DEFAULT 'modeling'
          CHECK(plane IN ('modeling', 'validation'));
      ALTER TABLE agent_threads ADD COLUMN owner_id TEXT;
      UPDATE agent_threads SET plane = 'modeling', owner_id = session_id;

      DROP INDEX agent_threads_active_session_agent_uq;
      CREATE UNIQUE INDEX agent_threads_active_session_agent_plane_uq
        ON agent_threads(session_id, external_agent, plane)
        WHERE archived_at IS NULL AND replaced_by_id IS NULL;

      DROP TRIGGER agent_runs_thread_session_insert;
      DROP TRIGGER agent_runs_thread_session_update;

      CREATE TRIGGER agent_threads_scope_insert
      BEFORE INSERT ON agent_threads
      WHEN NEW.owner_id IS NULL OR trim(NEW.owner_id) = ''
        OR (NEW.plane = 'modeling' AND NEW.owner_id <> NEW.session_id)
      BEGIN
        SELECT RAISE(ABORT, 'agent thread plane/owner scope is invalid');
      END;

      CREATE TRIGGER agent_threads_scope_update
      BEFORE UPDATE OF session_id, plane, owner_id ON agent_threads
      WHEN NEW.owner_id IS NULL OR trim(NEW.owner_id) = ''
        OR (NEW.plane = 'modeling' AND NEW.owner_id <> NEW.session_id)
      BEGIN
        SELECT RAISE(ABORT, 'agent thread plane/owner scope is invalid');
      END;

      CREATE TRIGGER agent_threads_replacement_scope_insert
      BEFORE INSERT ON agent_threads
      WHEN NEW.replaced_by_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads replacement
        WHERE replacement.id = NEW.replaced_by_id
          AND replacement.session_id = NEW.session_id
          AND replacement.external_agent = NEW.external_agent
          AND replacement.plane = NEW.plane
          AND replacement.owner_id = NEW.owner_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'replacement agent thread scope mismatch');
      END;

      CREATE TRIGGER agent_threads_replacement_scope_update
      BEFORE UPDATE OF replaced_by_id, session_id, external_agent, plane, owner_id ON agent_threads
      WHEN NEW.replaced_by_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads replacement
        WHERE replacement.id = NEW.replaced_by_id
          AND replacement.session_id = NEW.session_id
          AND replacement.external_agent = NEW.external_agent
          AND replacement.plane = NEW.plane
          AND replacement.owner_id = NEW.owner_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'replacement agent thread scope mismatch');
      END;

      CREATE TRIGGER agent_runs_thread_session_insert
      BEFORE INSERT ON agent_runs
      WHEN NEW.agent_thread_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads
        WHERE id = NEW.agent_thread_id
          AND session_id = NEW.session_id
          AND external_agent IS NEW.external_agent
          AND external_thread_id IS NEW.external_thread_id
          AND plane = 'modeling'
          AND owner_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'agent run requires matching modeling thread scope');
      END;

      CREATE TRIGGER agent_runs_thread_session_update
      BEFORE UPDATE OF agent_thread_id, session_id, external_agent, external_thread_id
      ON agent_runs
      WHEN NEW.agent_thread_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads
        WHERE id = NEW.agent_thread_id
          AND session_id = NEW.session_id
          AND external_agent IS NEW.external_agent
          AND external_thread_id IS NEW.external_thread_id
          AND plane = 'modeling'
          AND owner_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'agent run requires matching modeling thread scope');
      END;

      CREATE TABLE validation_evaluations (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        run_id TEXT NOT NULL,
        revision_id TEXT NOT NULL,
        artifact_id TEXT NOT NULL,
        kind TEXT NOT NULL CHECK(kind IN ('vlm')),
        attempt INTEGER NOT NULL CHECK(attempt >= 1),
        status TEXT NOT NULL CHECK(status IN ('queued', 'running', 'succeeded', 'failed')),
        evaluator_thread_id TEXT,
        external_turn_id TEXT,
        input_contract_json TEXT NOT NULL CHECK(json_valid(input_contract_json)),
        report_json TEXT CHECK(report_json IS NULL OR json_valid(report_json)),
        passed INTEGER CHECK(passed IS NULL OR passed IN (0, 1)),
        score REAL,
        pass_threshold REAL NOT NULL CHECK(pass_threshold >= 0.0 AND pass_threshold <= 1.0),
        error TEXT,
        created_at TEXT NOT NULL,
        started_at TEXT,
        completed_at TEXT,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(run_id) REFERENCES agent_runs(id) ON DELETE CASCADE,
        FOREIGN KEY(revision_id) REFERENCES revisions(id) ON DELETE CASCADE,
        FOREIGN KEY(artifact_id) REFERENCES artifacts(id) ON DELETE CASCADE,
        FOREIGN KEY(evaluator_thread_id) REFERENCES agent_threads(id),
        UNIQUE(run_id, revision_id, artifact_id, kind, attempt),
        CHECK(
          (status = 'queued'
            AND evaluator_thread_id IS NULL AND external_turn_id IS NULL
            AND started_at IS NULL AND completed_at IS NULL
            AND report_json IS NULL AND passed IS NULL AND score IS NULL AND error IS NULL)
          OR
          (status = 'running'
            AND evaluator_thread_id IS NOT NULL AND external_turn_id IS NOT NULL
            AND trim(external_turn_id) <> ''
            AND started_at IS NOT NULL AND completed_at IS NULL
            AND report_json IS NULL AND passed IS NULL AND score IS NULL AND error IS NULL)
          OR
          (status = 'succeeded'
            AND evaluator_thread_id IS NOT NULL AND external_turn_id IS NOT NULL
            AND trim(external_turn_id) <> ''
            AND started_at IS NOT NULL AND completed_at IS NOT NULL
            AND report_json IS NOT NULL AND passed IS NOT NULL AND score IS NOT NULL
            AND score >= 0.0 AND score <= 1.0 AND error IS NULL
            AND (passed = 0 OR score >= pass_threshold))
          OR
          (status = 'failed'
            AND completed_at IS NOT NULL AND error IS NOT NULL AND trim(error) <> ''
            AND report_json IS NULL AND passed IS NULL AND score IS NULL
            AND (external_turn_id IS NULL OR (evaluator_thread_id IS NOT NULL AND trim(external_turn_id) <> '')))
        )
      );

      CREATE TRIGGER validation_evaluations_graph_insert
      BEFORE INSERT ON validation_evaluations
      WHEN NOT EXISTS (
          SELECT 1 FROM agent_runs
          WHERE id = NEW.run_id AND session_id = NEW.session_id
            AND output_revision_id = NEW.revision_id
        )
        OR NOT EXISTS (
          SELECT 1 FROM revisions
          WHERE id = NEW.revision_id AND session_id = NEW.session_id
        )
        OR NOT EXISTS (
          SELECT 1 FROM artifacts
          WHERE id = NEW.artifact_id AND session_id = NEW.session_id
            AND revision_id = NEW.revision_id AND deleted_at IS NULL
        )
        OR (NEW.evaluator_thread_id IS NOT NULL AND NOT EXISTS (
          SELECT 1 FROM agent_threads
          WHERE id = NEW.evaluator_thread_id AND session_id = NEW.session_id
            AND plane = 'validation' AND owner_id = NEW.id
        ))
      BEGIN
        SELECT RAISE(ABORT, 'validation evaluation graph mismatch');
      END;

      CREATE TRIGGER validation_evaluations_graph_update
      BEFORE UPDATE OF evaluator_thread_id, external_turn_id ON validation_evaluations
      WHEN NEW.evaluator_thread_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM agent_threads
        WHERE id = NEW.evaluator_thread_id AND session_id = NEW.session_id
          AND plane = 'validation' AND owner_id = NEW.id
      )
      BEGIN
        SELECT RAISE(ABORT, 'validation evaluation thread scope mismatch');
      END;

      CREATE TRIGGER validation_evaluations_immutable_update
      BEFORE UPDATE OF session_id, run_id, revision_id, artifact_id, kind, attempt,
                       input_contract_json, pass_threshold, created_at
      ON validation_evaluations
      BEGIN
        SELECT RAISE(ABORT, 'validation evaluation attempt fields are immutable');
      END;

      CREATE TRIGGER validation_evaluations_status_transition
      BEFORE UPDATE OF status ON validation_evaluations
      WHEN NOT (
        (OLD.status = 'queued' AND NEW.status IN ('running', 'failed'))
        OR (OLD.status = 'running' AND NEW.status IN ('succeeded', 'failed'))
      )
      BEGIN
        SELECT RAISE(ABORT, 'invalid validation evaluation status transition');
      END;

      CREATE TRIGGER validation_thread_owner_insert
      BEFORE INSERT ON agent_threads
      WHEN NEW.plane = 'validation' AND NOT EXISTS (
        SELECT 1 FROM validation_evaluations
        WHERE id = NEW.owner_id AND session_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'validation thread owner evaluation mismatch');
      END;

      CREATE TRIGGER validation_thread_owner_update
      BEFORE UPDATE OF session_id, plane, owner_id ON agent_threads
      WHEN NEW.plane = 'validation' AND NOT EXISTS (
        SELECT 1 FROM validation_evaluations
        WHERE id = NEW.owner_id AND session_id = NEW.session_id
      )
      BEGIN
        SELECT RAISE(ABORT, 'validation thread owner evaluation mismatch');
      END;

      CREATE TABLE validation_evaluation_events (
        id TEXT PRIMARY KEY,
        session_id TEXT NOT NULL,
        evaluation_id TEXT NOT NULL,
        evaluator_thread_id TEXT NOT NULL,
        external_turn_id TEXT,
        external_item_id TEXT,
        method TEXT NOT NULL CHECK(trim(method) <> ''),
        sequence INTEGER NOT NULL CHECK(sequence >= 0),
        payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
        created_at TEXT NOT NULL,
        FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE,
        FOREIGN KEY(evaluation_id) REFERENCES validation_evaluations(id) ON DELETE CASCADE,
        FOREIGN KEY(evaluator_thread_id) REFERENCES agent_threads(id),
        UNIQUE(evaluation_id, sequence)
      );

      CREATE TRIGGER validation_evaluation_events_graph_insert
      BEFORE INSERT ON validation_evaluation_events
      WHEN NOT EXISTS (
        SELECT 1 FROM validation_evaluations evaluation
        WHERE evaluation.id = NEW.evaluation_id
          AND evaluation.session_id = NEW.session_id
          AND evaluation.evaluator_thread_id = NEW.evaluator_thread_id
          AND (NEW.external_turn_id IS NULL OR evaluation.external_turn_id = NEW.external_turn_id)
      )
      BEGIN
        SELECT RAISE(ABORT, 'validation evaluation event graph mismatch');
      END;

      CREATE INDEX idx_validation_evaluations_session_created_at
        ON validation_evaluations(session_id, created_at);
      CREATE INDEX idx_validation_evaluations_recovery
        ON validation_evaluations(status, created_at)
        WHERE status IN ('queued', 'running');
      CREATE INDEX idx_validation_evaluation_events_thread_turn
        ON validation_evaluation_events(evaluator_thread_id, external_turn_id, sequence);
    "#,
    },
];
