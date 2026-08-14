use crate::protocol::*;
use crate::runtime::{extract_open_scad_parameters, ok_diagnostics};
#[cfg(test)]
use crate::session_repository::InMemorySessionRepository;
use crate::session_repository::{SessionRepository, SessionRepositorySnapshot};
use crate::storage::{self, StorageLayout};
use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use uuid::Uuid;

mod agent_persistence;
mod artifact_management;
mod artifact_paths;
mod artifacts;
mod conversation;
mod events;
mod guards;
mod operations;
mod recovery;
mod refresh;
mod revisions;
mod runs;
mod runtime_render;
mod sessions;
mod state_view;
mod support;
mod validation_batch_persistence;
mod validation_persistence;

use artifact_paths::*;
use events::*;
use guards::*;
use runtime_render::*;
use state_view::*;
pub use support::metadata_from_value;
pub(crate) use support::timestamp;
use support::{
    json_to_parameter_value, lock_error, propose_session_title, source_hash, uuid,
    verify_diagnostic,
};

#[derive(Default)]
pub(crate) struct ServiceState {
    pub(crate) sessions: HashMap<String, CadSession>,
    pub(crate) revisions: HashMap<String, CadRevision>,
    pub(crate) artifacts: HashMap<String, CadArtifact>,
    pub(crate) messages: HashMap<String, Vec<CadUserMessage>>,
    pub(crate) conversation: HashMap<String, Vec<CadConversationMessage>>,
    pub(crate) agent_threads: HashMap<String, Vec<CadAgentThread>>,
    pub(crate) agent_runs: HashMap<String, Vec<CadAgentRun>>,
    pub(crate) agent_run_events: HashMap<String, Vec<CadAgentRunEvent>>,
    pub(crate) agent_transport_events: HashMap<String, Vec<CadAgentTransportEvent>>,
    pub(crate) validation_evaluations: HashMap<String, Vec<CadValidationEvaluation>>,
    pub(crate) validation_evaluation_events: HashMap<String, Vec<CadValidationEvaluationEvent>>,
    pub(crate) validation_batches: HashMap<String, Vec<CadValidationBatch>>,
    pub(crate) validation_checks: HashMap<String, Vec<CadValidationCheck>>,
    pub(crate) workflow_plans: HashMap<String, CadWorkflowPlan>,
    pub(crate) workflow_outer_iterations: HashMap<String, Vec<CadWorkflowOuterIteration>>,
    pub(crate) workflow_pending_vlm: HashMap<String, CadWorkflowPendingVlm>,
    pub(crate) current_interactive_session_id: Option<String>,
    pub(crate) has_completed_first_run: bool,
}

