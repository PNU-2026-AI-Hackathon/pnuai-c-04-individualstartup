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
fn migration_runner_creates_schema_once() {
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
        "agent_threads",
        "agent_runs",
        "agent_run_events",
        "agent_transport_events",
        "workflow_plans",
        "workflow_outer_iterations",
        "workflow_pending_vlm",
        "app_kv",
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

    let session_columns = table_columns(&connection, "sessions");
    assert!(
        session_columns
            .iter()
            .any(|candidate| candidate == "title_source"),
        "missing sessions.title_source"
    );

    let artifact_columns = table_columns(&connection, "artifacts");
    for column in [
        "sha256",
        "bytes",
        "created_at",
        "deleted_at",
        "missing_at",
        "metadata_json",
        "revision_hash",
        "profile_hash",
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
    assert_eq!(migration_count, 6);
    let migrations = connection
        .prepare("SELECT version, name FROM schema_migrations ORDER BY version")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        migrations,
        vec![
            (1, "milestone_2_1_initial_persistence_schema".to_string()),
            (2, "milestone_3_0_workflow_state_spine".to_string()),
            (3, "milestone_4_0_first_run_and_title_source".to_string()),
            (4, "agent_owned_vlm_handoff_context".to_string()),
            (5, "persistent_agent_thread_graph".to_string()),
            (6, "dfm_artifact_lineage_and_workflow_reports".to_string())
        ]
    );

    let applied_versions = applied_schema_versions(&connection);
    assert_eq!(applied_versions, vec![1, 2, 3, 4, 5, SCHEMA_VERSION]);
    let mut connection = Connection::open(layout.database_path()).unwrap();
    run_migrations(&mut connection).unwrap();
    assert_eq!(applied_schema_versions(&connection), applied_versions);
}

#[test]
fn later_migrations_upgrade_version_1_database_idempotently() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
        .execute_batch(
            r#"
                CREATE TABLE schema_migrations (
                  version INTEGER PRIMARY KEY,
                  name TEXT NOT NULL,
                  applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );
                "#,
        )
        .unwrap();
    connection.execute_batch(MIGRATIONS[0].sql).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![MIGRATIONS[0].version, MIGRATIONS[0].name],
        )
        .unwrap();

    run_migrations(&mut connection).unwrap();
    run_migrations(&mut connection).unwrap();

    assert_eq!(applied_schema_versions(&connection), vec![1, 2, 3, 4, 5, 6]);
    for (table, expected_columns) in [
        (
            "workflow_plans",
            vec![
                "run_id",
                "revision_id",
                "plan_json",
                "source_language",
                "created_at",
            ],
        ),
        (
            "workflow_outer_iterations",
            vec![
                "id",
                "run_id",
                "iteration",
                "revision_id",
                "structural_report_json",
                "dfm_report_json",
                "vlm_report_json",
                "failure_report_json",
                "passed",
                "created_at",
            ],
        ),
        (
            "workflow_pending_vlm",
            vec![
                "run_id",
                "artifact_id",
                "revision_id",
                "contract_json",
                "pass_threshold",
                "structural_report_json",
                "dfm_report_json",
                "created_at",
            ],
        ),
    ] {
        let columns = table_columns(&connection, table);
        for expected_column in expected_columns {
            assert!(
                columns.iter().any(|column| column == expected_column),
                "missing {table}.{expected_column}"
            );
        }
    }
    let session_columns = table_columns(&connection, "sessions");
    assert!(
        session_columns
            .iter()
            .any(|column| column == "title_source"),
        "missing sessions.title_source"
    );
    let app_kv_columns = table_columns(&connection, "app_kv");
    for expected_column in ["key", "value_json", "updated_at"] {
        assert!(
            app_kv_columns
                .iter()
                .any(|column| column == expected_column),
            "missing app_kv.{expected_column}"
        );
    }
    for expected_column in [
        "agent_thread_id",
        "connection_generation",
        "recovery_status",
    ] {
        assert!(
            table_columns(&connection, "agent_runs")
                .iter()
                .any(|column| column == expected_column),
            "missing agent_runs.{expected_column}"
        );
    }
    for expected_column in [
        "external_thread_id",
        "external_turn_id",
        "external_item_id",
        "phase",
        "sequence",
        "is_final",
    ] {
        assert!(
            table_columns(&connection, "conversation_messages")
                .iter()
                .any(|column| column == expected_column),
            "missing conversation_messages.{expected_column}"
        );
    }
}

