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
        "agent_runs",
        "agent_run_events",
        "workflow_plans",
        "workflow_outer_iterations",
        "workflow_pending_vlm",
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
    assert_eq!(migration_count, 2);
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
            (2, "milestone_3_0_workflow_state_spine".to_string())
        ]
    );

    let applied_versions = applied_schema_versions(&connection);
    assert_eq!(applied_versions, vec![1, SCHEMA_VERSION]);
    let mut connection = Connection::open(layout.database_path()).unwrap();
    run_migrations(&mut connection).unwrap();
    assert_eq!(applied_schema_versions(&connection), applied_versions);
}

#[test]
fn milestone_3_workflow_migration_upgrades_version_1_database_idempotently() {
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

    assert_eq!(applied_schema_versions(&connection), vec![1, 2]);
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
                "contract_json",
                "pass_threshold",
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
