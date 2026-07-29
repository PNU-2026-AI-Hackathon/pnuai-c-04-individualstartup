use crate::protocol::*;
use crate::session_service::ServiceState;
use crate::storage::{self, StorageLayout};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub(crate) type SessionRepositoryResult<T> = Result<T, String>;

#[derive(Default)]
pub(crate) struct SessionRepositorySnapshot {
    pub sessions: HashMap<String, CadSession>,
    pub revisions: HashMap<String, CadRevision>,
    pub artifacts: HashMap<String, CadArtifact>,
    pub conversation: HashMap<String, Vec<CadConversationMessage>>,
    pub agent_runs: HashMap<String, Vec<CadAgentRun>>,
    pub agent_run_events: HashMap<String, Vec<CadAgentRunEvent>>,
    pub workflow_plans: HashMap<String, CadWorkflowPlan>,
    pub workflow_outer_iterations: HashMap<String, Vec<CadWorkflowOuterIteration>>,
    pub workflow_pending_vlm: HashMap<String, CadWorkflowPendingVlm>,
    pub current_interactive_session_id: Option<String>,
}

pub(crate) trait SessionRepository: Send + Sync {
    fn load(&self) -> SessionRepositoryResult<SessionRepositorySnapshot>;
    fn save_session_graph(
        &self,
        state: &ServiceState,
        session_id: &str,
    ) -> SessionRepositoryResult<()>;
    fn save_artifact_manifest(
        &self,
        session_id: &str,
        artifact: &CadArtifact,
    ) -> SessionRepositoryResult<()>;
    fn save_conversation_message(
        &self,
        message: &CadConversationMessage,
    ) -> SessionRepositoryResult<()>;
    fn save_agent_run(&self, run: &CadAgentRun) -> SessionRepositoryResult<()>;
    fn save_agent_run_event(
        &self,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<CadAgentRunEvent>;
    fn save_workflow_plan(&self, plan: &CadWorkflowPlan) -> SessionRepositoryResult<()>;
    fn save_workflow_outer_iteration(
        &self,
        iteration: &CadWorkflowOuterIteration,
    ) -> SessionRepositoryResult<()>;
    fn save_workflow_pending_vlm(
        &self,
        pending_vlm: &CadWorkflowPendingVlm,
    ) -> SessionRepositoryResult<()>;
    fn clear_workflow_pending_vlm(&self, run_id: &str) -> SessionRepositoryResult<()>;
    fn load_artifact_manifest(
        &self,
        artifact_id: &str,
    ) -> SessionRepositoryResult<Option<CadArtifact>>;
    fn set_artifact_missing_at(
        &self,
        artifact_id: &str,
        missing_at: Option<&str>,
    ) -> SessionRepositoryResult<()>;
    fn mark_artifact_deleted(
        &self,
        artifact_id: &str,
        deleted_at: &str,
    ) -> SessionRepositoryResult<()>;
    fn delete_session(&self, session_id: &str, deleted_at: &str) -> SessionRepositoryResult<()>;
}

#[cfg(test)]
pub(crate) struct InMemorySessionRepository;

#[cfg(test)]
impl SessionRepository for InMemorySessionRepository {
    fn load(&self) -> SessionRepositoryResult<SessionRepositorySnapshot> {
        Ok(SessionRepositorySnapshot::default())
    }

