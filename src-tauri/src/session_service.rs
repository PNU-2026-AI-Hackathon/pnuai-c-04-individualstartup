use crate::protocol::*;
use crate::runtime::{extract_open_scad_parameters, ok_diagnostics, DEFAULT_SAMPLE_SOURCE};
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

mod artifact_management;
mod artifact_paths;
mod artifacts;
mod conversation;
mod events;
mod guards;
mod refresh;
mod revisions;
mod runs;
mod runtime_render;
mod sessions;
mod state_view;
mod support;

use artifact_paths::*;
use events::*;
use guards::*;
use runtime_render::*;
use state_view::*;
pub use support::metadata_from_value;
use support::{
    json_to_parameter_value, lock_error, propose_session_title, source_hash, timestamp, uuid,
    verify_diagnostic,
};

#[derive(Default)]
pub(crate) struct ServiceState {
    pub(crate) sessions: HashMap<String, CadSession>,
    pub(crate) revisions: HashMap<String, CadRevision>,
    pub(crate) artifacts: HashMap<String, CadArtifact>,
    pub(crate) messages: HashMap<String, Vec<CadUserMessage>>,
    pub(crate) conversation: HashMap<String, Vec<CadConversationMessage>>,
    pub(crate) agent_runs: HashMap<String, Vec<CadAgentRun>>,
    pub(crate) agent_run_events: HashMap<String, Vec<CadAgentRunEvent>>,
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
        let mut agent_runs = HashMap::new();
        let mut agent_run_events = HashMap::new();
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
        }
        Self {
            sessions: snapshot.sessions,
            revisions: snapshot.revisions,
            artifacts: snapshot.artifacts,
            messages,
            conversation,
            agent_runs,
            agent_run_events,
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
}

impl SessionService {
    #[cfg(test)]
    pub fn new(artifact_root: PathBuf) -> Self {
        Self::with_storage_layout(StorageLayout::from_artifact_root(artifact_root))
    }

    #[cfg(test)]
    pub fn with_storage_layout(storage_layout: StorageLayout) -> Self {
        Self::with_repository(storage_layout, Arc::new(InMemorySessionRepository))
            .expect("in-memory session repository cannot fail")
    }

    pub(crate) fn with_repository(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
    ) -> Result<Self, String> {
        Self::with_repository_options(storage_layout, repository, true)
    }

    pub(crate) fn with_repository_without_startup_verification(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
    ) -> Result<Self, String> {
        Self::with_repository_options(storage_layout, repository, false)
    }

    fn with_repository_options(
        storage_layout: StorageLayout,
        repository: Arc<dyn SessionRepository>,
        verify_artifacts: bool,
    ) -> Result<Self, String> {
        let (event_sender, _) = broadcast::channel(256);
        let snapshot = repository.load()?;
        let service = Self {
            inner: Mutex::new(ServiceState::from(snapshot)),
            storage_layout,
            repository,
            event_sender,
        };
        if verify_artifacts {
            service.verify_artifact_files_inner(None)?;
        }
        Ok(service)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CadBridgeEvent> {
        self.event_sender.subscribe()
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

#[cfg(test)]
mod tests;
