use crate::protocol::*;
use crate::session_service::ServiceState;
use crate::storage::{self, StorageLayout};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

mod load;
mod save;
mod support;

use load::*;
use save::*;
use support::*;

pub(crate) type SessionRepositoryResult<T> = Result<T, String>;

#[derive(Default)]
pub(crate) struct SessionRepositorySnapshot {
    pub sessions: HashMap<String, CadSession>,
    pub revisions: HashMap<String, CadRevision>,
    pub artifacts: HashMap<String, CadArtifact>,
    pub conversation: HashMap<String, Vec<CadConversationMessage>>,
    pub agent_threads: HashMap<String, Vec<CadAgentThread>>,
    pub agent_runs: HashMap<String, Vec<CadAgentRun>>,
    pub agent_run_events: HashMap<String, Vec<CadAgentRunEvent>>,
    pub agent_transport_events: HashMap<String, Vec<CadAgentTransportEvent>>,
    pub validation_evaluations: HashMap<String, Vec<CadValidationEvaluation>>,
    pub validation_evaluation_events: HashMap<String, Vec<CadValidationEvaluationEvent>>,
    pub workflow_plans: HashMap<String, CadWorkflowPlan>,
    pub workflow_outer_iterations: HashMap<String, Vec<CadWorkflowOuterIteration>>,
    pub workflow_pending_vlm: HashMap<String, CadWorkflowPendingVlm>,
    pub current_interactive_session_id: Option<String>,
    pub has_completed_first_run: bool,
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
    fn update_artifact_profile_hash(
        &self,
        session_id: &str,
        artifact_id: &str,
        profile_hash: &str,
    ) -> SessionRepositoryResult<()>;
    fn save_conversation_message(
        &self,
        message: &CadConversationMessage,
    ) -> SessionRepositoryResult<CadConversationMessage>;
    fn create_agent_run_with_user_message(
        &self,
        session: &CadSession,
        run: &CadAgentRun,
        message: &CadConversationMessage,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<(CadConversationMessage, CadAgentRunEvent)>;
    fn save_agent_thread(&self, thread: &CadAgentThread) -> SessionRepositoryResult<()>;
    fn replace_agent_thread(
        &self,
        archived_thread: &CadAgentThread,
        replacement: &CadAgentThread,
    ) -> SessionRepositoryResult<()>;
    fn save_agent_run(&self, run: &CadAgentRun) -> SessionRepositoryResult<()>;
    fn save_agent_run_event(
        &self,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<CadAgentRunEvent>;
    fn save_agent_transport_event(
        &self,
        event: &CadAgentTransportEvent,
    ) -> SessionRepositoryResult<CadAgentTransportEvent>;
    #[cfg(test)]
    fn create_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation>;
    fn create_next_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation>;
    fn update_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation>;
    fn save_validation_evaluation_event(
        &self,
        event: &CadValidationEvaluationEvent,
    ) -> SessionRepositoryResult<CadValidationEvaluationEvent>;
    fn delete_agent_transport_events(&self, event_ids: &[String])
        -> SessionRepositoryResult<usize>;
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
    fn set_app_kv_json(&self, key: &str, value: &Value) -> SessionRepositoryResult<()>;
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

    fn update_artifact_profile_hash(
        &self,
        _session_id: &str,
        _artifact_id: &str,
        _profile_hash: &str,
    ) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn save_conversation_message(
        &self,
        message: &CadConversationMessage,
    ) -> SessionRepositoryResult<CadConversationMessage> {
        Ok(message.clone())
    }

    fn create_agent_run_with_user_message(
        &self,
        _session: &CadSession,
        _run: &CadAgentRun,
        message: &CadConversationMessage,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<(CadConversationMessage, CadAgentRunEvent)> {
        Ok((message.clone(), event.clone()))
    }

    fn save_agent_thread(&self, _thread: &CadAgentThread) -> SessionRepositoryResult<()> {
        Ok(())
    }

    fn replace_agent_thread(
        &self,
        _archived_thread: &CadAgentThread,
        _replacement: &CadAgentThread,
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

    fn save_agent_transport_event(
        &self,
        event: &CadAgentTransportEvent,
    ) -> SessionRepositoryResult<CadAgentTransportEvent> {
        Ok(event.clone())
    }

    #[cfg(test)]
    fn create_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        Ok(evaluation.clone())
    }

    fn create_next_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        let mut evaluation = evaluation.clone();
        let contract = evaluation.input_contract.as_object_mut().ok_or_else(|| {
            "Validation evaluation input_contract must be a JSON object.".to_string()
        })?;
        if contract
            .get("evaluationId")
            .is_some_and(|value| value.as_str() != Some(evaluation.id.as_str()))
        {
            return Err("Validation evaluation contract evaluationId mismatch.".to_string());
        }
        if contract
            .get("attempt")
            .is_some_and(|value| value.as_u64() != Some(u64::from(evaluation.attempt)))
        {
            return Err("Validation evaluation contract attempt mismatch.".to_string());
        }
        contract.insert(
            "evaluationId".to_string(),
            Value::String(evaluation.id.clone()),
        );
        contract.insert(
            "attempt".to_string(),
            Value::Number(serde_json::Number::from(evaluation.attempt)),
        );
        Ok(evaluation)
    }

    fn update_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        Ok(evaluation.clone())
    }

    fn save_validation_evaluation_event(
        &self,
        event: &CadValidationEvaluationEvent,
    ) -> SessionRepositoryResult<CadValidationEvaluationEvent> {
        Ok(event.clone())
    }

    fn delete_agent_transport_events(
        &self,
        event_ids: &[String],
    ) -> SessionRepositoryResult<usize> {
        Ok(event_ids.len())
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

    fn set_app_kv_json(&self, _key: &str, _value: &Value) -> SessionRepositoryResult<()> {
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
        let agent_threads = load_agent_threads(&connection)?;
        let agent_runs = load_agent_runs(&connection)?;
        let agent_run_events = load_agent_run_events(&connection)?;
        let agent_transport_events = load_agent_transport_events(&connection)?;
        let validation_evaluations = load_validation_evaluations(&connection)?;
        let validation_evaluation_events = load_validation_evaluation_events(&connection)?;
        let workflow_plans = load_workflow_plans(&connection)?;
        let workflow_outer_iterations = load_workflow_outer_iterations(&connection)?;
        let workflow_pending_vlm = load_workflow_pending_vlm(&connection)?;
        attach_artifacts_to_revisions(&mut revisions, &artifacts);
        for session_id in sessions.keys().cloned().collect::<Vec<_>>() {
            rebuild_loaded_revision_summaries(&mut sessions, &revisions, &agent_runs, &session_id);
        }
        let current_interactive_session_id = load_current_session_id(&connection)?;
        let has_completed_first_run = load_app_kv_bool(&connection, "hasCompletedFirstRun")?;
        Ok(SessionRepositorySnapshot {
            sessions,
            revisions,
            artifacts,
            conversation,
            agent_threads,
            agent_runs,
            agent_run_events,
            agent_transport_events,
            validation_evaluations,
            validation_evaluation_events,
            workflow_plans,
            workflow_outer_iterations,
            workflow_pending_vlm,
            current_interactive_session_id,
            has_completed_first_run,
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
        let persisted_deleted_at: Option<Option<String>> = transaction
            .query_row(
                "SELECT deleted_at FROM sessions WHERE id = ?1",
                params![session.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if persisted_deleted_at
            .as_ref()
            .and_then(|deleted_at| deleted_at.as_ref())
            .is_some()
            && session.deleted_at.is_none()
        {
            return Err(format!(
                "CAD session has been deleted and cannot be saved from a stale process: {}",
                session.id
            ));
        }
        transaction
            .execute(
                r#"
                INSERT INTO sessions (
                  id, title, title_source, selected_runtime, status, active_revision_id,
                  created_at, updated_at, last_viewed_at, connected_ui_clients,
                  archived_at, deleted_at, metadata_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
                ON CONFLICT(id) DO UPDATE SET
                  title = excluded.title,
                  title_source = excluded.title_source,
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
                    to_db_text(&session.title_source)?,
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

    fn update_artifact_profile_hash(
        &self,
        session_id: &str,
        artifact_id: &str,
        profile_hash: &str,
    ) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        update_artifact_profile_hash(&connection, session_id, artifact_id, profile_hash)
    }

    fn save_conversation_message(
        &self,
        message: &CadConversationMessage,
    ) -> SessionRepositoryResult<CadConversationMessage> {
        let connection = self.connection()?;
        save_conversation_message(&connection, message)
    }

    fn create_agent_run_with_user_message(
        &self,
        session: &CadSession,
        run: &CadAgentRun,
        message: &CadConversationMessage,
        event: &CadAgentRunEvent,
    ) -> SessionRepositoryResult<(CadConversationMessage, CadAgentRunEvent)> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let changed_rows = transaction
            .execute(
                r#"
                UPDATE sessions
                SET title = ?1, title_source = ?2, updated_at = ?3
                WHERE id = ?4 AND deleted_at IS NULL
                "#,
                params![
                    session.title,
                    to_db_text(&session.title_source)?,
                    session.updated_at,
                    session.id,
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed_rows != 1 {
            return Err(format!(
                "Atomic agent run creation expected one active session row, changed {changed_rows}: {}",
                session.id
            ));
        }
        save_agent_run(&transaction, run)?;
        let saved_event = save_agent_run_event_in_transaction(&transaction, event)?;
        let saved_message = save_conversation_message(&transaction, message)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok((saved_message, saved_event))
    }

    fn save_agent_thread(&self, thread: &CadAgentThread) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        save_agent_thread(&connection, thread)
    }

    fn replace_agent_thread(
        &self,
        archived_thread: &CadAgentThread,
        replacement: &CadAgentThread,
    ) -> SessionRepositoryResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut temporarily_archived = archived_thread.clone();
        temporarily_archived.replaced_by_id = None;
        save_agent_thread(&transaction, &temporarily_archived)?;
        save_agent_thread(&transaction, replacement)?;
        save_agent_thread(&transaction, archived_thread)?;
        transaction.commit().map_err(|error| error.to_string())
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

    fn save_agent_transport_event(
        &self,
        event: &CadAgentTransportEvent,
    ) -> SessionRepositoryResult<CadAgentTransportEvent> {
        let connection = self.connection()?;
        save_agent_transport_event(&connection, event)
    }

    #[cfg(test)]
    fn create_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        let connection = self.connection()?;
        create_validation_evaluation(&connection, evaluation)
    }

    fn create_next_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        let mut connection = self.connection()?;
        create_next_validation_evaluation(&mut connection, evaluation)
    }

    fn update_validation_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> SessionRepositoryResult<CadValidationEvaluation> {
        let connection = self.connection()?;
        update_validation_evaluation(&connection, evaluation)
    }

    fn save_validation_evaluation_event(
        &self,
        event: &CadValidationEvaluationEvent,
    ) -> SessionRepositoryResult<CadValidationEvaluationEvent> {
        let connection = self.connection()?;
        save_validation_evaluation_event(&connection, event)
    }

    fn delete_agent_transport_events(
        &self,
        event_ids: &[String],
    ) -> SessionRepositoryResult<usize> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let mut deleted_count = 0;
        for event_id in event_ids {
            let changed_rows = transaction
                .execute(
                    "DELETE FROM agent_transport_events WHERE id = ?1",
                    params![event_id],
                )
                .map_err(|error| error.to_string())?;
            if changed_rows != 1 {
                return Err(format!(
                    "Agent transport cleanup expected one row for event {event_id}, changed {changed_rows}."
                ));
            }
            deleted_count += changed_rows;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(deleted_count)
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
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM agent_threads WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|error| error.to_string())?;
        let changed_rows = transaction
            .execute(
                "UPDATE sessions SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![deleted_at, session_id],
            )
            .map_err(|error| error.to_string())?;
        if changed_rows != 1 {
            return Err(format!(
                "CAD session delete expected one row, changed {changed_rows}: {session_id}"
            ));
        }
        transaction.commit().map_err(|error| error.to_string())?;
        eprintln!(
            "[cadastrophe:delete-session] sqlite delete update finished session_id={} deleted_at={} changed_rows={}",
            session_id, deleted_at, changed_rows
        );
        Ok(())
    }

    fn set_app_kv_json(&self, key: &str, value: &Value) -> SessionRepositoryResult<()> {
        let connection = self.connection()?;
        connection
            .execute(
                r#"
                INSERT INTO app_kv (key, value_json, updated_at)
                VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ON CONFLICT(key) DO UPDATE SET
                  value_json = excluded.value_json,
                  updated_at = excluded.updated_at
                "#,
                params![
                    key,
                    serde_json::to_string(value).map_err(|error| error.to_string())?,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