    fn save_session_graph(
        &self,
        _state: &ServiceState,
        _session_id: &str,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_artifact_manifest(
        &self,
        _session_id: &str,
        _artifact: &CadArtifact,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_conversation_message(
        &self,
        _message: &CadConversationMessage,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_agent_run(&self, _run: &CadAgentRun) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_agent_run_event(
        &self,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<CadAgentRunEvent> {
        Ok(event.clone())
    }

    fn save_workflow_plan(&self, _plan: &CadWorkflowPlan) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_workflow_outer_iteration(
        &self,
        _iteration: &CadWorkflowOuterIteration,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_workflow_pending_vlm(
        &self,
        _pending_vlm: &CadWorkflowPendingVlm,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn clear_workflow_pending_vlm(&self, _run_id: &str) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn load_artifact_manifest(
        &self,
        _artifact_id: &str,
    ) -> SessionRepositoryResult<Option<CadArtifact>> {
        Ok(None)
    }

    fn set_artifact_missing_at(
        &self,
        _artifact_id: &str,
        _missing_at: Option<&str>,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn mark_artifact_deleted(
        &self,
        _artifact_id: &str,
        _deleted_at: &str,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn delete_session(&self, _session_id: &str, _deleted_at: &str) -> SessionRepositoryResult<()> {
        Ok(())
    }
}

pub(crate) struct SqliteSessionRepository {
    layout: StorageLayout,
}

impl SqliteSessionRepository {
    pub fn new(layout: StorageLayout) -> Self {
        Self { layout }
    }

    fn connection(&self) -> SessionRepositoryResult<Connection> {
        let mut connection =
            Connection::open(self.layout.database_path()).map_err(|error| error.to_string())?;
        storage::run_migrations(&mut connection).map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| error.to_string())?;
        Ok(connection)
    }
}

impl SessionRepository for SqliteSessionRepository {
    fn load(&self) -> SessionRepositoryResult<SessionRepositorySnapshot> {
        let connection = self.connection()?;
        let mut sessions = load_sessions(&connection)?;
        let mut revisions = load_revisions(&connection)?;
        let artifacts = load_artifacts(&connection, &self.layout, None)?;
        let conversation = load_conversation_messages(&connection)?;
        let agent_runs = load_agent_runs(&connection)?;
        let agent_run_events = load_agent_run_events(&connection)?;
        let workflow_plans = load_workflow_plans(&connection)?;
        let workflow_outer_iterations = load_workflow_outer_iterations(&connection)?;
        let workflow_pending_vlm = load_workflow_pending_vlm(&connection)?;
        attach_artifacts_to_revisions(&mut revisions, &artifacts);
        for session_id in sessions.keys().cloned().collect::<Vec<_>>() {
            rebuild_loaded_revision_summaries(&mut sessions, &revisions, &agent_runs, &session_id);
        }
        let current_interactive_session_id = load_current_session_id(&connection)?;
        Ok(SessionRepositorySnapshot {
            sessions,
            revisions,
            artifacts,
            conversation,
            agent_runs,
            agent_run_events,
            workflow_plans,
            workflow_outer_iterations,
            workflow_pending_vlm,
            current_interactive_session_id,
        })
    }

    fn save_session_graph(
        &self,
        state: &ServiceState,
        session_id: &str,
    ) -> SessionRepositoryResult<()> {
        let session = state
            .sessions
            .get(session_id)
            .ok_or_else(|| format!("CAD session not found: {session_id}"))?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                r#"
                INSERT INTO sessions (
                  id, title, selected_runtime, status, active_revision_id,
                  created_at, updated_at, last_viewed_at, connected_ui_clients,
                  archived_at, deleted_at, metadata_json
                ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
                ON CONFLICT(id) DO UPDATE SET
                  title = excluded.title,
                  selected_runtime = excluded.selected_runtime,
                  status = excluded.status,
                  active_revision_id = NULL,
                  created_at = excluded.created_at,
                  updated_at = excluded.updated_at,
                  last_viewed_at = excluded.last_viewed_at,
                  connected_ui_clients = excluded.connected_ui_clients,
                  archived_at = excluded.archived_at,
                  deleted_at = excluded.deleted_at,
                  metadata_json = excluded.metadata_json
                "#,
                params![
                    session.id,
                    session.title,
                    to_db_text(&session.selected_runtime)?,
                    to_db_text(&session.status)?,
                    session.created_at,
                    session.updated_at,
                    session.last_viewed_at,
                    i64::from(session.connected_ui_clients),
                    session.archived_at,
                    session.deleted_at,
                ],
            )
            .map_err(|error| error.to_string())?;

        for revision in state
            .revisions
            .values()
            .filter(|revision| revision.session_id == session_id)
        {
            transaction
                .execute(
                    r#"
                    INSERT INTO revisions (
                      id, session_id, parent_revision_id, restored_from_revision_id,
                      source_language, source, parameters_json, diagnostics_json,
                      created_at, metadata_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                    ON CONFLICT(id) DO UPDATE SET
                      session_id = excluded.session_id,
                      parent_revision_id = excluded.parent_revision_id,
                      restored_from_revision_id = excluded.restored_from_revision_id,
                      source_language = excluded.source_language,
                      source = excluded.source,
                      parameters_json = excluded.parameters_json,
                      diagnostics_json = excluded.diagnostics_json,
                      created_at = excluded.created_at,
                      metadata_json = excluded.metadata_json
                    "#,
                    params![
                        revision.id,
                        revision.session_id,
                        revision.parent_revision_id,
                        revision.restored_from_revision_id,
                        to_db_text(&revision.source_language)?,
                        revision.source,
                        serde_json::to_string(&revision.parameters)
                            .map_err(|error| error.to_string())?,
                        serde_json::to_string(&revision.diagnostics)
                            .map_err(|error| error.to_string())?,
                        revision.created_at,
                        serde_json::to_string(&json!({ "userEvents": revision.user_events }))
                            .map_err(|error| error.to_string())?,
                    ],
                )
                .map_err(|error| error.to_string())?;
        }

        transaction
            .execute(
                "UPDATE sessions SET active_revision_id = ?1 WHERE id = ?2",
                params![session.active_revision_id, session.id],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    fn save_artifact_manifest(
        &self,
        session_id: &str,
        artifact: &CadArtifact,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_artifact_manifest(&connection, session_id, artifact)
    }

    fn save_conversation_message(
        &self,
        message: &CadConversationMessage,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_conversation_message(&connection, message)
    }

    fn save_agent_run(&self, run: &CadAgentRun) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_agent_run(&connection, run)
    }

    fn save_agent_run_event(
        &self,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<CadAgentRunEvent> {
        let mut connection = self.connection()?;
        save_agent_run_event(&mut connection, event)
    }

    fn save_workflow_plan(&self, plan: &CadWorkflowPlan) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_workflow_plan(&connection, plan)
    }

    fn save_workflow_outer_iteration(
        &self,
        iteration: &CadWorkflowOuterIteration,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_workflow_outer_iteration(&connection, iteration)
    }

    fn save_workflow_pending_vlm(
        &self,
        pending_vlm: &CadWorkflowPendingVlm,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_workflow_pending_vlm(&connection, pending_vlm)
    }

    fn clear_workflow_pending_vlm(&self, run_id: &str) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        connection
            .execute(
                "DELETE FROM workflow_pending_vlm WHERE run_id = ?1",
                params![run_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn load_artifact_manifest(
        &self,
        artifact_id: &str,
    ) -> SessionRepositoryResult<Option<CadArtifact>> {
        let connection = self.connection()?;
        load_artifacts(&connection, &self.layout, Some(artifact_id))
            .map(|mut artifacts| artifacts.remove(artifact_id))
    }

    fn set_artifact_missing_at(
        &self,
        artifact_id: &str,
        missing_at: Option<&str>,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE artifacts SET missing_at = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![missing_at, artifact_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn mark_artifact_deleted(
        &self,
        artifact_id: &str,
        deleted_at: &str,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE artifacts SET deleted_at = ?1 WHERE id = ?2",
                params![deleted_at, artifact_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn delete_session(&self, session_id: &str, deleted_at: &str) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE sessions SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![deleted_at, session_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn load_sessions(connection: &Connection) -> SessionRepositoryResult<HashMap<String, CadSession>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, title, selected_runtime, status, active_revision_id,
                   created_at, updated_at, last_viewed_at, connected_ui_clients,
                   archived_at, deleted_at
            FROM sessions
            WHERE deleted_at IS NULL
            ORDER BY updated_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let selected_runtime: String = row.get(2)?;
            let status: String = row.get(3)?;
            let (selected_runtime, runtime_diagnostic) = recover_runtime_kind(&selected_runtime);
            Ok(CadSession {
                id: row.get(0)?,
                title: row.get(1)?,
                selected_runtime,
                status: from_db_text(&status).map_err(to_rusqlite_error)?,
                recovery_diagnostics: runtime_diagnostic.into_iter().collect(),
                active_revision_id: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                last_viewed_at: row.get(7)?,
                connected_ui_clients: row.get::<_, i64>(8)?.max(0) as u32,
                archived_at: row.get(9)?,
                deleted_at: row.get(10)?,
                revisions: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?;
    let mut sessions = HashMap::new();
    for row in rows {
        let session = row.map_err(|error| error.to_string())?;
        sessions.insert(session.id.clone(), session);
    }
    Ok(sessions)
}

fn load_revisions(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadRevision>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT revisions.id, revisions.session_id, revisions.parent_revision_id,
                   revisions.restored_from_revision_id, revisions.source_language,
                   revisions.source, revisions.parameters_json, revisions.diagnostics_json,
                   revisions.created_at, revisions.metadata_json
            FROM revisions
            INNER JOIN sessions ON sessions.id = revisions.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY revisions.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let source: String = row.get(5)?;
            let source_language: String = row.get(4)?;
            let parameters_json: String = row.get(6)?;
            let diagnostics_json: String = row.get(7)?;
            let metadata_json: Option<String> = row.get(9)?;
            let (source_language, source_language_diagnostic) =
                recover_source_language(&source_language);
            let mut diagnostics = recover_diagnostics(&diagnostics_json);
            if let Some(diagnostic) = source_language_diagnostic {
                diagnostics.ok = false;
                diagnostics.items.push(diagnostic);
            }
            Ok(CadRevision {
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: revision_user_events(metadata_json).map_err(to_rusqlite_error)?,
                run_links: Vec::new(),
                id,
                session_id: row.get(1)?,
                parent_revision_id: row.get(2)?,
                restored_from_revision_id: row.get(3)?,
                source_hash: storage::sha256_hex(source.as_bytes()),
                source_language,
                source,
                parameters: serde_json::from_str(&parameters_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                diagnostics,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut revisions = HashMap::new();
    for row in rows {
        let revision = row.map_err(|error| error.to_string())?;
        revisions.insert(revision.id.clone(), revision);
    }
    Ok(revisions)
}

fn load_artifacts(
    connection: &Connection,
    layout: &StorageLayout,
    artifact_id: Option<&str>,
) -> SessionRepositoryResult<HashMap<String, CadArtifact>> {
    let sql = if artifact_id.is_some() {
        r#"
        SELECT artifacts.id, artifacts.revision_id, artifacts.kind, artifacts.format,
               artifacts.relative_path, artifacts.uri, artifacts.sha256, artifacts.bytes,
               artifacts.created_at, artifacts.deleted_at, artifacts.missing_at,
               artifacts.metadata_json
        FROM artifacts
        INNER JOIN sessions ON sessions.id = artifacts.session_id
        WHERE sessions.deleted_at IS NULL AND artifacts.id = ?1
        "#
    } else {
        r#"
        SELECT artifacts.id, artifacts.revision_id, artifacts.kind, artifacts.format,
               artifacts.relative_path, artifacts.uri, artifacts.sha256, artifacts.bytes,
               artifacts.created_at, artifacts.deleted_at, artifacts.missing_at,
               artifacts.metadata_json
        FROM artifacts
        INNER JOIN sessions ON sessions.id = artifacts.session_id
        WHERE sessions.deleted_at IS NULL AND artifacts.deleted_at IS NULL
        ORDER BY artifacts.created_at ASC
        "#
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<CadArtifact> {
        let kind: String = row.get(2)?;
        let relative_path: String = row.get(4)?;
        let sha256: String = row.get(6)?;
        let metadata_json: Option<String> = row.get(11)?;
        let mut metadata = artifact_metadata(metadata_json).map_err(to_rusqlite_error)?;
        metadata.insert(
            "relativePath".to_string(),
            Value::String(relative_path.clone()),
        );
        metadata.insert("sha256".to_string(), Value::String(sha256));
        metadata.insert(
            "path".to_string(),
            Value::String(
                layout
                    .app_data_dir()
                    .join(&relative_path)
                    .to_string_lossy()
                    .to_string(),
            ),
        );
        Ok(CadArtifact {
            id: row.get(0)?,
            revision_id: row.get(1)?,
            kind: from_db_text(&kind).map_err(to_rusqlite_error)?,
            format: row.get(3)?,
            uri: row.get(5)?,
            bytes: Some(row.get::<_, i64>(7)?.max(0) as u64),
            created_at: row.get(8)?,
            deleted_at: row.get(9)?,
            missing_at: row.get(10)?,
            metadata: Some(metadata),
        })
    };
    let rows = if let Some(artifact_id) = artifact_id {
        statement
            .query_map(params![artifact_id], map_row)
            .map_err(|error| error.to_string())?
    } else {
        statement
            .query_map([], map_row)
            .map_err(|error| error.to_string())?
    };
    let mut artifacts = HashMap::new();
    for row in rows {
        let artifact = row.map_err(|error| error.to_string())?;
        artifacts.insert(artifact.id.clone(), artifact);
    }
    Ok(artifacts)
}

fn load_conversation_messages(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadConversationMessage>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT conversation_messages.id, conversation_messages.session_id,
                   conversation_messages.revision_id, conversation_messages.run_id,
                   conversation_messages.role, conversation_messages.content,
                   conversation_messages.created_at, conversation_messages.metadata_json
            FROM conversation_messages
            INNER JOIN sessions ON sessions.id = conversation_messages.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY conversation_messages.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let role: String = row.get(4)?;
            let metadata_json: Option<String> = row.get(7)?;
            Ok(CadConversationMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                revision_id: row.get(2)?,
                run_id: row.get(3)?,
                role: from_db_text(&role).map_err(to_rusqlite_error)?,
                content: row.get(5)?,
                created_at: row.get(6)?,
                metadata: optional_metadata(metadata_json).map_err(to_rusqlite_error)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut messages: HashMap<String, Vec<CadConversationMessage>> = HashMap::new();
    for row in rows {
        let message = row.map_err(|error| error.to_string())?;
        messages
            .entry(message.session_id.clone())
            .or_default()
            .push(message);
    }
    Ok(messages)
}

fn load_agent_runs(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadAgentRun>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT agent_runs.id, agent_runs.session_id, agent_runs.input_revision_id,
                   agent_runs.output_revision_id, agent_runs.status, agent_runs.prompt,
                   agent_runs.created_at, agent_runs.updated_at, agent_runs.started_at,
                   agent_runs.completed_at, agent_runs.error, agent_runs.active_step,
                   agent_runs.external_agent, agent_runs.external_thread_id,
                   agent_runs.external_turn_id
            FROM agent_runs
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY agent_runs.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let status: String = row.get(4)?;
            Ok(CadAgentRun {
                id: row.get(0)?,
                session_id: row.get(1)?,
                input_revision_id: row.get(2)?,
                output_revision_id: row.get(3)?,
                status: from_db_text(&status).map_err(to_rusqlite_error)?,
                prompt: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                started_at: row.get(8)?,
                completed_at: row.get(9)?,
                error: row.get(10)?,
                active_step: row.get(11)?,
                external_agent: row.get(12)?,
                external_thread_id: row.get(13)?,
                external_turn_id: row.get(14)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut runs: HashMap<String, Vec<CadAgentRun>> = HashMap::new();
    for row in rows {
        let run = row.map_err(|error| error.to_string())?;
        runs.entry(run.session_id.clone()).or_default().push(run);
    }
    Ok(runs)
}

fn load_agent_run_events(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadAgentRunEvent>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT agent_run_events.id, agent_run_events.session_id,
                   agent_run_events.run_id, agent_run_events.revision_id,
                   agent_run_events.event_type, agent_run_events.sequence,
                   agent_run_events.created_at, agent_run_events.payload_json,
                   agent_run_events.metadata_json
            FROM agent_run_events
            INNER JOIN sessions ON sessions.id = agent_run_events.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY agent_run_events.run_id ASC, agent_run_events.sequence ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let event_type: String = row.get(4)?;
            let payload_json: String = row.get(7)?;
            let metadata_json: Option<String> = row.get(8)?;
            Ok(CadAgentRunEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                run_id: row.get(2)?,
                revision_id: row.get(3)?,
                event_type: from_db_text(&event_type).map_err(to_rusqlite_error)?,
                sequence: row.get::<_, i64>(5)?.max(0) as u64,
                created_at: row.get(6)?,
                payload: serde_json::from_str(&payload_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                metadata: optional_metadata(metadata_json).map_err(to_rusqlite_error)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut events: HashMap<String, Vec<CadAgentRunEvent>> = HashMap::new();
    for row in rows {
        let event = row.map_err(|error| error.to_string())?;
        events
            .entry(event.session_id.clone())
            .or_default()
            .push(event);
    }
    Ok(events)
}

fn load_workflow_plans(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadWorkflowPlan>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_plans.run_id, workflow_plans.revision_id,
                   workflow_plans.plan_json, workflow_plans.source_language,
                   workflow_plans.created_at
            FROM workflow_plans
            INNER JOIN agent_runs ON agent_runs.id = workflow_plans.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_plans.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let plan_json: String = row.get(2)?;
            let source_language: String = row.get(3)?;
            Ok(CadWorkflowPlan {
                run_id: row.get(0)?,
                revision_id: row.get(1)?,
                plan: serde_json::from_str(&plan_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                source_language: from_db_text(&source_language).map_err(to_rusqlite_error)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut plans = HashMap::new();
    for row in rows {
        let plan = row.map_err(|error| error.to_string())?;
        plans.insert(plan.run_id.clone(), plan);
    }
    Ok(plans)
}

fn load_workflow_outer_iterations(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadWorkflowOuterIteration>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_outer_iterations.id, workflow_outer_iterations.run_id,
                   workflow_outer_iterations.iteration,
                   workflow_outer_iterations.revision_id,
                   workflow_outer_iterations.structural_report_json,
                   workflow_outer_iterations.vlm_report_json,
                   workflow_outer_iterations.failure_report_json,
                   workflow_outer_iterations.passed,
                   workflow_outer_iterations.created_at
            FROM workflow_outer_iterations
            INNER JOIN agent_runs ON agent_runs.id = workflow_outer_iterations.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_outer_iterations.run_id ASC,
                     workflow_outer_iterations.iteration ASC,
                     workflow_outer_iterations.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let structural_report_json: String = row.get(4)?;
            let vlm_report_json: Option<String> = row.get(5)?;
            let failure_report_json: Option<String> = row.get(6)?;
            Ok(CadWorkflowOuterIteration {
                id: row.get(0)?,
                run_id: row.get(1)?,
                iteration: row.get::<_, i64>(2)?.max(0) as u64,
                revision_id: row.get(3)?,
                structural_report: serde_json::from_str(&structural_report_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                vlm_report: optional_json_value(vlm_report_json).map_err(to_rusqlite_error)?,
                failure_report: optional_json_value(failure_report_json)
                    .map_err(to_rusqlite_error)?,
                passed: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut iterations: HashMap<String, Vec<CadWorkflowOuterIteration>> = HashMap::new();
    for row in rows {
        let iteration = row.map_err(|error| error.to_string())?;
        iterations
            .entry(iteration.run_id.clone())
            .or_default()
            .push(iteration);
    }
    Ok(iterations)
}

fn load_workflow_pending_vlm(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadWorkflowPendingVlm>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_pending_vlm.run_id, workflow_pending_vlm.artifact_id,
                   workflow_pending_vlm.contract_json,
                   workflow_pending_vlm.pass_threshold,
                   workflow_pending_vlm.created_at
            FROM workflow_pending_vlm
            INNER JOIN agent_runs ON agent_runs.id = workflow_pending_vlm.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_pending_vlm.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let contract_json: String = row.get(2)?;
            Ok(CadWorkflowPendingVlm {
                run_id: row.get(0)?,
                artifact_id: row.get(1)?,
                contract: serde_json::from_str(&contract_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                pass_threshold: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut pending = HashMap::new();
    for row in rows {
        let pending_vlm = row.map_err(|error| error.to_string())?;
        pending.insert(pending_vlm.run_id.clone(), pending_vlm);
    }
    Ok(pending)
}

fn attach_artifacts_to_revisions(
    revisions: &mut HashMap<String, CadRevision>,
    artifacts: &HashMap<String, CadArtifact>,
) {
    for artifact in artifacts
        .values()
        .filter(|artifact| artifact.deleted_at.is_none())
    {
        if let Some(revision) = revisions.get_mut(&artifact.revision_id) {
            revision.artifacts.push(artifact.clone());
        }
    }
    for revision in revisions.values_mut() {
        revision
            .artifacts
            .sort_by(|left, right| left.created_at.cmp(&right.created_at));
        revision.artifact_count = revision.artifacts.len();
    }
}

fn save_artifact_manifest(
    connection: &Connection,
    session_id: &str,
    artifact: &CadArtifact,
) -> SessionRepositoryResult<()> {
    let metadata = artifact.metadata.clone().unwrap_or_default();
    let relative_path = metadata
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Artifact manifest {} is missing relativePath.", artifact.id))?;
    let sha256 = metadata
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Artifact manifest {} is missing sha256.", artifact.id))?;
    connection
        .execute(
            r#"
            INSERT INTO artifacts (
              id, session_id, revision_id, kind, format, relative_path, uri,
              sha256, bytes, created_at, deleted_at, missing_at, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
              session_id = excluded.session_id,
              revision_id = excluded.revision_id,
              kind = excluded.kind,
              format = excluded.format,
              relative_path = excluded.relative_path,
              uri = excluded.uri,
              sha256 = excluded.sha256,
              bytes = excluded.bytes,
              created_at = excluded.created_at,
              deleted_at = excluded.deleted_at,
              missing_at = excluded.missing_at,
              metadata_json = excluded.metadata_json
            "#,
            params![
                artifact.id,
                session_id,
                artifact.revision_id,
                to_db_text(&artifact.kind)?,
                artifact.format,
                relative_path,
                artifact.uri,
                sha256,
                artifact.bytes.unwrap_or_default() as i64,
                artifact.created_at,
                artifact.deleted_at,
                artifact.missing_at,
                serde_json::to_string(&metadata).map_err(|error| error.to_string())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_conversation_message(
    connection: &Connection,
    message: &CadConversationMessage,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO conversation_messages (
              id, session_id, revision_id, run_id, role, content, created_at, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                message.id,
                message.session_id,
                message.revision_id,
                message.run_id,
                to_db_text(&message.role)?,
                message.content,
                message.created_at,
                optional_metadata_json(message.metadata.as_ref())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_agent_run(connection: &Connection, run: &CadAgentRun) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO agent_runs (
              id, session_id, input_revision_id, output_revision_id, status, prompt,
              created_at, updated_at, started_at, completed_at, error, active_step,
              external_agent, external_thread_id, external_turn_id, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL)
            ON CONFLICT(id) DO UPDATE SET
              session_id = excluded.session_id,
              input_revision_id = excluded.input_revision_id,
              output_revision_id = excluded.output_revision_id,
              status = excluded.status,
              prompt = excluded.prompt,
              created_at = excluded.created_at,
              updated_at = excluded.updated_at,
              started_at = excluded.started_at,
              completed_at = excluded.completed_at,
              error = excluded.error,
              active_step = excluded.active_step,
              external_agent = excluded.external_agent,
              external_thread_id = excluded.external_thread_id,
              external_turn_id = excluded.external_turn_id
            "#,
            params![
                run.id,
                run.session_id,
                run.input_revision_id,
                run.output_revision_id,
                to_db_text(&run.status)?,
                run.prompt,
                run.created_at,
                run.updated_at,
                run.started_at,
                run.completed_at,
                run.error,
                run.active_step,
                run.external_agent,
                run.external_thread_id,
                run.external_turn_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_agent_run_event(
    connection: &mut Connection,
    event: &CadAgentRunEvent,
) -> SessionRepositoryResult<CadAgentRunEvent> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some(sequence) = transaction
        .query_row(
            "SELECT sequence FROM agent_run_events WHERE id = ?1",
            params![event.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        let mut saved = event.clone();
        saved.sequence = sequence.max(0) as u64;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(saved);
    }
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_run_events WHERE run_id = ?1",
            params![event.run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    let mut saved = event.clone();
    saved.sequence = sequence.max(1) as u64;
    transaction
        .execute(
            r#"
            INSERT INTO agent_run_events (
              id, session_id, run_id, revision_id, event_type, sequence,
              created_at, payload_json, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                saved.id,
                saved.session_id,
                saved.run_id,
                saved.revision_id,
                to_db_text(&saved.event_type)?,
                saved.sequence as i64,
                saved.created_at,
                serde_json::to_string(&saved.payload).map_err(|error| error.to_string())?,
                optional_metadata_json(saved.metadata.as_ref())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

fn save_workflow_plan(
    connection: &Connection,
    plan: &CadWorkflowPlan,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_plans (
              run_id, revision_id, plan_json, source_language, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(run_id) DO UPDATE SET
              revision_id = excluded.revision_id,
              plan_json = excluded.plan_json,
              source_language = excluded.source_language,
              created_at = excluded.created_at
            "#,
            params![
                plan.run_id,
                plan.revision_id,
                serde_json::to_string(&plan.plan).map_err(|error| error.to_string())?,
                to_db_text(&plan.source_language)?,
                plan.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_workflow_outer_iteration(
    connection: &Connection,
    iteration: &CadWorkflowOuterIteration,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_outer_iterations (
              id, run_id, iteration, revision_id, structural_report_json,
              vlm_report_json, failure_report_json, passed, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
              run_id = excluded.run_id,
              iteration = excluded.iteration,
              revision_id = excluded.revision_id,
              structural_report_json = excluded.structural_report_json,
              vlm_report_json = excluded.vlm_report_json,
              failure_report_json = excluded.failure_report_json,
              passed = excluded.passed,
              created_at = excluded.created_at
            "#,
            params![
                iteration.id,
                iteration.run_id,
                iteration.iteration as i64,
                iteration.revision_id,
                serde_json::to_string(&iteration.structural_report)
                    .map_err(|error| error.to_string())?,
                optional_json_value_text(iteration.vlm_report.as_ref())?,
                optional_json_value_text(iteration.failure_report.as_ref())?,
                i64::from(iteration.passed),
                iteration.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn save_workflow_pending_vlm(
    connection: &Connection,
    pending_vlm: &CadWorkflowPendingVlm,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_pending_vlm (
              run_id, artifact_id, contract_json, pass_threshold, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(run_id) DO UPDATE SET
              artifact_id = excluded.artifact_id,
              contract_json = excluded.contract_json,
              pass_threshold = excluded.pass_threshold,
              created_at = excluded.created_at
            "#,
            params![
                pending_vlm.run_id,
                pending_vlm.artifact_id,
                serde_json::to_string(&pending_vlm.contract).map_err(|error| error.to_string())?,
                pending_vlm.pass_threshold,
                pending_vlm.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn artifact_metadata(metadata_json: Option<String>) -> SessionRepositoryResult<Metadata> {
    let Some(metadata_json) = metadata_json else {
        return Ok(Metadata::new());
    };
    match serde_json::from_str(&metadata_json) {
        Ok(metadata) => Ok(metadata),
        Err(error) => {
            let mut metadata = Metadata::new();
            metadata.insert(
                "metadataRecovery".to_string(),
                json!({
                    "status": "corrupt-metadata",
                    "error": error.to_string()
                }),
            );
            Ok(metadata)
        }
    }
}

fn optional_metadata(metadata_json: Option<String>) -> SessionRepositoryResult<Option<Metadata>> {
    metadata_json
        .map(|json| artifact_metadata(Some(json)))
        .transpose()
}

fn optional_metadata_json(metadata: Option<&Metadata>) -> SessionRepositoryResult<Option<String>> {
    metadata
        .map(|metadata| serde_json::to_string(metadata).map_err(|error| error.to_string()))
        .transpose()
}

fn optional_json_value(value: Option<String>) -> SessionRepositoryResult<Option<Value>> {
    value
        .map(|json| serde_json::from_str(&json).map_err(|error| error.to_string()))
        .transpose()
}

fn optional_json_value_text(value: Option<&Value>) -> SessionRepositoryResult<Option<String>> {
    value
        .map(|value| serde_json::to_string(value).map_err(|error| error.to_string()))
        .transpose()
}

fn load_current_session_id(connection: &Connection) -> SessionRepositoryResult<Option<String>> {
    connection
        .query_row(
            r#"
            SELECT id
            FROM sessions
            WHERE deleted_at IS NULL AND archived_at IS NULL
            ORDER BY COALESCE(last_viewed_at, updated_at) DESC
            LIMIT 1
            "#,
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

fn rebuild_loaded_revision_summaries(
    sessions: &mut HashMap<String, CadSession>,
    revisions: &HashMap<String, CadRevision>,
    agent_runs: &HashMap<String, Vec<CadAgentRun>>,
    session_id: &str,
) {
    let mut summaries: Vec<CadRevisionSummary> = revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| CadRevisionSummary {
            id: revision.id.clone(),
            source_hash: storage::sha256_hex(revision.source.as_bytes()),
            parent_revision_id: revision.parent_revision_id.clone(),
            restored_from_revision_id: revision.restored_from_revision_id.clone(),
            source_language: revision.source_language.clone(),
            created_at: revision.created_at.clone(),
            diagnostics: revision.diagnostics.clone(),
            artifact_count: revision.artifact_count,
            run_links: loaded_revision_run_links(agent_runs, session_id, &revision.id),
        })
        .collect();
    summaries.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    if let Some(session) = sessions.get_mut(session_id) {
        session.revisions = summaries;
    }
}

fn loaded_revision_run_links(
    agent_runs: &HashMap<String, Vec<CadAgentRun>>,
    session_id: &str,
    revision_id: &str,
) -> Vec<CadRevisionRunLink> {
    let mut links = Vec::new();
    for run in agent_runs.get(session_id).into_iter().flatten() {
        if run.input_revision_id.as_deref() == Some(revision_id) {
            links.push(CadRevisionRunLink {
                run_id: run.id.clone(),
                role: "input".to_string(),
                status: run.status.clone(),
                updated_at: run.updated_at.clone(),
            });
        }
        if run.output_revision_id.as_deref() == Some(revision_id) {
            links.push(CadRevisionRunLink {
                run_id: run.id.clone(),
                role: "output".to_string(),
                status: run.status.clone(),
                updated_at: run.updated_at.clone(),
            });
        }
    }
    links.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
    links
}

fn revision_user_events(
    metadata_json: Option<String>,
) -> SessionRepositoryResult<Vec<CadUserEvent>> {
    let Some(metadata_json) = metadata_json else {
        return Ok(Vec::new());
    };
    let Ok(metadata) = serde_json::from_str::<Value>(&metadata_json) else {
        return Ok(Vec::new());
    };
    metadata
        .get("userEvents")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| error.to_string())
        .map(|events| events.unwrap_or_default())
}

fn recover_runtime_kind(value: &str) -> (CadRuntimeKind, Option<CadDiagnostic>) {
    match from_db_text(value) {
        Ok(runtime) => (runtime, None),
        Err(_) => (
            CadRuntimeKind::OpenscadWasm,
            Some(CadDiagnostic {
                severity: "warning".to_string(),
                message: format!(
                    "Unknown persisted runtime {value:?}; recovered with openscad-wasm."
                ),
                line: None,
                column: None,
            }),
        ),
    }
}

fn recover_source_language(value: &str) -> (CadSourceLanguage, Option<CadDiagnostic>) {
    match from_db_text(value) {
        Ok(source_language) => (source_language, None),
        Err(_) => (
            CadSourceLanguage::Openscad,
            Some(CadDiagnostic {
                severity: "warning".to_string(),
                message: format!(
                    "Unknown persisted source language {value:?}; recovered with openscad."
                ),
                line: None,
                column: None,
            }),
        ),
    }
}

fn recover_diagnostics(value: &str) -> CadDiagnostics {
    serde_json::from_str(value).unwrap_or_else(|error| CadDiagnostics {
        ok: false,
        elapsed_ms: 0,
        items: vec![CadDiagnostic {
            severity: "warning".to_string(),
            message: format!("Corrupt persisted diagnostics were reset: {error}"),
            line: None,
            column: None,
        }],
    })
}

fn to_db_text<T: Serialize>(value: &T) -> SessionRepositoryResult<String> {
    match serde_json::to_value(value).map_err(|error| error.to_string())? {
        Value::String(value) => Ok(value),
        other => Err(format!("Expected string-backed enum, got {other:?}")),
    }
}

fn from_db_text<T: DeserializeOwned>(value: &str) -> SessionRepositoryResult<T> {
    serde_json::from_value(Value::String(value.to_string())).map_err(|error| error.to_string())
}

fn to_rusqlite_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(error.into())
}