#[test]
fn dfm_migration_backfills_artifact_revision_hash_and_enforces_hash_integrity() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_migrations (
              version INTEGER PRIMARY KEY,
              name TEXT NOT NULL,
              applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
            "#,
        )
        .unwrap();
    connection.execute_batch(MIGRATIONS[0].sql).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![MIGRATIONS[0].version, MIGRATIONS[0].name],
        )
        .unwrap();
    let source = "cube([7, 8, 9]);";
    connection
        .execute(
            r#"
            INSERT INTO sessions (
              id, selected_runtime, status, created_at, updated_at
            ) VALUES ('session-1', 'openscad-wasm', 'idle', '2026-01-01', '2026-01-01')
            "#,
            [],
        )
        .unwrap();
    connection
        .execute(
            r#"
            INSERT INTO revisions (
              id, session_id, source_language, source, created_at
            ) VALUES ('revision-1', 'session-1', 'openscad', ?1, '2026-01-01')
            "#,
            params![source],
        )
        .unwrap();
    connection
        .execute(
            r#"
            INSERT INTO artifacts (
              id, session_id, revision_id, kind, format, relative_path, uri,
              sha256, bytes, created_at
            ) VALUES (
              'artifact-1', 'session-1', 'revision-1', 'stl', 'stl',
              'artifacts/session-1/revision-1/artifact-1.stl',
              'tauri://artifact/artifact-1', ?1, 1, '2026-01-01'
            )
            "#,
            params!["a".repeat(64)],
        )
        .unwrap();

    run_migrations(&mut connection).unwrap();

    let revision_hash: String = connection
        .query_row(
            "SELECT revision_hash FROM artifacts WHERE id = 'artifact-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision_hash, sha256_hex(source.as_bytes()));
    let invalid = connection.execute(
        "UPDATE artifacts SET profile_hash = 'invalid' WHERE id = 'artifact-1'",
        [],
    );
    assert!(invalid
        .expect_err("invalid profile hash must fail fast")
        .to_string()
        .contains("artifact revision/profile hash is invalid"));
}