impl From<SessionRepositorySnapshot> for ServiceState {
    fn from(snapshot: SessionRepositorySnapshot) -> Self {
        let mut messages = HashMap::new();
        let mut conversation = HashMap::new();
        let mut agent_threads = HashMap::new();
        let mut agent_runs = HashMap::new();
        let mut agent_run_events = HashMap::new();
        let mut agent_transport_events = HashMap::new();
        let mut validation_evaluations = HashMap::new();
        let mut validation_evaluation_events = HashMap::new();
        let mut validation_batches = HashMap::new();
        let mut validation_checks = HashMap::new();
        let workflow_plans = snapshot.workflow_plans;
        let workflow_outer_iterations = snapshot.workflow_outer_iterations;
        let workflow_pending_vlm = snapshot.workflow_pending_vlm;
        let has_completed_first_run = snapshot.has_completed_first_run;
        for session_id in snapshot.sessions.keys() {
            messages.insert(session_id.clone(), Vec::new());
            conversation.insert(
                session_id.clone(),
                snapshot
                    .conversation
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            agent_threads.insert(
                session_id.clone(),
                snapshot
                    .agent_threads
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            agent_runs.insert(
                session_id.clone(),
                snapshot
                    .agent_runs
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            agent_run_events.insert(
                session_id.clone(),
                snapshot
                    .agent_run_events
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            agent_transport_events.insert(
                session_id.clone(),
                snapshot
                    .agent_transport_events
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            validation_evaluations.insert(
                session_id.clone(),
                snapshot
                    .validation_evaluations
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            validation_evaluation_events.insert(
                session_id.clone(),
                snapshot
                    .validation_evaluation_events
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            validation_batches.insert(
                session_id.clone(),
                snapshot
                    .validation_batches
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            validation_checks.insert(
                session_id.clone(),
                snapshot
                    .validation_checks
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        Self {
            sessions: snapshot.sessions,
            revisions: snapshot.revisions,
            artifacts: snapshot.artifacts,
            messages,
            conversation,
            agent_threads,
            agent_runs,
            agent_run_events,
            agent_transport_events,
            validation_evaluations,
            validation_evaluation_events,
            validation_batches,
            validation_checks,
            workflow_plans,
            workflow_outer_iterations,
            workflow_pending_vlm,
            current_interactive_session_id: snapshot.current_interactive_session_id,
            has_completed_first_run,
        }
    }
}

pub struct SessionService {
    inner: Mutex<ServiceState>,
    storage_layout: StorageLayout,
    repository: Arc<dyn SessionRepository>,
    event_sender: broadcast::Sender<CadBridgeEvent>,
    stream_sender: broadcast::Sender<CadAgentStreamEvent>,
}

impl SessionService {
    #[cfg(test)]
    pub fn new(artifact_root: PathBuf) -> Self {
        Self::with_storage_layout(StorageLayout::from_artifact_root(artifact_root))
    }

    #[cfg(test)]
    pub fn with_storage_layout(storage_layout: StorageLayout) -> Self {
        Self::with_repository(
            storage_layout,
            Arc::new(InMemorySessionRepository::default()),
        )
        .expect("in-memory session repository cannot fail")
    }

    pub(crate) fn with_repository(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
    ) -> Result<Self, String> {
        Self::with_repository_options(storage_layout, repository, true, true)
    }

    pub(crate) fn with_repository_without_startup_verification(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
    ) -> Result<Self, String> {
        Self::with_repository_options(storage_layout, repository, false, false)
    }

    fn with_repository_options(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
        verify_artifacts: bool,
        normalize_process_state: bool,
    ) -> Result<Self, String> {
        let (event_sender, _) = broadcast::channel(256);
        let (stream_sender, _) = broadcast::channel(1024);
        let snapshot = repository.load()?;
        let service = Self {
            inner: Mutex::new(ServiceState::from(snapshot)),
            storage_layout,
            repository,
            event_sender,
            stream_sender,
        };
        if normalize_process_state {
            service.normalize_agent_threads_after_process_restart()?;
            service.normalize_validation_batches_after_process_restart()?;
        }
        if verify_artifacts {
            service.verify_artifact_files_inner(None)?;
        }
        Ok(service)
    }

    pub(crate) fn normalize_agent_threads_after_process_restart(&self) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let mut changed_sessions = Vec::new();
        for (session_id, threads) in &mut state.agent_threads {
            let mut changed = false;
            for thread in threads {
                if matches!(
                    thread.status,
                    CadAgentThreadStatus::Ready | CadAgentThreadStatus::Active
                ) {
                    thread.status = CadAgentThreadStatus::NotLoaded;
                    thread.connection_generation = None;
                    thread.updated_at = timestamp();
                    changed = true;
                }
            }
            if changed {
                changed_sessions.push(session_id.clone());
            }
        }
        for session_id in changed_sessions {
            self.persist_session_graph(&state, &session_id)?;
        }
        Ok(())
    }

    pub(crate) fn normalize_validation_batches_after_process_restart(&self) -> Result<(), String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let running_checks = state
            .validation_checks
            .values()
            .flatten()
            .filter(|check| {
                check.status == CadValidationCheckStatus::Running
                    && matches!(
                        check.kind,
                        CadValidationCheckKind::Structural | CadValidationCheckKind::Dfm
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        for persisted in running_checks {
            let mut reset = persisted.clone();
            reset.status = CadValidationCheckStatus::Queued;
            reset.started_at = None;
            let saved = self
                .repository
                .update_validation_check(&reset, &persisted.status)?;
            let slot = state
                .validation_checks
                .get_mut(&saved.session_id)
                .into_iter()
                .flatten()
                .find(|check| check.id == saved.id)
                .ok_or_else(|| format!("Validation check state not found: {}", saved.id))?;
            *slot = saved;
        }
        let claimed_batches = state
            .validation_batches
            .values()
            .flatten()
            .filter_map(|batch| {
                batch
                    .settlement_claimed_at
                    .as_ref()
                    .map(|claim| (batch.session_id.clone(), batch.id.clone(), claim.clone()))
            })
            .collect::<Vec<_>>();
        for (session_id, batch_id, claim_token) in claimed_batches {
            let saved = self.repository.release_validation_batch_settlement(
                &session_id,
                &batch_id,
                &claim_token,
            )?;
            let slot = state
                .validation_batches
                .get_mut(&session_id)
                .into_iter()
                .flatten()
                .find(|batch| batch.id == batch_id)
                .ok_or_else(|| format!("Validation batch state not found: {batch_id}"))?;
            *slot = saved;
        }
        let effect_claims = state
            .validation_batches
            .values()
            .flatten()
            .filter_map(|batch| {
                batch
                    .effects_claimed_at
                    .as_ref()
                    .map(|claim| (batch.session_id.clone(), batch.id.clone(), claim.clone()))
            })
            .collect::<Vec<_>>();
        for (session_id, batch_id, claim_token) in effect_claims {
            let saved = self.repository.release_validation_batch_effects(
                &session_id,
                &batch_id,
                &claim_token,
            )?;
            let slot = state
                .validation_batches
                .get_mut(&session_id)
                .into_iter()
                .flatten()
                .find(|batch| batch.id == batch_id)
                .ok_or_else(|| format!("Validation batch state not found: {batch_id}"))?;
            *slot = saved;
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CadBridgeEvent> {
        self.event_sender.subscribe()
    }

    pub fn subscribe_agent_stream(&self) -> broadcast::Receiver<CadAgentStreamEvent> {
        self.stream_sender.subscribe()
    }

    pub fn emit_agent_stream(&self, event: CadAgentStreamEvent) -> Result<(), String> {
        validate_agent_stream_event(&event)?;
        // No receiver is valid outside the Tauri runtime (for example service
        // tests), so broadcast's zero-subscriber error is not a delivery error.
        let _ = self.stream_sender.send(event);
        Ok(())
    }

    pub fn app_data_dir(&self) -> &Path {
        self.storage_layout.app_data_dir()
    }

    fn emit(&self, event_type: CadBridgeEventType, session_id: &str, state: CadSessionState) {
        let _ = self.event_sender.send(CadBridgeEvent {
            id: uuid(),
            event_type,
            session_id: session_id.to_string(),
            created_at: timestamp(),
            state,
        });
    }

    fn persist_session_graph(&self, state: &ServiceState, session_id: &str) -> Result<(), String> {
        self.repository.save_session_graph(state, session_id)
    }
}

fn validate_agent_stream_event(event: &CadAgentStreamEvent) -> Result<(), String> {
    if event.session_id.trim().is_empty()
        || event.run_id.trim().is_empty()
        || event.thread_id.trim().is_empty()
        || event.turn_id.trim().is_empty()
        || event.item_id.trim().is_empty()
    {
        return Err("Agent stream identifiers cannot be empty.".to_string());
    }
    if event.completed && !event.delta.is_empty() {
        return Err("Completed agent stream event delta must be empty.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