#[test]
fn legacy_agent_graph_backfill_is_deterministic_and_preserves_unmapped_rows() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE schema_migrations (
              version INTEGER PRIMARY KEY,
              name TEXT NOT NULL,
              applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );
            "#,
        )
        .unwrap();
    for migration in &MIGRATIONS[..4] {
        connection.execute_batch(migration.sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
                params![migration.version, migration.name],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            r#"
            INSERT INTO sessions (
              id, selected_runtime, status, created_at, updated_at, title_source
            ) VALUES
              ('session-1', 'openscad-wasm', 'idle', '2026-01-01T00:00:00Z',
               '2026-01-01T00:00:00Z', 'system');

            INSERT INTO agent_runs (
              id, session_id, status, prompt, created_at, updated_at,
              external_agent, external_thread_id, external_turn_id, metadata_json
            ) VALUES
              ('run-a', 'session-1', 'completed', 'a', '2026-01-01T00:00:00Z',
               '2026-01-02T00:00:00Z', 'codex', 'thread-a', 'turn-a', '{}'),
              ('run-b', 'session-1', 'completed', 'b', '2026-01-01T00:00:00Z',
               '2026-01-03T00:00:00Z', 'codex', 'thread-b', 'turn-b', '{}'),
              ('run-c', 'session-1', 'running', 'c', '2026-01-01T00:00:00Z',
               '2026-01-04T00:00:00Z', 'codex', 'thread-c', 'turn-c', '{}'),
              ('run-unmapped', 'session-1', 'failed', 'u', '2026-01-01T00:00:00Z',
               '2026-01-05T00:00:00Z', NULL, 'thread-without-agent', NULL,
               '{"preserveMe":true}');

            INSERT INTO conversation_messages (
              id, session_id, run_id, role, content, created_at, metadata_json
            ) VALUES
              ('message-mapped', 'session-1', 'run-b', 'assistant', 'done',
               '2026-01-03T00:00:00Z', '{}'),
              ('message-unmapped', 'session-1', 'run-unmapped', 'assistant', 'failed',
               '2026-01-05T00:00:00Z', '{"preserveMe":true}');
            "#,
        )
        .unwrap();

    run_migrations(&mut connection).unwrap();
    run_migrations(&mut connection).unwrap();

    assert_eq!(applied_schema_versions(&connection), vec![1, 2, 3, 4, 5, 6]);
    let active_thread: String = connection
        .query_row(
            r#"
            SELECT external_thread_id FROM agent_threads
            WHERE session_id = 'session-1' AND external_agent = 'codex'
              AND archived_at IS NULL AND replaced_by_id IS NULL
            "#,
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_thread, "thread-b");

    let mapped_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE agent_thread_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mapped_count, 3);
    let replacement_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_threads WHERE replaced_by_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(replacement_count, 2);

    let (thread_id, turn_id): (Option<String>, Option<String>) = connection
        .query_row(
            r#"
            SELECT external_thread_id, external_turn_id
            FROM conversation_messages WHERE id = 'message-mapped'
            "#,
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(thread_id.as_deref(), Some("thread-b"));
    assert_eq!(turn_id.as_deref(), Some("turn-b"));

    let run_metadata: String = connection
        .query_row(
            "SELECT metadata_json FROM agent_runs WHERE id = 'run-unmapped'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let run_metadata: serde_json::Value = serde_json::from_str(&run_metadata).unwrap();
    assert_eq!(run_metadata["migrationStatus"], "unmapped");
    assert_eq!(run_metadata["preserveMe"], true);
    let message_metadata: String = connection
        .query_row(
            "SELECT metadata_json FROM conversation_messages WHERE id = 'message-unmapped'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let message_metadata: serde_json::Value = serde_json::from_str(&message_metadata).unwrap();
    assert_eq!(message_metadata["migrationStatus"], "unmapped");
    assert_eq!(message_metadata["preserveMe"], true);
    let run_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM agent_runs", [], |row| row.get(0))
        .unwrap();
    let message_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM conversation_messages", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!((run_count, message_count), (4, 2));
}

#[test]
fn persistent_agent_graph_constraints_reject_cross_session_links_and_duplicates() {
    let mut connection = Connection::open_in_memory().unwrap();
    run_migrations(&mut connection).unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO sessions (
              id, selected_runtime, status, created_at, updated_at
            ) VALUES
              ('session-1', 'openscad-wasm', 'idle', '2026-01-01', '2026-01-01'),
              ('session-2', 'openscad-wasm', 'idle', '2026-01-01', '2026-01-01');
            INSERT INTO agent_threads (
              id, session_id, external_agent, external_thread_id, status,
              created_at, updated_at
            ) VALUES
              ('thread-row-1', 'session-1', 'codex', 'external-1', 'ready',
               '2026-01-01', '2026-01-01');
            "#,
        )
        .unwrap();

    assert!(connection
        .execute(
            r#"
            INSERT INTO agent_threads (
              id, session_id, external_agent, external_thread_id, status,
              created_at, updated_at
            ) VALUES ('thread-row-2', 'session-2', 'codex', 'external-1', 'ready',
              '2026-01-01', '2026-01-01')
            "#,
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            r#"
            INSERT INTO agent_runs (
              id, session_id, status, prompt, created_at, updated_at,
              external_agent, external_thread_id, agent_thread_id
            ) VALUES ('run-invalid', 'session-2', 'queued', 'prompt',
              '2026-01-01', '2026-01-01', 'codex', 'external-1', 'thread-row-1')
            "#,
            [],
        )
        .is_err());

    connection
        .execute_batch(
            r#"
            INSERT INTO agent_runs (
              id, session_id, status, prompt, created_at, updated_at,
              external_agent, external_thread_id, external_turn_id, agent_thread_id
            ) VALUES ('run-valid', 'session-1', 'queued', 'prompt',
              '2026-01-01', '2026-01-01', 'codex', 'external-1', 'turn-1', 'thread-row-1');
            INSERT INTO conversation_messages (
              id, session_id, run_id, role, content, created_at,
              external_thread_id, external_turn_id, external_item_id
            ) VALUES ('message-1', 'session-1', 'run-valid', 'assistant', 'one',
              '2026-01-01', 'external-1', 'turn-1', 'item-1');
            "#,
        )
        .unwrap();
    assert!(connection
        .execute(
            r#"
            INSERT INTO conversation_messages (
              id, session_id, run_id, role, content, created_at,
              external_thread_id, external_turn_id, external_item_id
            ) VALUES ('message-2', 'session-1', 'run-valid', 'assistant', 'two',
              '2026-01-01', 'external-1', 'turn-1', 'item-1')
            "#,
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            r#"
            INSERT INTO conversation_messages (
              id, session_id, run_id, role, content, created_at
            ) VALUES ('message-cross', 'session-2', 'run-valid', 'assistant', 'bad',
              '2026-01-01')
            "#,
            [],
        )
        .is_err());
}

#[test]
fn workflow_tables_enforce_foreign_keys_and_integrity() {
    let mut connection = Connection::open_in_memory().unwrap();
    run_migrations(&mut connection).unwrap();

    let invalid_plan = connection.execute(
        r#"
            INSERT INTO workflow_plans (
              run_id, revision_id, plan_json, source_language, created_at
            ) VALUES ('missing-run', NULL, '{}', 'openscad', '2026-07-29T00:00:00.000Z')
            "#,
        [],
    );
    assert!(invalid_plan.is_err(), "workflow_plans.run_id must exist");

    connection
        .execute(
            r#"
                INSERT INTO sessions (
                  id, selected_runtime, status, created_at, updated_at
                ) VALUES ('session-1', 'openscad-wasm', 'idle',
                  '2026-07-29T00:00:00.000Z', '2026-07-29T00:00:00.000Z')
                "#,
            [],
        )
        .unwrap();
    connection
        .execute(
            r#"
                INSERT INTO agent_runs (
                  id, session_id, status, prompt, created_at, updated_at
                ) VALUES ('run-1', 'session-1', 'queued', 'prompt',
                  '2026-07-29T00:00:00.000Z', '2026-07-29T00:00:00.000Z')
                "#,
            [],
        )
        .unwrap();

    let invalid_revision = connection.execute(
        r#"
            INSERT INTO workflow_outer_iterations (
              id, run_id, iteration, revision_id, structural_report_json, passed, created_at
            ) VALUES ('outer-1', 'run-1', 1, 'missing-revision', '{}', 0,
              '2026-07-29T00:00:00.000Z')
            "#,
        [],
    );
    assert!(
        invalid_revision.is_err(),
        "workflow_outer_iterations.revision_id must exist"
    );

    let invalid_passed = connection.execute(
        r#"
            INSERT INTO workflow_outer_iterations (
              id, run_id, iteration, structural_report_json, passed, created_at
            ) VALUES ('outer-1', 'run-1', 1, '{}', 2,
              '2026-07-29T00:00:00.000Z')
            "#,
        [],
    );
    assert!(
        invalid_passed.is_err(),
        "workflow_outer_iterations.passed must be boolean-backed"
    );

    let invalid_pending = connection.execute(
        r#"
            INSERT INTO workflow_pending_vlm (
              run_id, artifact_id, contract_json, pass_threshold, created_at
            ) VALUES ('run-1', 'missing-artifact', '{}', 0.8,
              '2026-07-29T00:00:00.000Z')
            "#,
        [],
    );
    assert!(
        invalid_pending.is_err(),
        "workflow_pending_vlm.artifact_id must exist"
    );
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
use super::migrations::{MIGRATIONS, SCHEMA_VERSION};
use super::*;
use rusqlite::{params, Connection};
use std::path::PathBuf;
