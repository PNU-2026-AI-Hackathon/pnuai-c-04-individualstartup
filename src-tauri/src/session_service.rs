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

    pub fn create_session(
        &self,
        input: CreateCadSessionInput,
    ) -> Result<CreateCadSessionResult, String> {
        let now = timestamp();
        let session_id = uuid();
        let session = CadSession {
            id: session_id.clone(),
            created_at: now.clone(),
            updated_at: now,
            last_viewed_at: None,
            connected_ui_clients: 0,
            title: Some(
                input
                    .title
                    .unwrap_or_else(|| "Untitled CAD session".to_string()),
            ),
            active_revision_id: None,
            selected_runtime: input
                .selected_runtime
                .unwrap_or(CadRuntimeKind::OpenscadWasm),
            status: CadSessionStatus::Idle,
            recovery_diagnostics: Vec::new(),
            archived_at: None,
            deleted_at: None,
            revisions: Vec::new(),
        };
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            state.sessions.insert(session_id.clone(), session);
            state.messages.insert(session_id.clone(), Vec::new());
            state.conversation.insert(session_id.clone(), Vec::new());
            state.agent_runs.insert(session_id.clone(), Vec::new());
            state
                .agent_run_events
                .insert(session_id.clone(), Vec::new());
        }
        self.update_model_source(UpdateModelSourceInput {
            session_id: session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: DEFAULT_SAMPLE_SOURCE.to_string(),
            parent_revision_id: None,
            parameters: None,
        })?;
        let state = self.get_session_state(&session_id)?;
        self.emit(
            CadBridgeEventType::SessionCreated,
            &session_id,
            state.clone(),
        );
        Ok(CreateCadSessionResult {
            session_id: session_id.clone(),
            ui_url: format!("/sessions/{session_id}"),
            state,
        })
    }

    pub fn get_current_session(&self) -> Result<CurrentCadSessionResult, String> {
        let session_id = {
            let state = self.inner.lock().map_err(lock_error)?;
            state.current_interactive_session_id.clone()
        };
        let Some(session_id) = session_id else {
            return Ok(CurrentCadSessionResult::default());
        };
        Ok(CurrentCadSessionResult {
            ui_url: Some(format!("/sessions/{session_id}")),
            state: Some(self.get_session_state(&session_id)?),
            session_id: Some(session_id),
        })
    }

    pub fn mark_session_viewed(&self, session_id: &str) -> Result<CadSessionState, String> {
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session_mut(&mut state, session_id)?;
            let now = timestamp();
            let session = state.sessions.get_mut(session_id).expect("session checked");
            session.last_viewed_at = Some(now.clone());
            session.updated_at = now;
            if session.archived_at.is_none() {
                state.current_interactive_session_id = Some(session_id.to_string());
            }
            self.persist_session_graph(&state, session_id)?;
        }
        self.get_session_state(session_id)
    }

    #[cfg(test)]
    pub fn list_sessions(&self, include_archived: bool) -> Result<Vec<CadSessionListItem>, String> {
        self.list_sessions_for_input(ListCadSessionsInput {
            include_archived,
            query: None,
        })
        .map(|result| result.sessions)
    }

    pub fn list_sessions_for_input(
        &self,
        input: ListCadSessionsInput,
    ) -> Result<ListCadSessionsResult, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let query = normalized_query(input.query.as_deref());
        let mut sessions: Vec<CadSessionListItem> = state
            .sessions
            .values()
            .filter(|session| session.deleted_at.is_none())
            .filter(|session| input.include_archived || session.archived_at.is_none())
            .filter(|session| {
                query
                    .as_deref()
                    .is_none_or(|query| session_matches_search(&state, session, query))
            })
            .map(|session| session_list_item(&state, session))
            .collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(ListCadSessionsResult {
            sessions,
            search_fields: vec![
                "title".to_string(),
                "source".to_string(),
                "conversation".to_string(),
            ],
        })
    }

    pub fn rename_session(&self, input: RenameCadSessionInput) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.title = Some(input.title.trim().to_string());
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn archive_session(
        &self,
        input: ArchiveCadSessionInput,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let now = timestamp();
            let archived = input.archived.unwrap_or(true);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.archived_at = archived.then_some(now.clone());
            session.updated_at = now;
            if archived
                && state
                    .current_interactive_session_id
                    .as_deref()
                    .is_some_and(|current| current == input.session_id)
            {
                state.current_interactive_session_id = None;
            }
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<DeleteCadSessionResult, String> {
        let deleted_at = timestamp();
        let current_session_id = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, session_id)?;
            state.sessions.remove(session_id);
            state.messages.remove(session_id);
            state.conversation.remove(session_id);
            let run_ids: Vec<String> = state
                .agent_runs
                .get(session_id)
                .into_iter()
                .flatten()
                .map(|run| run.id.clone())
                .collect();
            state.agent_runs.remove(session_id);
            state.agent_run_events.remove(session_id);
            for run_id in &run_ids {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            let revision_ids: Vec<String> = state
                .revisions
                .values()
                .filter(|revision| revision.session_id == session_id)
                .map(|revision| revision.id.clone())
                .collect();
            for revision_id in &revision_ids {
                state.revisions.remove(revision_id);
            }
            state
                .artifacts
                .retain(|_, artifact| !revision_ids.contains(&artifact.revision_id));
            if state
                .current_interactive_session_id
                .as_deref()
                .is_some_and(|current| current == session_id)
            {
                state.current_interactive_session_id = None;
            }
            state.current_interactive_session_id.clone()
        };
        self.repository.delete_session(session_id, &deleted_at)?;
        Ok(DeleteCadSessionResult {
            session_id: session_id.to_string(),
            current_session_id,
        })
    }

    pub fn duplicate_session(
        &self,
        input: DuplicateCadSessionInput,
    ) -> Result<CreateCadSessionResult, String> {
        let now = timestamp();
        let new_session_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let source_session = require_session(&state, &input.session_id)?.clone();
            let active_revision = source_session
                .active_revision_id
                .as_ref()
                .and_then(|revision_id| state.revisions.get(revision_id))
                .cloned();
            let title = input.title.or_else(|| {
                source_session
                    .title
                    .as_ref()
                    .map(|title| format!("{title} copy"))
            });
            let active_revision_id = active_revision.as_ref().map(|_| uuid());
            let session = CadSession {
                id: new_session_id.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
                last_viewed_at: None,
                connected_ui_clients: 0,
                title,
                active_revision_id: active_revision_id.clone(),
                selected_runtime: source_session.selected_runtime,
                status: CadSessionStatus::Idle,
                recovery_diagnostics: source_session.recovery_diagnostics,
                archived_at: None,
                deleted_at: None,
                revisions: Vec::new(),
            };
            state.sessions.insert(new_session_id.clone(), session);
            state.messages.insert(new_session_id.clone(), Vec::new());
            state
                .conversation
                .insert(new_session_id.clone(), Vec::new());
            state.agent_runs.insert(new_session_id.clone(), Vec::new());
            state
                .agent_run_events
                .insert(new_session_id.clone(), Vec::new());
            if let (Some(mut revision), Some(new_revision_id)) =
                (active_revision, active_revision_id)
            {
                revision.id = new_revision_id.clone();
                revision.session_id = new_session_id.clone();
                revision.parent_revision_id = None;
                revision.restored_from_revision_id = None;
                revision.source_hash = source_hash(&revision.source);
                revision.created_at = now.clone();
                revision.artifact_count = 0;
                revision.artifacts = Vec::new();
                revision.user_events = Vec::new();
                revision.run_links = Vec::new();
                state.revisions.insert(new_revision_id, revision);
            }
            rebuild_revision_summaries(&mut state, &new_session_id);
            self.persist_session_graph(&state, &new_session_id)?;
            build_state(&state, &new_session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionCreated,
            &new_session_id,
            snapshot.clone(),
        );
        Ok(CreateCadSessionResult {
            session_id: new_session_id.clone(),
            ui_url: format!("/sessions/{new_session_id}"),
            state: snapshot,
        })
    }

    pub fn update_model_source(
        &self,
        input: UpdateModelSourceInput,
    ) -> Result<UpdateModelSourceResult, String> {
        let revision_id = uuid();
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, &input.session_id)?;
            let now = timestamp();
            let parameters = input
                .parameters
                .unwrap_or_else(|| match input.source_language {
                    CadSourceLanguage::Openscad => extract_open_scad_parameters(&input.source),
                    _ => Vec::new(),
                });
            let revision = CadRevision {
                id: revision_id.clone(),
                session_id: input.session_id.clone(),
                parent_revision_id: input.parent_revision_id,
                restored_from_revision_id: None,
                source_hash: source_hash(&input.source),
                source_language: input.source_language,
                source: input.source,
                parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
                run_links: Vec::new(),
            };
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(revision_id.clone());
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionCreated,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok(UpdateModelSourceResult {
            revision_id,
            state: state_snapshot,
        })
    }

    pub fn set_active_revision(
        &self,
        input: SetActiveRevisionInput,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, &input.revision_id)?;
            if revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {} does not belong to session {}.",
                    input.revision_id, input.session_id
                ));
            }
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(input.revision_id.clone());
            session.updated_at = timestamp();
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionActivated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn restore_revision(
        &self,
        input: RestoreRevisionInput,
    ) -> Result<RestoreRevisionResult, String> {
        let revision_id = uuid();
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?.clone();
            let source_revision = require_revision(&state, &input.revision_id)?.clone();
            if source_revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {} does not belong to session {}.",
                    input.revision_id, input.session_id
                ));
            }
            let now = timestamp();
            let mut revision = CadRevision {
                id: revision_id.clone(),
                session_id: input.session_id.clone(),
                parent_revision_id: session.active_revision_id.clone(),
                restored_from_revision_id: Some(input.revision_id.clone()),
                source_hash: source_hash(&source_revision.source),
                source_language: source_revision.source_language,
                source: source_revision.source,
                parameters: source_revision.parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
                run_links: Vec::new(),
            };
            add_user_event(
                &mut revision,
                "revision.restored",
                json!({ "restoredFromRevisionId": input.revision_id }),
            );
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(revision_id.clone());
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionRestored,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok(RestoreRevisionResult {
            revision_id,
            state: state_snapshot,
        })
    }

    pub fn render_preview(
        &self,
        input: RenderPreviewInput,
    ) -> Result<(CadPreviewResult, CadSessionState), String> {
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = CadSessionStatus::Rendering;
            session.updated_at = timestamp();
            let snapshot = build_state(&state, &input.session_id)?;
            self.emit(
                CadBridgeEventType::SessionUpdated,
                &input.session_id,
                snapshot,
            );
        }

        let (revision_id, mesh, diagnostics, preview_artifact, stl_artifact) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone())
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let revision = require_revision(&state, &revision_id)?.clone();
            drop(state);
            let rendered =
                render_open_scad_wasm_node(&revision.source, self.storage_layout.app_data_dir())?;
            let diagnostics = rendered.diagnostics.clone();
            if !diagnostics.ok {
                (revision_id, None, diagnostics, None, None)
            } else {
                let mesh = rendered.mesh.clone().ok_or_else(|| {
                    "OpenSCAD WASM render did not return preview mesh.".to_string()
                })?;
                let metadata = Some(runtime_artifact_metadata(
                    &revision.source,
                    &revision.parameters,
                    &rendered,
                    "backend-preview",
                )?);
                let preview_artifact = self.write_artifact(
                    &revision_id,
                    CadArtifactKind::PreviewMesh,
                    "json",
                    &serde_json::to_string(&mesh).map_err(|error| error.to_string())?,
                    metadata.clone(),
                )?;
                let stl_bytes = rendered
                    .stl_base64
                    .as_ref()
                    .map(|contents| {
                        base64::engine::general_purpose::STANDARD
                            .decode(contents.as_bytes())
                            .map_err(|error| error.to_string())
                    })
                    .transpose()?
                    .ok_or_else(|| "OpenSCAD WASM render did not return STL bytes.".to_string())?;
                let stl_artifact = self.write_artifact_bytes(
                    &revision_id,
                    CadArtifactKind::Stl,
                    "stl",
                    &stl_bytes,
                    metadata,
                )?;
                (
                    revision_id,
                    Some(mesh),
                    diagnostics,
                    Some(preview_artifact),
                    Some(stl_artifact),
                )
            }
        };
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &revision_id)?;
            revision.diagnostics = diagnostics.clone();
            revision
                .artifacts
                .retain(|candidate| candidate.kind != CadArtifactKind::PreviewMesh);
            if let Some(artifact) = preview_artifact.clone() {
                revision.artifacts.push(artifact);
            }
            if let Some(artifact) = stl_artifact.clone() {
                revision.artifacts.push(artifact);
            }
            revision.artifact_count = revision.artifacts.len();
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = if diagnostics.ok {
                CadSessionStatus::Idle
            } else {
                CadSessionStatus::Failed
            };
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::PreviewRendered,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok((
            CadPreviewResult {
                diagnostics,
                mesh,
                artifacts: preview_artifact.into_iter().chain(stl_artifact).collect(),
            },
            state_snapshot,
        ))
    }

    pub fn persist_runtime_artifact(
        &self,
        input: PersistRuntimeArtifactInput,
    ) -> Result<PersistRuntimeArtifactResult, String> {
        if input.kind != CadArtifactKind::PreviewMesh && input.kind != CadArtifactKind::Stl {
            return Err(
                "Runtime artifact persistence supports preview-mesh and stl only.".to_string(),
            );
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(input.contents_base64.as_bytes())
            .map_err(|error| format!("Runtime artifact contents are not valid base64: {error}"))?;
        {
            let state = self.inner.lock().map_err(lock_error)?;
            validate_revision_session(&state, &input.session_id, &input.revision_id)?;
        }
        let artifact = self.write_artifact_bytes(
            &input.revision_id,
            input.kind.clone(),
            &input.format,
            &bytes,
            Some(Value::Object(input.metadata)),
        )?;
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &input.revision_id)?;
            if input.kind == CadArtifactKind::PreviewMesh {
                revision
                    .artifacts
                    .retain(|candidate| candidate.kind != CadArtifactKind::PreviewMesh);
            }
            revision.artifacts.push(artifact.clone());
            revision.artifact_count = revision.artifacts.len();
            revision.diagnostics = input.diagnostics.clone();
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = if input.diagnostics.ok {
                CadSessionStatus::Idle
            } else {
                CadSessionStatus::Failed
            };
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        let event_type = match input.kind {
            CadArtifactKind::PreviewMesh => CadBridgeEventType::PreviewRendered,
            _ => CadBridgeEventType::ArtifactExported,
        };
        self.emit(event_type, &input.session_id, snapshot.clone());
        Ok(PersistRuntimeArtifactResult {
            artifact,
            state: snapshot,
        })
    }

    pub fn record_runtime_diagnostics(
        &self,
        session_id: &str,
        revision_id: &str,
        diagnostics: CadDiagnostics,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_revision_session(&state, session_id, revision_id)?;
            let revision = require_revision_mut(&mut state, revision_id)?;
            revision.diagnostics = diagnostics.clone();
            let session = require_session_mut(&mut state, session_id)?;
            session.status = if diagnostics.ok {
                CadSessionStatus::Idle
            } else {
                CadSessionStatus::Failed
            };
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, session_id);
            self.persist_session_graph(&state, session_id)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::PreviewRendered,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn update_parameters(
        &self,
        session_id: &str,
        values: Map<String, Value>,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let active_revision_id = require_session(&state, session_id)?
                .active_revision_id
                .clone()
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let source_revision = require_revision(&state, &active_revision_id)?.clone();
            let now = timestamp();
            let revision_id = uuid();
            let mut parameters = source_revision.parameters.clone();
            for parameter in &mut parameters {
                if let Some(value) = values.get(&parameter.name) {
                    parameter.value = json_to_parameter_value(value.clone());
                }
            }
            let mut revision = CadRevision {
                id: revision_id.clone(),
                session_id: session_id.to_string(),
                parent_revision_id: Some(active_revision_id),
                restored_from_revision_id: None,
                source_hash: source_hash(&source_revision.source),
                source_language: source_revision.source_language,
                source: source_revision.source,
                parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
                run_links: Vec::new(),
            };
            add_user_event(
                &mut revision,
                "parameter.updated",
                json!({ "values": values }),
            );
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, session_id)?;
            session.active_revision_id = Some(revision_id);
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, session_id);
            self.persist_session_graph(&state, session_id)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionCreated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn post_user_message(
        &self,
        input: PostUserMessageInput,
    ) -> Result<(CadUserMessage, CadSessionState), String> {
        let message_id = uuid();
        let created_at = timestamp();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone());
            let mut event_id = None;
            if let Some(revision_id) = &revision_id {
                let revision = require_revision_mut(&mut state, revision_id)?;
                let event = add_user_event(
                    revision,
                    "message.created",
                    json!({"channel": "web-ui", "message": input.message}),
                );
                event_id = Some(event.id);
            }
            let message = CadUserMessage {
                id: message_id.clone(),
                session_id: input.session_id.clone(),
                revision_id: revision_id.clone(),
                event_id,
                channel: CadUserMessageChannel::WebUi,
                message: input.message.trim().to_string(),
                created_at: created_at.clone(),
            };
            state
                .messages
                .entry(input.session_id.clone())
                .or_default()
                .push(message);
            let conversation_message = CadConversationMessage {
                id: uuid(),
                session_id: input.session_id.clone(),
                revision_id,
                role: CadConversationRole::User,
                content: input.message.trim().to_string(),
                created_at: created_at.clone(),
                run_id: None,
                metadata: Some(metadata_from_value(
                    json!({"channel": "web-ui", "legacyMessageId": message_id}),
                )),
            };
            state
                .conversation
                .entry(input.session_id.clone())
                .or_default()
                .push(conversation_message.clone());
            self.repository
                .save_conversation_message(&conversation_message)?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.updated_at = created_at;
            build_state(&state, &input.session_id)?
        };
        let message = snapshot
            .messages
            .iter()
            .find(|message| message.id == message_id)
            .cloned()
            .ok_or_else(|| "Message write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::MessageCreated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok((message, snapshot))
    }

    pub fn create_conversation_message(
        &self,
        session_id: &str,
        revision_id: Option<String>,
        role: CadConversationRole,
        content: String,
        run_id: Option<String>,
        metadata: Option<Metadata>,
    ) -> Result<(CadConversationMessage, CadSessionState), String> {
        let message_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, session_id)?;
            let message = CadConversationMessage {
                id: message_id.clone(),
                session_id: session_id.to_string(),
                revision_id,
                role,
                content: content.trim().to_string(),
                created_at: timestamp(),
                run_id,
                metadata,
            };
            state
                .conversation
                .entry(session_id.to_string())
                .or_default()
                .push(message.clone());
            self.repository.save_conversation_message(&message)?;
            if let Some(run_id) = &message.run_id {
                if matches!(
                    message.role,
                    CadConversationRole::Assistant
                        | CadConversationRole::System
                        | CadConversationRole::Tool
                ) {
                    let event = append_agent_run_event(
                        &mut state,
                        session_id,
                        run_id,
                        message.revision_id.clone(),
                        CadAgentRunEventType::AgentMessageCreated,
                        json!({
                            "messageId": message.id,
                            "role": message.role,
                            "content": message.content
                        }),
                        None,
                    );
                    persist_agent_run_event(
                        self.repository.as_ref(),
                        &mut state,
                        session_id,
                        event,
                    )?;
                }
            }
            build_state(&state, session_id)?
        };
        let message = snapshot
            .conversation
            .iter()
            .find(|message| message.id == message_id)
            .cloned()
            .ok_or_else(|| "Conversation write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::AgentMessageCreated,
            session_id,
            snapshot.clone(),
        );
        Ok((message, snapshot))
    }

    pub fn create_agent_run(
        &self,
        session_id: &str,
        prompt: String,
        input_revision_id: Option<String>,
        external_agent: Option<String>,
        retry_of_run_id: Option<String>,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let run_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let resolved_input_revision_id = {
                let session = require_session(&state, session_id)?;
                input_revision_id.or_else(|| session.active_revision_id.clone())
            };
            if let Some(revision_id) = &resolved_input_revision_id {
                let revision = require_revision(&state, revision_id)?;
                if revision.session_id != session_id {
                    return Err(format!(
                        "CAD revision {revision_id} does not belong to session {session_id}."
                    ));
                }
            }
            if let Some(retry_of_run_id) = &retry_of_run_id {
                let retry_source_exists = state
                    .agent_runs
                    .get(session_id)
                    .into_iter()
                    .flatten()
                    .any(|run| run.id == *retry_of_run_id);
                if !retry_source_exists {
                    return Err(format!(
                        "Retry source agent run not found: {retry_of_run_id}"
                    ));
                }
            }
            let now = timestamp();
            let run = CadAgentRun {
                id: run_id.clone(),
                session_id: session_id.to_string(),
                input_revision_id: resolved_input_revision_id.clone(),
                output_revision_id: None,
                status: CadAgentRunStatus::Queued,
                prompt,
                created_at: now.clone(),
                updated_at: now,
                started_at: None,
                completed_at: None,
                error: None,
                active_step: None,
                external_agent,
                external_thread_id: None,
                external_turn_id: None,
            };
            state
                .agent_runs
                .entry(session_id.to_string())
                .or_default()
                .push(run.clone());
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                &run.id,
                resolved_input_revision_id.clone(),
                CadAgentRunEventType::AgentRunCreated,
                json!({
                    "status": &run.status,
                    "prompt": &run.prompt,
                    "inputRevisionId": resolved_input_revision_id,
                    "retryOfRunId": retry_of_run_id
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = timestamp();
            build_state(&state, session_id)?
        };
        let run = snapshot
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
            .ok_or_else(|| "Run write failed.".to_string())?;
        self.emit(
            CadBridgeEventType::AgentRunCreated,
            session_id,
            snapshot.clone(),
        );
        Ok((run, snapshot))
    }

    pub fn link_agent_run_output_revision(
        &self,
        session_id: &str,
        run_id: &str,
        output_revision_id: String,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, &output_revision_id)?;
            if revision.session_id != session_id {
                return Err(format!(
                    "CAD revision {output_revision_id} does not belong to session {session_id}."
                ));
            }
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            run.output_revision_id = Some(output_revision_id.clone());
            run.updated_at = timestamp();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                Some(output_revision_id.clone()),
                CadAgentRunEventType::AgentRunUpdated,
                json!({ "outputRevisionId": output_revision_id }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            rebuild_revision_summaries(&mut state, session_id);
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn update_agent_run(
        &self,
        session_id: &str,
        run_id: &str,
        status: Option<CadAgentRunStatus>,
        active_step: Option<Option<String>>,
        error: Option<String>,
        event_type: Option<CadBridgeEventType>,
        event_payload: Option<Value>,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let now = timestamp();
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            let previous_status = run.status.clone();
            let previous_active_step = run.active_step.clone();
            if let Some(status) = status {
                if status == CadAgentRunStatus::Running && run.started_at.is_none() {
                    run.started_at = Some(now.clone());
                }
                if is_terminal_run_status(&status) && run.completed_at.is_none() {
                    run.completed_at = Some(now.clone());
                }
                run.status = status;
            }
            if let Some(active_step) = active_step {
                run.active_step = active_step;
            }
            if let Some(error) = error.clone() {
                run.error = Some(error);
            }
            run.updated_at = now.clone();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event_type = run_event_type_for_update(event_type.as_ref(), &run.status);
            let mut payload = metadata_from_value(event_payload.unwrap_or_else(|| json!({})));
            payload.insert(
                "previousStatus".to_string(),
                serde_json::to_value(previous_status).map_err(|error| error.to_string())?,
            );
            payload.insert(
                "status".to_string(),
                serde_json::to_value(&run.status).map_err(|error| error.to_string())?,
            );
            if previous_active_step != run.active_step {
                payload.insert(
                    "previousActiveStep".to_string(),
                    serde_json::to_value(previous_active_step)
                        .map_err(|error| error.to_string())?,
                );
                payload.insert(
                    "activeStep".to_string(),
                    serde_json::to_value(&run.active_step).map_err(|error| error.to_string())?,
                );
            }
            if let Some(error) = error {
                payload.insert("error".to_string(), Value::String(error));
            }
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                run.input_revision_id.clone(),
                event_type,
                Value::Object(payload),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = now;
            build_state(&state, session_id)?
        };
        let run = snapshot
            .agent_runs
            .iter()
            .find(|run| run.id == run_id)
            .cloned()
            .ok_or_else(|| "Run update failed.".to_string())?;
        let event_type = event_type.unwrap_or_else(|| event_type_for_run_status(&run.status));
        self.emit(event_type, session_id, snapshot.clone());
        Ok((run, snapshot))
    }

    pub fn update_agent_run_external_metadata(
        &self,
        session_id: &str,
        run_id: &str,
        external_agent: Option<String>,
        external_thread_id: Option<String>,
        external_turn_id: Option<String>,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
            if external_agent.is_some() {
                run.external_agent = external_agent.clone();
            }
            if external_thread_id.is_some() {
                run.external_thread_id = external_thread_id.clone();
            }
            if external_turn_id.is_some() {
                run.external_turn_id = external_turn_id.clone();
            }
            run.updated_at = timestamp();
            let run = run.clone();
            self.repository.save_agent_run(&run)?;
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                run.input_revision_id.clone(),
                CadAgentRunEventType::AgentRunUpdated,
                json!({
                    "externalAgent": external_agent,
                    "externalThreadId": external_thread_id,
                    "externalTurnId": external_turn_id
                }),
                None,
            );
            persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn save_workflow_plan(
        &self,
        session_id: &str,
        plan: CadWorkflowPlan,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &plan.run_id)?;
            if let Some(revision_id) = &plan.revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            if plan.plan.source_language != plan.source_language {
                return Err(
                    "Workflow plan source_language must match CadModelPlan source_language."
                        .to_string(),
                );
            }
            self.repository.save_workflow_plan(&plan)?;
            state.workflow_plans.insert(plan.run_id.clone(), plan);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn save_workflow_outer_iteration(
        &self,
        session_id: &str,
        iteration: CadWorkflowOuterIteration,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &iteration.run_id)?;
            if let Some(revision_id) = &iteration.revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            self.repository.save_workflow_outer_iteration(&iteration)?;
            let iterations = state
                .workflow_outer_iterations
                .entry(iteration.run_id.clone())
                .or_default();
            if let Some(existing) = iterations
                .iter_mut()
                .find(|candidate| candidate.id == iteration.id)
            {
                *existing = iteration;
            } else {
                iterations.push(iteration);
            }
            iterations.sort_by(|left, right| {
                left.iteration
                    .cmp(&right.iteration)
                    .then_with(|| left.created_at.cmp(&right.created_at))
            });
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn save_workflow_pending_vlm(
        &self,
        session_id: &str,
        pending_vlm: CadWorkflowPendingVlm,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, &pending_vlm.run_id)?;
            validate_artifact_session(&state, session_id, &pending_vlm.artifact_id)?;
            self.repository.save_workflow_pending_vlm(&pending_vlm)?;
            state
                .workflow_pending_vlm
                .insert(pending_vlm.run_id.clone(), pending_vlm);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn clear_workflow_pending_vlm(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<CadWorkflowState, String> {
        let workflow = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, run_id)?;
            self.repository.clear_workflow_pending_vlm(run_id)?;
            state.workflow_pending_vlm.remove(run_id);
            build_workflow_state(&state, session_id)?
        };
        Ok(workflow)
    }

    pub fn record_agent_tool_event(
        &self,
        session_id: &str,
        run_id: &str,
        revision_id: Option<String>,
        event_type: CadAgentRunEventType,
        payload: Value,
    ) -> Result<CadAgentRunEvent, String> {
        let bridge_event_type = match event_type {
            CadAgentRunEventType::AgentToolStarted => CadBridgeEventType::AgentToolStarted,
            CadAgentRunEventType::AgentToolCompleted => CadBridgeEventType::AgentToolCompleted,
            _ => {
                return Err(
                    "record_agent_tool_event only accepts agent.tool.started/completed."
                        .to_string(),
                )
            }
        };
        let (event, snapshot) = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            validate_workflow_run(&state, session_id, run_id)?;
            if let Some(revision_id) = &revision_id {
                validate_revision_session(&state, session_id, revision_id)?;
            }
            let event = append_agent_run_event(
                &mut state,
                session_id,
                run_id,
                revision_id,
                event_type,
                payload,
                None,
            );
            let event =
                persist_agent_run_event(self.repository.as_ref(), &mut state, session_id, event)?;
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = timestamp();
            let snapshot = build_state(&state, session_id)?;
            (event, snapshot)
        };
        self.emit(bridge_event_type, session_id, snapshot);
        Ok(event)
    }

    pub fn revision_prompt_context(
        &self,
        session_id: &str,
        revision_id: Option<&str>,
    ) -> Result<(Option<String>, Option<CadSourceLanguage>, Option<String>), String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let session = require_session(&state, session_id)?;
        let revision_id = revision_id
            .map(ToString::to_string)
            .or_else(|| session.active_revision_id.clone());
        let Some(revision_id) = revision_id else {
            return Ok((None, None, None));
        };
        let revision = require_revision(&state, &revision_id)?;
        if revision.session_id != session_id {
            return Err(format!(
                "CAD revision {revision_id} does not belong to session {session_id}."
            ));
        }
        Ok((
            Some(revision_id),
            Some(revision.source_language.clone()),
            Some(revision.source.clone()),
        ))
    }

    pub fn get_revision(&self, session_id: &str, revision_id: &str) -> Result<CadRevision, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        validate_revision_session(&state, session_id, revision_id)?;
        require_revision(&state, revision_id).cloned()
    }

    pub fn record_agent_failure(
        &self,
        session_id: &str,
        message: &str,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let active_revision_id = require_session(&state, session_id)?
                .active_revision_id
                .clone();
            if let Some(revision_id) = active_revision_id {
                let revision = require_revision_mut(&mut state, &revision_id)?;
                revision.diagnostics.ok = false;
                revision.diagnostics.items.push(CadDiagnostic {
                    severity: "error".to_string(),
                    message: message.to_string(),
                    line: None,
                    column: None,
                });
            }
            let session = require_session_mut(&mut state, session_id)?;
            session.status = CadSessionStatus::Failed;
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, session_id);
            self.persist_session_graph(&state, session_id)?;
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn list_agent_runs(&self, session_id: &str) -> Result<Vec<CadAgentRun>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .agent_runs
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn get_agent_run(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<Option<CadAgentRun>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .agent_runs
            .get(session_id)
            .and_then(|runs| runs.iter().find(|run| run.id == run_id).cloned()))
    }

    pub fn export_artifact(
        &self,
        input: ExportArtifactInput,
    ) -> Result<(CadExportResult, CadSessionState), String> {
        let (revision_id, format) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone())
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let revision = require_revision(&state, &revision_id)?;
            if revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {revision_id} does not belong to session {}.",
                    input.session_id
                ));
            }
            (revision_id, input.format.clone())
        };

        let (diagnostics, artifact) = if format == "metadata" {
            let contents = serde_json::to_string_pretty(
                &json!({"revisionId": revision_id, "runtime": "openscad-wasm"}),
            )
            .map_err(|error| error.to_string())?;
            (
                ok_diagnostics(1),
                Some(self.write_artifact(
                    &revision_id,
                    CadArtifactKind::Metadata,
                    &format,
                    &contents,
                    None,
                )?),
            )
        } else {
            let artifact = {
                let state = self.inner.lock().map_err(lock_error)?;
                require_revision(&state, &revision_id)?
                    .artifacts
                    .iter()
                    .rev()
                    .find(|artifact| {
                        artifact.kind == CadArtifactKind::Stl && artifact.format == format
                    })
                    .cloned()
            };
            if artifact.is_some() {
                (ok_diagnostics(0), artifact)
            } else {
                let revision = {
                    let state = self.inner.lock().map_err(lock_error)?;
                    require_revision(&state, &revision_id)?.clone()
                };
                let rendered =
                    render_open_scad_wasm_node(&revision.source, self.storage_layout.app_data_dir())?;
                if !rendered.diagnostics.ok {
                    (rendered.diagnostics, None)
                } else {
                    let stl_base64 = rendered.stl_base64.as_ref().ok_or_else(|| {
                        "OpenSCAD WASM render did not return STL bytes.".to_string()
                    })?;
                    let stl_bytes = base64::engine::general_purpose::STANDARD
                        .decode(stl_base64.as_bytes())
                        .map_err(|error| error.to_string())?;
                    (
                        rendered.diagnostics.clone(),
                        Some(self.write_artifact_bytes(
                            &revision_id,
                            CadArtifactKind::Stl,
                            &format,
                            &stl_bytes,
                            Some(runtime_artifact_metadata(
                                &revision.source,
                                &revision.parameters,
                                &rendered,
                                "backend-export",
                            )?),
                        )?),
                    )
                }
            }
        };
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            if let Some(artifact) = &artifact {
                let revision = require_revision_mut(&mut state, &revision_id)?;
                if !revision
                    .artifacts
                    .iter()
                    .any(|candidate| candidate.id == artifact.id)
                {
                    revision.artifacts.push(artifact.clone());
                }
                revision.artifact_count = revision.artifacts.len();
            }
            if let Some(revision) = state.revisions.get_mut(&revision_id) {
                revision.diagnostics = diagnostics.clone();
            }
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::ArtifactExported,
            &input.session_id,
            snapshot.clone(),
        );
        Ok((
            CadExportResult {
                diagnostics,
                artifact,
            },
            snapshot,
        ))
    }

    pub fn read_artifact(&self, artifact_id: &str) -> Result<String, String> {
        let artifact = self.load_artifact_manifest(artifact_id)?;
        if artifact.deleted_at.is_some() {
            return Err("Artifact has been deleted.".to_string());
        }
        let path = self.artifact_manifest_path(&artifact)?;
        match fs::read_to_string(&path) {
            Ok(contents) => Ok(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.mark_artifact_missing(artifact_id, Some(timestamp()))?;
                Err(format!("Artifact file is missing: {}", path.display()))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn open_artifact(&self, artifact_id: &str) -> Result<OpenArtifactResult, String> {
        let artifact = self.load_artifact_manifest(artifact_id)?;
        if artifact.deleted_at.is_some() {
            return Err("Artifact has been deleted.".to_string());
        }
        let path = self.artifact_manifest_path(&artifact)?;
        if !path.exists() {
            self.mark_artifact_missing(artifact_id, Some(timestamp()))?;
            return Err(format!("Artifact file is missing: {}", path.display()));
        }
        Ok(OpenArtifactResult {
            artifact,
            path: path.to_string_lossy().to_string(),
        })
    }

    pub fn delete_artifact(
        &self,
        input: DeleteArtifactInput,
    ) -> Result<DeleteArtifactResult, String> {
        let artifact = self.load_artifact_manifest(&input.artifact_id)?;
        if self.artifact_session_id(&artifact)? != input.session_id {
            return Err(format!(
                "Artifact {} does not belong to session {}.",
                input.artifact_id, input.session_id
            ));
        }
        let deleted_at = timestamp();
        let path = self.artifact_manifest_path(&artifact)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        self.repository
            .mark_artifact_deleted(&input.artifact_id, &deleted_at)?;
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            state.artifacts.remove(&input.artifact_id);
            for revision in state
                .revisions
                .values_mut()
                .filter(|revision| revision.session_id == input.session_id)
            {
                revision
                    .artifacts
                    .retain(|artifact| artifact.id != input.artifact_id);
                revision.artifact_count = revision.artifacts.len();
            }
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.updated_at = deleted_at;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::ArtifactDeleted,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(DeleteArtifactResult {
            artifact_id: input.artifact_id,
            state: snapshot,
        })
    }

    pub fn verify_artifact_files(
        &self,
        session_id: Option<String>,
    ) -> Result<VerifyArtifactFilesResult, String> {
        let result = self.verify_artifact_files_inner(session_id.as_deref())?;
        let state = match session_id {
            Some(session_id) => Some(self.get_session_state(&session_id)?),
            None => None,
        };
        if let Some(state) = &state {
            self.emit(
                CadBridgeEventType::ArtifactVerified,
                &state.session.id,
                state.clone(),
            );
        }
        Ok(VerifyArtifactFilesResult { state, ..result })
    }

    pub fn cleanup_orphan_artifacts(
        &self,
        input: CleanupOrphanArtifactsInput,
    ) -> Result<CleanupOrphanArtifactsResult, String> {
        let known_paths = {
            let state = self.inner.lock().map_err(lock_error)?;
            state
                .artifacts
                .values()
                .filter_map(|artifact| self.artifact_manifest_path(artifact).ok())
                .collect::<std::collections::HashSet<_>>()
        };
        let mut checked_file_count = 0;
        let mut orphan_paths = Vec::new();
        let mut deleted_paths = Vec::new();
        for file_path in collect_artifact_files(self.storage_layout.artifact_root())? {
            checked_file_count += 1;
            if known_paths.contains(&file_path) {
                continue;
            }
            let display_path = file_path.to_string_lossy().to_string();
            orphan_paths.push(display_path.clone());
            if !input.dry_run {
                fs::remove_file(&file_path).map_err(|error| error.to_string())?;
                deleted_paths.push(display_path);
            }
        }
        Ok(CleanupOrphanArtifactsResult {
            checked_file_count,
            orphan_paths,
            deleted_paths,
        })
    }

    pub fn get_session_state(&self, session_id: &str) -> Result<CadSessionState, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        build_state(&state, session_id)
    }

    pub fn refresh_session_from_repository(
        &self,
        session_id: &str,
    ) -> Result<CadSessionState, String> {
        let snapshot_state = ServiceState::from(self.repository.load()?);
        let mut refreshed_session = snapshot_state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("CAD session not found: {session_id}"))?;
        let refreshed_run_ids = snapshot_state
            .agent_runs
            .get(session_id)
            .into_iter()
            .flatten()
            .map(|run| run.id.clone())
            .collect::<HashSet<_>>();
        let refreshed_revision_ids = snapshot_state
            .revisions
            .values()
            .filter(|revision| revision.session_id == session_id)
            .map(|revision| revision.id.clone())
            .collect::<HashSet<_>>();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            if let Some(existing_session) = state.sessions.get(session_id) {
                refreshed_session.connected_ui_clients = existing_session.connected_ui_clients;
            }
            let previous_revision_ids = state
                .revisions
                .values()
                .filter(|revision| revision.session_id == session_id)
                .map(|revision| revision.id.clone())
                .collect::<HashSet<_>>();
            let previous_run_ids = state
                .agent_runs
                .get(session_id)
                .into_iter()
                .flatten()
                .map(|run| run.id.clone())
                .collect::<HashSet<_>>();
            state
                .sessions
                .insert(session_id.to_string(), refreshed_session);
            state
                .revisions
                .retain(|revision_id, _| !previous_revision_ids.contains(revision_id));
            state.revisions.extend(
                snapshot_state
                    .revisions
                    .iter()
                    .filter(|(_, revision)| revision.session_id == session_id)
                    .map(|(revision_id, revision)| (revision_id.clone(), revision.clone())),
            );
            state
                .artifacts
                .retain(|_, artifact| !previous_revision_ids.contains(&artifact.revision_id));
            state.artifacts.extend(
                snapshot_state
                    .artifacts
                    .iter()
                    .filter(|(_, artifact)| refreshed_revision_ids.contains(&artifact.revision_id))
                    .map(|(artifact_id, artifact)| (artifact_id.clone(), artifact.clone())),
            );
            state.messages.entry(session_id.to_string()).or_default();
            state.conversation.insert(
                session_id.to_string(),
                snapshot_state
                    .conversation
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            state.agent_runs.insert(
                session_id.to_string(),
                snapshot_state
                    .agent_runs
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            state.agent_run_events.insert(
                session_id.to_string(),
                snapshot_state
                    .agent_run_events
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            for run_id in previous_run_ids.difference(&refreshed_run_ids) {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            for run_id in &refreshed_run_ids {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            for (run_id, plan) in snapshot_state.workflow_plans {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_plans.insert(run_id, plan);
                }
            }
            for (run_id, iterations) in snapshot_state.workflow_outer_iterations {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_outer_iterations.insert(run_id, iterations);
                }
            }
            for (run_id, pending_vlm) in snapshot_state.workflow_pending_vlm {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_pending_vlm.insert(run_id, pending_vlm);
                }
            }
            rebuild_revision_summaries(&mut state, session_id);
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    fn write_artifact(
        &self,
        revision_id: &str,
        kind: CadArtifactKind,
        format: &str,
        contents: &str,
        metadata: Option<Value>,
    ) -> Result<CadArtifact, String> {
        self.write_artifact_bytes(revision_id, kind, format, contents.as_bytes(), metadata)
    }

    fn write_artifact_bytes(
        &self,
        revision_id: &str,
        kind: CadArtifactKind,
        format: &str,
        contents_bytes: &[u8],
        metadata: Option<Value>,
    ) -> Result<CadArtifact, String> {
        let id = uuid();
        let (session_id, path, relative_path) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, revision_id)?;
            (
                revision.session_id.clone(),
                self.storage_layout
                    .artifact_path(&revision.session_id, revision_id, &id, format)
                    .map_err(|error| error.to_string())?,
                self.storage_layout
                    .artifact_relative_path(&revision.session_id, revision_id, &id, format)
                    .map_err(|error| error.to_string())?,
            )
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, contents_bytes).map_err(|error| error.to_string())?;
        let sha256 = storage::sha256_hex(contents_bytes);
        let mut metadata_map = metadata.map(metadata_from_value).unwrap_or_default();
        metadata_map.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().to_string()),
        );
        metadata_map.insert(
            "relativePath".to_string(),
            Value::String(relative_path.to_string_lossy().to_string()),
        );
        metadata_map.insert("sha256".to_string(), Value::String(sha256));
        let artifact = CadArtifact {
            id: id.clone(),
            revision_id: revision_id.to_string(),
            kind,
            format: format.to_string(),
            uri: format!("tauri://artifact/{id}"),
            bytes: Some(contents_bytes.len() as u64),
            created_at: timestamp(),
            deleted_at: None,
            missing_at: None,
            metadata: Some(metadata_map),
        };
        self.repository
            .save_artifact_manifest(&session_id, &artifact)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        state.artifacts.insert(id, artifact.clone());
        Ok(artifact)
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

    fn load_artifact_manifest(&self, artifact_id: &str) -> Result<CadArtifact, String> {
        if let Some(artifact) = self.repository.load_artifact_manifest(artifact_id)? {
            return Ok(artifact);
        }
        let state = self.inner.lock().map_err(lock_error)?;
        state
            .artifacts
            .get(artifact_id)
            .cloned()
            .ok_or_else(|| "Artifact not found.".to_string())
    }

    fn artifact_manifest_path(&self, artifact: &CadArtifact) -> Result<PathBuf, String> {
        let metadata = artifact
            .metadata
            .as_ref()
            .ok_or_else(|| "Artifact metadata missing.".to_string())?;
        if let Some(relative_path) = metadata.get("relativePath").and_then(Value::as_str) {
            let relative_path = PathBuf::from(relative_path);
            validate_artifact_relative_path(&relative_path)?;
            return Ok(self.storage_layout.app_data_dir().join(relative_path));
        }
        let path = metadata
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "Artifact path missing.".to_string())?;
        validate_artifact_absolute_path(&path, self.storage_layout.artifact_root())?;
        Ok(path)
    }

    fn artifact_session_id(&self, artifact: &CadArtifact) -> Result<String, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let revision = require_revision(&state, &artifact.revision_id)?;
        Ok(revision.session_id.clone())
    }

    fn mark_artifact_missing(
        &self,
        artifact_id: &str,
        missing_at: Option<String>,
    ) -> Result<(), String> {
        self.repository
            .set_artifact_missing_at(artifact_id, missing_at.as_deref())?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        if let Some(artifact) = state.artifacts.get_mut(artifact_id) {
            artifact.missing_at = missing_at.clone();
        }
        for revision in state.revisions.values_mut() {
            for artifact in &mut revision.artifacts {
                if artifact.id == artifact_id {
                    artifact.missing_at = missing_at.clone();
                }
            }
        }
        Ok(())
    }

    fn verify_artifact_files_inner(
        &self,
        session_id: Option<&str>,
    ) -> Result<VerifyArtifactFilesResult, String> {
        let (artifacts, recovery_diagnostics) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let artifacts = state
                .artifacts
                .values()
                .filter(|artifact| artifact.deleted_at.is_none())
                .filter(|artifact| {
                    session_id.is_none_or(|session_id| {
                        state
                            .revisions
                            .get(&artifact.revision_id)
                            .is_some_and(|revision| revision.session_id == session_id)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut recovery_diagnostics = state
                .sessions
                .values()
                .filter(|session| {
                    session_id
                        .map(|session_id| session.id == session_id)
                        .unwrap_or(true)
                })
                .flat_map(|session| session.recovery_diagnostics.clone())
                .collect::<Vec<_>>();
            recovery_diagnostics.extend(
                state
                    .revisions
                    .values()
                    .filter(|revision| {
                        session_id
                            .map(|session_id| revision.session_id == session_id)
                            .unwrap_or(true)
                    })
                    .flat_map(|revision| {
                        revision
                            .diagnostics
                            .items
                            .iter()
                            .filter(|diagnostic| diagnostic.message.contains("persisted"))
                            .cloned()
                    }),
            );
            (artifacts, recovery_diagnostics)
        };
        let mut missing_artifact_ids = Vec::new();
        let mut hash_mismatch_artifact_ids = Vec::new();
        let mut size_mismatch_artifact_ids = Vec::new();
        let mut corrupt_metadata_artifact_ids = Vec::new();
        let mut invalid_path_artifact_ids = Vec::new();
        let mut diagnostics = recovery_diagnostics;
        let mut known_paths = std::collections::HashSet::new();

        for artifact in &artifacts {
            let metadata = artifact.metadata.as_ref();
            if metadata
                .and_then(|metadata| metadata.get("metadataRecovery"))
                .is_some()
            {
                corrupt_metadata_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "warning",
                    format!("Artifact {} has corrupt persisted metadata.", artifact.id),
                ));
            }

            let path = match self.artifact_manifest_path(artifact) {
                Ok(path) => path,
                Err(error) => {
                    invalid_path_artifact_ids.push(artifact.id.clone());
                    diagnostics.push(verify_diagnostic(
                        "error",
                        format!(
                            "Artifact {} has an invalid manifest path: {error}",
                            artifact.id
                        ),
                    ));
                    let missing_at = artifact.missing_at.clone().unwrap_or_else(timestamp);
                    if artifact.missing_at.as_deref() != Some(missing_at.as_str()) {
                        self.mark_artifact_missing(&artifact.id, Some(missing_at))?;
                    }
                    continue;
                }
            };
            known_paths.insert(path.clone());

            let missing_at = if path.exists() {
                None
            } else {
                let missing_at = artifact.missing_at.clone().unwrap_or_else(timestamp);
                missing_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} file is missing at {}.",
                        artifact.id,
                        path.display()
                    ),
                ));
                Some(missing_at)
            };
            if artifact.missing_at != missing_at {
                self.mark_artifact_missing(&artifact.id, missing_at)?;
            }
            if !path.exists() {
                continue;
            }

            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            if artifact.bytes != Some(bytes.len() as u64) {
                size_mismatch_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} size mismatch: manifest {:?}, file {} bytes.",
                        artifact.id,
                        artifact.bytes,
                        bytes.len()
                    ),
                ));
            }
            let actual_sha256 = storage::sha256_hex(&bytes);
            let expected_sha256 = metadata
                .and_then(|metadata| metadata.get("sha256"))
                .and_then(Value::as_str);
            if expected_sha256 != Some(actual_sha256.as_str()) {
                hash_mismatch_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} sha256 does not match its manifest.",
                        artifact.id
                    ),
                ));
            }
        }
        let mut orphan_paths = Vec::new();
        for file_path in collect_artifact_files(self.storage_layout.artifact_root())? {
            if known_paths.contains(&file_path) {
                continue;
            }
            let path = file_path.to_string_lossy().to_string();
            diagnostics.push(verify_diagnostic(
                "warning",
                format!("Found artifact file without a SQLite manifest: {path}."),
            ));
            orphan_paths.push(path);
        }
        Ok(VerifyArtifactFilesResult {
            checked_count: artifacts.len(),
            missing_artifact_ids,
            hash_mismatch_artifact_ids,
            size_mismatch_artifact_ids,
            corrupt_metadata_artifact_ids,
            invalid_path_artifact_ids,
            orphan_paths,
            diagnostics,
            state: None,
        })
    }
}

pub fn metadata_from_value(value: Value) -> Metadata {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn collect_artifact_files(root: &std::path::Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_artifact_files(&path)?);
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(files)
}

fn validate_artifact_relative_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || !path.starts_with(storage::ARTIFACT_DIR_NAME)
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "Artifact relativePath escapes artifact root: {path:?}"
        ));
    }
    Ok(())
}

fn validate_artifact_absolute_path(path: &Path, artifact_root: &Path) -> Result<(), String> {
    if !path.starts_with(artifact_root) {
        return Err(format!("Artifact path is outside artifact root: {path:?}"));
    }
    Ok(())
}

fn verify_diagnostic(severity: &str, message: String) -> CadDiagnostic {
    CadDiagnostic {
        severity: severity.to_string(),
        message,
        line: None,
        column: None,
    }
}

fn add_user_event(revision: &mut CadRevision, event_type: &str, payload: Value) -> CadUserEvent {
    let event = CadUserEvent {
        id: uuid(),
        revision_id: revision.id.clone(),
        event_type: event_type.to_string(),
        created_at: timestamp(),
        payload: metadata_from_value(payload),
    };
    revision.user_events.push(event.clone());
    event
}

fn append_agent_run_event(
    state: &mut ServiceState,
    session_id: &str,
    run_id: &str,
    revision_id: Option<String>,
    event_type: CadAgentRunEventType,
    payload: Value,
    metadata: Option<Metadata>,
) -> CadAgentRunEvent {
    let events = state
        .agent_run_events
        .entry(session_id.to_string())
        .or_default();
    let sequence = events
        .iter()
        .filter(|event| event.run_id == run_id)
        .map(|event| event.sequence)
        .max()
        .unwrap_or(0)
        + 1;
    let event = CadAgentRunEvent {
        id: uuid(),
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        revision_id,
        event_type,
        sequence,
        created_at: timestamp(),
        payload: metadata_from_value(payload),
        metadata,
    };
    events.push(event.clone());
    event
}

fn persist_agent_run_event(
    repository: &dyn SessionRepository,
    state: &mut ServiceState,
    session_id: &str,
    event: CadAgentRunEvent,
) -> Result<CadAgentRunEvent, String> {
    let saved = repository.save_agent_run_event(&event)?;
    if saved.sequence != event.sequence {
        if let Some(events) = state.agent_run_events.get_mut(session_id) {
            if let Some(existing) = events.iter_mut().find(|candidate| candidate.id == saved.id) {
                *existing = saved.clone();
            }
            events.sort_by(|left, right| {
                left.run_id
                    .cmp(&right.run_id)
                    .then_with(|| left.sequence.cmp(&right.sequence))
            });
        }
    }
    Ok(saved)
}

fn rebuild_revision_summaries(state: &mut ServiceState, session_id: &str) {
    let mut summaries: Vec<CadRevisionSummary> = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| revision_summary(state, revision))
        .collect();
    summaries.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    if let Some(session) = state.sessions.get_mut(session_id) {
        session.revisions = summaries;
    }
}

fn build_state(state: &ServiceState, session_id: &str) -> Result<CadSessionState, String> {
    let session = require_session(state, session_id)?;
    let mut session = session.clone();
    session.revisions = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| revision_summary(state, revision))
        .collect();
    session
        .revisions
        .sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let active_revision = session
        .active_revision_id
        .as_ref()
        .and_then(|revision_id| state.revisions.get(revision_id))
        .map(|revision| revision_with_derived_fields(state, revision));
    Ok(CadSessionState {
        session,
        active_revision,
        messages: state.messages.get(session_id).cloned().unwrap_or_default(),
        conversation: state
            .conversation
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        agent_runs: state
            .agent_runs
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        agent_run_events: state
            .agent_run_events
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        workflow: build_workflow_state(state, session_id)?,
    })
}

fn build_workflow_state(
    state: &ServiceState,
    session_id: &str,
) -> Result<CadWorkflowState, String> {
    require_session(state, session_id)?;
    let run_ids = state
        .agent_runs
        .get(session_id)
        .into_iter()
        .flatten()
        .map(|run| run.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut plans = run_ids
        .iter()
        .filter_map(|run_id| state.workflow_plans.get(*run_id).cloned())
        .collect::<Vec<_>>();
    plans.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let mut outer_iterations = run_ids
        .iter()
        .flat_map(|run_id| {
            state
                .workflow_outer_iterations
                .get(*run_id)
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect::<Vec<_>>();
    outer_iterations.sort_by(|left, right| {
        left.run_id
            .cmp(&right.run_id)
            .then_with(|| left.iteration.cmp(&right.iteration))
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    let mut pending_vlm = run_ids
        .iter()
        .filter_map(|run_id| state.workflow_pending_vlm.get(*run_id).cloned())
        .collect::<Vec<_>>();
    pending_vlm.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(CadWorkflowState {
        plans,
        outer_iterations,
        pending_vlm,
    })
}

fn revision_summary(state: &ServiceState, revision: &CadRevision) -> CadRevisionSummary {
    CadRevisionSummary {
        id: revision.id.clone(),
        source_hash: source_hash(&revision.source),
        parent_revision_id: revision.parent_revision_id.clone(),
        restored_from_revision_id: revision.restored_from_revision_id.clone(),
        source_language: revision.source_language.clone(),
        created_at: revision.created_at.clone(),
        diagnostics: revision.diagnostics.clone(),
        artifact_count: revision.artifact_count,
        run_links: revision_run_links(state, &revision.session_id, &revision.id),
    }
}

fn revision_with_derived_fields(state: &ServiceState, revision: &CadRevision) -> CadRevision {
    let mut revision = revision.clone();
    revision.source_hash = source_hash(&revision.source);
    revision.run_links = revision_run_links(state, &revision.session_id, &revision.id);
    revision
}

fn session_list_item(state: &ServiceState, session: &CadSession) -> CadSessionListItem {
    let active_revision = session
        .active_revision_id
        .as_ref()
        .and_then(|revision_id| state.revisions.get(revision_id))
        .map(|revision| revision_summary(state, revision));
    let session_revisions = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session.id);
    let mut revision_count = 0;
    let mut artifact_count = 0;
    for revision in session_revisions {
        revision_count += 1;
        artifact_count += revision.artifact_count;
    }
    CadSessionListItem {
        id: session.id.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        last_viewed_at: session.last_viewed_at.clone(),
        title: session.title.clone(),
        active_revision_id: session.active_revision_id.clone(),
        active_revision,
        selected_runtime: session.selected_runtime.clone(),
        status: session.status.clone(),
        archived: session.archived_at.is_some(),
        archived_at: session.archived_at.clone(),
        revision_count,
        artifact_count,
    }
}

fn normalized_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(|query| query.to_lowercase())
}

fn session_matches_search(state: &ServiceState, session: &CadSession, query: &str) -> bool {
    session
        .title
        .as_deref()
        .is_some_and(|title| title.to_lowercase().contains(query))
        || state
            .revisions
            .values()
            .filter(|revision| revision.session_id == session.id)
            .any(|revision| revision.source.to_lowercase().contains(query))
        || state
            .conversation
            .get(&session.id)
            .into_iter()
            .flatten()
            .any(|message| message.content.to_lowercase().contains(query))
}

fn revision_run_links(
    state: &ServiceState,
    session_id: &str,
    revision_id: &str,
) -> Vec<CadRevisionRunLink> {
    let mut links = Vec::new();
    for run in state.agent_runs.get(session_id).into_iter().flatten() {
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

fn source_hash(source: &str) -> String {
    storage::sha256_hex(source.as_bytes())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenscadWasmNodeOutput {
    diagnostics: CadDiagnostics,
    #[serde(default)]
    mesh: Option<CadMesh>,
    #[serde(default)]
    stl_base64: Option<String>,
    #[serde(default)]
    stl_sha256: Option<String>,
    #[serde(default)]
    stl_bytes: Option<u64>,
}

fn render_open_scad_wasm_node(
    source: &str,
    app_data_dir: &Path,
) -> Result<OpenscadWasmNodeOutput, String> {
    fs::create_dir_all(app_data_dir).map_err(|error| error.to_string())?;
    let source_path = app_data_dir.join(format!("openscad-render-{}.scad", Uuid::new_v4()));
    fs::write(&source_path, source).map_err(|error| error.to_string())?;
    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| "Could not resolve repository root for OpenSCAD WASM helper.".to_string())?
        .join("scripts")
        .join("openscad-render.mjs");
    let output = Command::new("node")
        .arg(&script_path)
        .arg(&source_path)
        .output()
        .map_err(|error| {
            format!(
                "Failed to execute OpenSCAD WASM helper {}: {error}",
                script_path.display()
            )
        })?;
    let _ = fs::remove_file(&source_path);
    if !output.status.success() {
        return Err(format!(
            "OpenSCAD WASM helper exited with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "OpenSCAD WASM helper returned invalid JSON: {error}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn runtime_artifact_metadata(
    source: &str,
    parameters: &[CadParameter],
    rendered: &OpenscadWasmNodeOutput,
    phase: &str,
) -> Result<Value, String> {
    Ok(json!({
        "runtime": "openscad-wasm",
        "sourceLanguage": "openscad",
        "sourceHash": source_hash(source),
        "parameterHash": storage::sha256_hex(
            serde_json::to_string(parameters)
                .map_err(|error| error.to_string())?
                .as_bytes()
        ),
        "stlSha256": rendered.stl_sha256,
        "stlBytes": rendered.stl_bytes,
        "renderDurationMs": rendered.diagnostics.elapsed_ms,
        "diagnosticsSource": "openscad-wasm",
        "phase": phase
    }))
}

fn require_session<'a>(
    state: &'a ServiceState,
    session_id: &str,
) -> Result<&'a CadSession, String> {
    state
        .sessions
        .get(session_id)
        .ok_or_else(|| format!("CAD session is missing or has been deleted: {session_id}"))
}

fn require_session_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
) -> Result<&'a mut CadSession, String> {
    state
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("CAD session is missing or has been deleted: {session_id}"))
}

fn require_revision<'a>(
    state: &'a ServiceState,
    revision_id: &str,
) -> Result<&'a CadRevision, String> {
    state
        .revisions
        .get(revision_id)
        .ok_or_else(|| format!("CAD revision not found: {revision_id}"))
}

fn require_revision_mut<'a>(
    state: &'a mut ServiceState,
    revision_id: &str,
) -> Result<&'a mut CadRevision, String> {
    state
        .revisions
        .get_mut(revision_id)
        .ok_or_else(|| format!("CAD revision not found: {revision_id}"))
}

fn require_agent_run_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
    run_id: &str,
) -> Result<&'a mut CadAgentRun, String> {
    state
        .agent_runs
        .get_mut(session_id)
        .and_then(|runs| runs.iter_mut().find(|run| run.id == run_id))
        .ok_or_else(|| format!("Agent run not found: {run_id}"))
}

fn validate_workflow_run(
    state: &ServiceState,
    session_id: &str,
    run_id: &str,
) -> Result<(), String> {
    require_session(state, session_id)?;
    state
        .agent_runs
        .get(session_id)
        .into_iter()
        .flatten()
        .find(|run| run.id == run_id)
        .map(|_| ())
        .ok_or_else(|| format!("Agent run not found: {run_id}"))
}

fn validate_revision_session(
    state: &ServiceState,
    session_id: &str,
    revision_id: &str,
) -> Result<(), String> {
    let revision = require_revision(state, revision_id)?;
    if revision.session_id != session_id {
        return Err(format!(
            "CAD revision {revision_id} does not belong to session {session_id}."
        ));
    }
    Ok(())
}

fn validate_artifact_session(
    state: &ServiceState,
    session_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    let artifact = state
        .artifacts
        .get(artifact_id)
        .ok_or_else(|| format!("CAD artifact not found: {artifact_id}"))?;
    validate_revision_session(state, session_id, &artifact.revision_id)
}

fn json_to_parameter_value(value: Value) -> CadParameterValue {
    match value {
        Value::Bool(value) => CadParameterValue::Boolean(value),
        Value::Number(value) => CadParameterValue::Number(value.as_f64().unwrap_or_default()),
        Value::String(value) => CadParameterValue::String(value),
        other => CadParameterValue::String(other.to_string()),
    }
}

fn event_type_for_run_status(status: &CadAgentRunStatus) -> CadBridgeEventType {
    match status {
        CadAgentRunStatus::Completed => CadBridgeEventType::AgentRunCompleted,
        CadAgentRunStatus::Failed => CadBridgeEventType::AgentRunFailed,
        CadAgentRunStatus::Queued => CadBridgeEventType::AgentRunCreated,
        _ => CadBridgeEventType::AgentRunUpdated,
    }
}

fn run_event_type_for_update(
    bridge_event_type: Option<&CadBridgeEventType>,
    status: &CadAgentRunStatus,
) -> CadAgentRunEventType {
    match bridge_event_type {
        Some(CadBridgeEventType::AgentMessageCreated) => CadAgentRunEventType::AgentMessageCreated,
        Some(CadBridgeEventType::AgentToolStarted) => CadAgentRunEventType::AgentToolStarted,
        Some(CadBridgeEventType::AgentToolCompleted) => CadAgentRunEventType::AgentToolCompleted,
        Some(CadBridgeEventType::AgentRunCompleted) => CadAgentRunEventType::AgentRunCompleted,
        Some(CadBridgeEventType::AgentRunFailed) => CadAgentRunEventType::AgentRunFailed,
        _ => match status {
            CadAgentRunStatus::Completed => CadAgentRunEventType::AgentRunCompleted,
            CadAgentRunStatus::Failed => CadAgentRunEventType::AgentRunFailed,
            CadAgentRunStatus::Cancelled => CadAgentRunEventType::AgentRunCancelled,
            CadAgentRunStatus::Queued => CadAgentRunEventType::AgentRunCreated,
            _ => CadAgentRunEventType::AgentRunUpdated,
        },
    }
}

fn is_terminal_run_status(status: &CadAgentRunStatus) -> bool {
    matches!(
        status,
        CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
    )
}

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

fn timestamp() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}.{:03}Z", chrono_like_seconds(millis), millis % 1000)
}

fn chrono_like_seconds(millis: u128) -> String {
    let seconds = millis / 1000;
    let tm = time_from_unix(seconds as i64);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.year, tm.month, tm.day, tm.hour, tm.minute, tm.second
    )
}

struct SimpleUtcTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn time_from_unix(seconds: i64) -> SimpleUtcTime {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    SimpleUtcTime {
        year,
        month,
        day,
        hour: seconds_of_day / 3600,
        minute: seconds_of_day % 3600 / 60,
        second: seconds_of_day % 60,
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "Session service lock is poisoned.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_repository::SqliteSessionRepository;

    #[test]
    fn serializes_camel_case_state() {
        let service =
            SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let value = serde_json::to_value(created.state).unwrap();
        assert!(value["session"]["activeRevisionId"].is_string());
        assert!(value["activeRevision"]["sourceLanguage"].is_string());
    }

    #[test]
    fn render_preview_uses_active_revision_source() {
        let service =
            SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let updated = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source:
                    "radius = 6; // @param min=1 max=20 step=1 label=Radius\nsphere(r = radius);"
                        .to_string(),
                parent_revision_id: created.state.session.active_revision_id.clone(),
                parameters: None,
            })
            .unwrap();

        let (preview, state) = service
            .render_preview(RenderPreviewInput {
                session_id: created.session_id,
                revision_id: Some(updated.revision_id),
            })
            .unwrap();

        assert!(preview.diagnostics.ok);
        let mesh = preview.mesh.unwrap();
        assert!(mesh.vertices.len() / 3 > 100);
        assert!(state
            .active_revision
            .unwrap()
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == CadArtifactKind::PreviewMesh));
    }

    #[test]
    fn openscad_wasm_preview_and_export_share_boolean_stl_output_hash() {
        let service =
            SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let updated = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: r#"
$fn = 24;
difference() {
  union() {
    cube([12, 8, 4], center=true);
    rotate([0, 0, 30]) translate([0, 0, 3]) cylinder(h=4, r=3, center=true);
  }
  translate([0, 0, -4]) cylinder(h=12, r=1.5, center=true);
}
"#
                .to_string(),
                parent_revision_id: created.state.session.active_revision_id.clone(),
                parameters: None,
            })
            .unwrap();

        let (preview, state) = service
            .render_preview(RenderPreviewInput {
                session_id: created.session_id.clone(),
                revision_id: Some(updated.revision_id.clone()),
            })
            .unwrap();
        assert!(preview.diagnostics.ok);
        let preview_stl_hash = state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == CadArtifactKind::PreviewMesh)
            .and_then(|artifact| artifact.metadata.as_ref())
            .and_then(|metadata| metadata.get("stlSha256"))
            .and_then(Value::as_str)
            .unwrap()
            .to_string();

        let (export, exported_state) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id,
                revision_id: Some(updated.revision_id),
                format: "stl".to_string(),
            })
            .unwrap();
        assert!(export.diagnostics.ok);
        let export_stl_hash = export
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.metadata.as_ref())
            .and_then(|metadata| metadata.get("stlSha256"))
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(preview_stl_hash, export_stl_hash);
        assert_eq!(
            exported_state
                .active_revision
                .as_ref()
                .unwrap()
                .artifacts
                .iter()
                .filter(|artifact| artifact.kind == CadArtifactKind::Stl)
                .count(),
            1
        );
    }

    #[test]
    fn revision_switch_restore_and_parameters_use_immutable_snapshots() {
        let service =
            SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
        let parameterized = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source:
                    "radius = 4; // @param min=1 max=20 step=1 label=Radius\nsphere(r = radius);"
                        .to_string(),
                parent_revision_id: Some(root_revision_id.clone()),
                parameters: None,
            })
            .unwrap();
        let parameterized_revision_id = parameterized.revision_id.clone();

        let parameter_update = service
            .update_parameters(
                &created.session_id,
                metadata_from_value(json!({ "radius": 9 })),
            )
            .unwrap();
        let parameter_revision = parameter_update.active_revision.as_ref().unwrap();
        assert_ne!(parameter_revision.id, parameterized_revision_id);
        assert_eq!(
            parameter_revision.parent_revision_id.as_deref(),
            Some(parameterized_revision_id.as_str())
        );
        assert_eq!(
            service
                .get_session_state(&created.session_id)
                .unwrap()
                .session
                .revisions
                .len(),
            3
        );

        let switched = service
            .set_active_revision(SetActiveRevisionInput {
                session_id: created.session_id.clone(),
                revision_id: root_revision_id.clone(),
            })
            .unwrap();
        assert_eq!(
            switched.session.active_revision_id.as_deref(),
            Some(root_revision_id.as_str())
        );

        let restored = service
            .restore_revision(RestoreRevisionInput {
                session_id: created.session_id.clone(),
                revision_id: parameterized_revision_id.clone(),
            })
            .unwrap();
        let restored_revision = restored.state.active_revision.as_ref().unwrap();
        assert_eq!(
            restored_revision.parent_revision_id.as_deref(),
            Some(root_revision_id.as_str())
        );
        assert_eq!(
            restored_revision.restored_from_revision_id.as_deref(),
            Some(parameterized_revision_id.as_str())
        );
        assert_eq!(restored_revision.source_hash.len(), 64);
        assert_eq!(restored_revision.artifact_count, 0);
    }

    #[test]
    fn export_artifact_uses_session_revision_artifact_layout() {
        let artifact_root = std::env::temp_dir()
            .join(format!("cadastrophe-test-{}", uuid()))
            .join("artifacts");
        let service = SessionService::new(artifact_root.clone());
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let revision_id = created
            .state
            .session
            .active_revision_id
            .clone()
            .expect("session has initial revision");

        let (export, _) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: Some(revision_id.clone()),
                format: "stl".to_string(),
            })
            .unwrap();

        let artifact = export.artifact.expect("artifact exported");
        let metadata = artifact.metadata.as_ref().expect("artifact metadata");
        let path = PathBuf::from(metadata["path"].as_str().expect("path metadata"));
        assert_eq!(
            path,
            artifact_root
                .join(&created.session_id)
                .join(&revision_id)
                .join(format!("{}.stl", artifact.id))
        );
        assert_eq!(
            metadata["relativePath"].as_str(),
            Some(
                PathBuf::from("artifacts")
                    .join(&created.session_id)
                    .join(&revision_id)
                    .join(format!("{}.stl", artifact.id))
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(path.exists());
        assert_eq!(artifact.bytes, Some(fs::metadata(path).unwrap().len()));
        assert_eq!(
            metadata["sha256"].as_str().map(str::len),
            Some(64),
            "sha256 metadata should be stored as hex"
        );
    }

    #[test]
    fn sqlite_repository_restores_artifact_manifest_after_restart() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-artifact-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        service.mark_session_viewed(&created.session_id).unwrap();
        let revision_id = created.state.session.active_revision_id.clone().unwrap();
        let (export, _) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: Some(revision_id),
                format: "stl".to_string(),
            })
            .unwrap();
        let artifact = export.artifact.unwrap();
        let original_contents = service.read_artifact(&artifact.id).unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();
        let restored_artifact = state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .find(|candidate| candidate.id == artifact.id)
            .expect("artifact manifest restored");
        assert_eq!(restored_artifact.kind, CadArtifactKind::Stl);
        assert_eq!(
            reloaded.read_artifact(&artifact.id).unwrap(),
            original_contents
        );
        let opened = reloaded.open_artifact(&artifact.id).unwrap();
        assert!(PathBuf::from(&opened.path).exists());

        let deleted = reloaded
            .delete_artifact(DeleteArtifactInput {
                session_id: created.session_id.clone(),
                artifact_id: artifact.id.clone(),
            })
            .unwrap();
        assert!(!PathBuf::from(opened.path).exists());
        assert!(!deleted
            .state
            .active_revision
            .unwrap()
            .artifacts
            .iter()
            .any(|candidate| candidate.id == artifact.id));

        let reloaded_after_delete = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state_after_delete = reloaded_after_delete
            .get_session_state(&created.session_id)
            .unwrap();
        assert!(!state_after_delete
            .active_revision
            .unwrap()
            .artifacts
            .iter()
            .any(|candidate| candidate.id == artifact.id));
        assert!(reloaded_after_delete.read_artifact(&artifact.id).is_err());
    }

    #[test]
    fn sqlite_repository_marks_missing_artifacts_on_startup_and_verify() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-artifact-missing-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let (export, _) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: created.state.session.active_revision_id.clone(),
                format: "metadata".to_string(),
            })
            .unwrap();
        let artifact = export.artifact.unwrap();
        let path = service.open_artifact(&artifact.id).unwrap().path;
        fs::remove_file(&path).unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();
        let missing_artifact = state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .find(|candidate| candidate.id == artifact.id)
            .expect("missing artifact remains visible");
        assert!(missing_artifact.missing_at.is_some());
        assert!(reloaded.read_artifact(&artifact.id).is_err());

        let verified = reloaded
            .verify_artifact_files(Some(created.session_id.clone()))
            .unwrap();
        assert_eq!(verified.checked_count, 1);
        assert_eq!(verified.missing_artifact_ids, vec![artifact.id]);
        assert!(verified
            .state
            .unwrap()
            .active_revision
            .unwrap()
            .artifacts
            .iter()
            .any(|candidate| candidate.missing_at.is_some()));
    }

    #[test]
    fn sqlite_repository_restores_current_session_and_session_index() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput {
                title: Some("Original title".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        service.mark_session_viewed(&created.session_id).unwrap();
        service
            .rename_session(RenameCadSessionInput {
                session_id: created.session_id.clone(),
                title: "Persisted title".to_string(),
            })
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let current = reloaded.get_current_session().unwrap();

        assert_eq!(
            current.session_id.as_deref(),
            Some(created.session_id.as_str())
        );
        let state = current.state.expect("current session state");
        assert_eq!(state.session.title.as_deref(), Some("Persisted title"));
        assert_eq!(
            state
                .active_revision
                .as_ref()
                .map(|revision| revision.source.as_str()),
            Some(DEFAULT_SAMPLE_SOURCE)
        );
        assert_eq!(reloaded.list_sessions(false).unwrap().len(), 1);
    }

    #[test]
    fn session_list_returns_active_revision_summary_and_searches_title_source_conversation() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-session-list-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let bracket = service
            .create_session(CreateCadSessionInput {
                title: Some("Bracket Assembly".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        service
            .update_model_source(UpdateModelSourceInput {
                session_id: bracket.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source:
                    "// mounting_slot_fixture\ndifference() { cube([8, 8, 2]); cylinder(r = 2); }"
                        .to_string(),
                parent_revision_id: bracket.state.session.active_revision_id.clone(),
                parameters: None,
            })
            .unwrap();
        service
            .post_user_message(PostUserMessageInput {
                session_id: bracket.session_id.clone(),
                revision_id: None,
                message: "Needs a mounting tab".to_string(),
            })
            .unwrap();
        let _other = service
            .create_session(CreateCadSessionInput {
                title: Some("Plain block".to_string()),
                selected_runtime: None,
            })
            .unwrap();

        let listed = service
            .list_sessions_for_input(ListCadSessionsInput {
                include_archived: false,
                query: Some("mounting_slot_fixture".to_string()),
            })
            .unwrap();
        assert_eq!(
            listed.search_fields,
            vec!["title", "source", "conversation"]
        );
        assert_eq!(listed.sessions.len(), 1);
        assert_eq!(listed.sessions[0].id, bracket.session_id);
        assert_eq!(
            listed.sessions[0].title.as_deref(),
            Some("Bracket Assembly")
        );
        assert!(listed.sessions[0].active_revision.is_some());
        assert_eq!(listed.sessions[0].revision_count, 2);
        assert_eq!(listed.sessions[0].archived, false);

        let conversation_match = service
            .list_sessions_for_input(ListCadSessionsInput {
                include_archived: false,
                query: Some("mounting tab".to_string()),
            })
            .unwrap();
        assert_eq!(conversation_match.sessions.len(), 1);
        assert_eq!(conversation_match.sessions[0].id, bracket.session_id);
    }

    #[test]
    fn archived_sessions_open_readable_but_do_not_become_current_and_deleted_is_explicit_error() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-session-state-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let active = service
            .create_session(CreateCadSessionInput {
                title: Some("Active".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        let archived = service
            .create_session(CreateCadSessionInput {
                title: Some("Archived".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        service.mark_session_viewed(&active.session_id).unwrap();
        service
            .archive_session(ArchiveCadSessionInput {
                session_id: archived.session_id.clone(),
                archived: Some(true),
            })
            .unwrap();

        let archived_state = service.get_session_state(&archived.session_id).unwrap();
        assert!(archived_state.session.archived_at.is_some());
        service.mark_session_viewed(&archived.session_id).unwrap();
        assert_eq!(
            service.get_current_session().unwrap().session_id.as_deref(),
            Some(active.session_id.as_str())
        );

        service.delete_session(&archived.session_id).unwrap();
        let error = service
            .get_session_state(&archived.session_id)
            .expect_err("deleted session should not open");
        assert!(error.contains("missing or has been deleted"));
    }

    #[test]
    fn sqlite_repository_persists_duplicate_archive_and_delete() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput {
                title: Some("Original".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        let duplicated = service
            .duplicate_session(DuplicateCadSessionInput {
                session_id: created.session_id.clone(),
                title: Some("Copy".to_string()),
            })
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let sessions = reloaded.list_sessions(false).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .any(|session| session.title.as_deref() == Some("Copy")));
        assert_eq!(
            reloaded
                .get_session_state(&duplicated.session_id)
                .unwrap()
                .active_revision
                .as_ref()
                .map(|revision| revision.source.as_str()),
            Some(DEFAULT_SAMPLE_SOURCE)
        );

        reloaded
            .archive_session(ArchiveCadSessionInput {
                session_id: created.session_id.clone(),
                archived: None,
            })
            .unwrap();
        reloaded.delete_session(&duplicated.session_id).unwrap();

        let reloaded_again = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        assert!(reloaded_again
            .list_sessions(false)
            .unwrap()
            .iter()
            .all(|session| session.id != created.session_id));
        let archived_sessions = reloaded_again.list_sessions(true).unwrap();
        assert_eq!(archived_sessions.len(), 1);
        assert_eq!(archived_sessions[0].id, created.session_id);
        assert!(archived_sessions[0].archived_at.is_some());
        assert!(reloaded_again
            .get_session_state(&duplicated.session_id)
            .is_err());
    }

    #[test]
    fn sqlite_repository_persists_restore_summary_fields_and_artifact_count() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
        let updated = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "sphere(r = 8);".to_string(),
                parent_revision_id: Some(root_revision_id.clone()),
                parameters: None,
            })
            .unwrap();
        service
            .set_active_revision(SetActiveRevisionInput {
                session_id: created.session_id.clone(),
                revision_id: root_revision_id.clone(),
            })
            .unwrap();
        let restored = service
            .restore_revision(RestoreRevisionInput {
                session_id: created.session_id.clone(),
                revision_id: updated.revision_id.clone(),
            })
            .unwrap();
        service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: Some(restored.revision_id.clone()),
                format: "metadata".to_string(),
            })
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();
        let restored_summary = state
            .session
            .revisions
            .iter()
            .find(|revision| revision.id == restored.revision_id)
            .unwrap();
        assert_eq!(
            restored_summary.parent_revision_id.as_deref(),
            Some(root_revision_id.as_str())
        );
        assert_eq!(
            restored_summary.restored_from_revision_id.as_deref(),
            Some(updated.revision_id.as_str())
        );
        assert_eq!(restored_summary.source_hash.len(), 64);
        assert_eq!(restored_summary.artifact_count, 1);
    }

    #[test]
    fn sqlite_repository_restores_conversation_runs_and_run_events_after_restart() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-run-log-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let input_revision_id = created.state.session.active_revision_id.clone();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Create a persisted run log fixture.".to_string(),
                input_revision_id.clone(),
                Some("fake".to_string()),
                None,
            )
            .unwrap();
        service
            .create_conversation_message(
                &created.session_id,
                input_revision_id.clone(),
                CadConversationRole::Assistant,
                "I will update the model.".to_string(),
                Some(run.id.clone()),
                Some(metadata_from_value(json!({"source": "test"}))),
            )
            .unwrap();
        service
            .update_agent_run_external_metadata(
                &created.session_id,
                &run.id,
                Some("fake".to_string()),
                Some("thread-1".to_string()),
                Some("turn-1".to_string()),
            )
            .unwrap();
        service
            .update_agent_run(
                &created.session_id,
                &run.id,
                Some(CadAgentRunStatus::Running),
                Some(Some("generate_source".to_string())),
                None,
                Some(CadBridgeEventType::AgentToolStarted),
                Some(json!({"tool": "generate_source"})),
            )
            .unwrap();
        service
            .update_agent_run(
                &created.session_id,
                &run.id,
                None,
                Some(None),
                None,
                Some(CadBridgeEventType::AgentToolCompleted),
                Some(json!({"tool": "generate_source"})),
            )
            .unwrap();
        let output = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "sphere(r = 4);".to_string(),
                parent_revision_id: input_revision_id.clone(),
                parameters: None,
            })
            .unwrap();
        service
            .link_agent_run_output_revision(&created.session_id, &run.id, output.revision_id)
            .unwrap();
        service
            .update_agent_run(
                &created.session_id,
                &run.id,
                Some(CadAgentRunStatus::Completed),
                Some(None),
                None,
                Some(CadBridgeEventType::AgentRunCompleted),
                Some(json!({"status": "completed"})),
            )
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();
        assert!(state
            .conversation
            .iter()
            .any(|message| message.run_id.as_deref() == Some(run.id.as_str())
                && message.role == CadConversationRole::Assistant
                && message.content == "I will update the model."));
        let restored_run = state
            .agent_runs
            .iter()
            .find(|candidate| candidate.id == run.id)
            .expect("agent run restored");
        assert_eq!(restored_run.status, CadAgentRunStatus::Completed);
        assert_eq!(restored_run.external_agent.as_deref(), Some("fake"));
        assert_eq!(restored_run.external_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(restored_run.external_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(
            restored_run.input_revision_id.as_deref(),
            input_revision_id.as_deref()
        );
        assert!(restored_run.output_revision_id.is_some());
        let event_types = state
            .agent_run_events
            .iter()
            .filter(|event| event.run_id == run.id)
            .map(|event| event.event_type.clone())
            .collect::<Vec<_>>();
        assert!(event_types.contains(&CadAgentRunEventType::AgentRunCreated));
        assert!(event_types.contains(&CadAgentRunEventType::AgentMessageCreated));
        assert!(event_types.contains(&CadAgentRunEventType::AgentToolStarted));
        assert!(event_types.contains(&CadAgentRunEventType::AgentToolCompleted));
        assert!(event_types.contains(&CadAgentRunEventType::AgentRunCompleted));
        let generate_tool_event = state
            .agent_run_events
            .iter()
            .find(|event| event.event_type == CadAgentRunEventType::AgentToolStarted)
            .expect("tool event restored");
        assert_eq!(
            generate_tool_event
                .payload
                .get("tool")
                .and_then(Value::as_str),
            Some("generate_source")
        );
    }

    #[test]
    fn sqlite_repository_restores_workflow_state_after_restart() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-workflow-repo-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
        let updated = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "// @main_component wall_bracket\ncube([30, 10, 20]);".to_string(),
                parent_revision_id: Some(root_revision_id),
                parameters: None,
            })
            .unwrap();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Create a workflow state fixture.".to_string(),
                Some(updated.revision_id.clone()),
                Some("fake".to_string()),
                None,
            )
            .unwrap();
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap();
        let workflow_plan = CadWorkflowPlan {
            run_id: run.id.clone(),
            revision_id: Some(updated.revision_id.clone()),
            source_language: plan.source_language.clone(),
            plan,
            created_at: "2026-07-29T00:00:00.000Z".to_string(),
        };
        service
            .save_workflow_plan(&created.session_id, workflow_plan)
            .unwrap();
        service
            .save_workflow_outer_iteration(
                &created.session_id,
                CadWorkflowOuterIteration {
                    id: "workflow-outer-test-1".to_string(),
                    run_id: run.id.clone(),
                    iteration: 1,
                    revision_id: Some(updated.revision_id.clone()),
                    structural_report: serde_json::from_str(include_str!(
                        "../../fixtures/contracts/structural_report.v1.json"
                    ))
                    .unwrap(),
                    vlm_report: Some(
                        serde_json::from_str(include_str!(
                            "../../fixtures/contracts/vlm_judge_report.v1.json"
                        ))
                        .unwrap(),
                    ),
                    failure_report: Some(json!({
                        "contractType": "cadastrophe.failure_report.v1",
                        "reason": "missing_support_tab",
                        "nextAction": "outer_loop_refine_source"
                    })),
                    passed: false,
                    created_at: "2026-07-29T00:00:01.000Z".to_string(),
                },
            )
            .unwrap();
        let artifact = service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: updated.revision_id.clone(),
                kind: CadArtifactKind::Stl,
                format: "stl".to_string(),
                contents_base64: {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD
                        .encode(b"solid workflow_fixture\nendsolid workflow_fixture\n")
                },
                diagnostics: ok_diagnostics(1),
                metadata: metadata_from_value(json!({
                    "runtime": "openscad-wasm",
                    "sourceLanguage": "openscad"
                })),
            })
            .unwrap()
            .artifact;
        service
            .save_workflow_pending_vlm(
                &created.session_id,
                CadWorkflowPendingVlm {
                    run_id: run.id.clone(),
                    artifact_id: artifact.id.clone(),
                    contract: serde_json::from_str(include_str!(
                        "../../fixtures/contracts/vlm_judge_contract.v1.json"
                    ))
                    .unwrap(),
                    pass_threshold: 0.8,
                    created_at: "2026-07-29T00:00:02.000Z".to_string(),
                },
            )
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();

        assert_eq!(state.workflow.plans.len(), 1);
        assert_eq!(state.workflow.plans[0].run_id, run.id);
        assert_eq!(
            state.workflow.plans[0].revision_id.as_deref(),
            Some(updated.revision_id.as_str())
        );
        assert_eq!(
            state.workflow.plans[0].plan.main_component.name,
            "wall_bracket"
        );
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert_eq!(state.workflow.outer_iterations[0].iteration, 1);
        assert!(!state.workflow.outer_iterations[0].passed);
        assert_eq!(
            state.workflow.outer_iterations[0]
                .failure_report
                .as_ref()
                .and_then(|report| report.get("contractType"))
                .and_then(Value::as_str),
            Some("cadastrophe.failure_report.v1")
        );
        assert_eq!(state.workflow.pending_vlm.len(), 1);
        assert_eq!(state.workflow.pending_vlm[0].artifact_id, artifact.id);
        assert_eq!(state.workflow.pending_vlm[0].pass_threshold, 0.8);
    }

    #[test]
    fn sqlite_repository_assigns_agent_event_sequence_from_database() {
        let app_data_dir = std::env::temp_dir().join(format!(
            "cadastrophe-run-event-sequence-race-test-{}",
            uuid()
        ));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let app_service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = app_service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let input_revision_id = created.state.session.active_revision_id.clone();
        let (run, _) = app_service
            .create_agent_run(
                &created.session_id,
                "Create a stale writer race fixture.".to_string(),
                input_revision_id,
                Some("codex".to_string()),
                None,
            )
            .unwrap();

        let stale_cli_service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        app_service
            .update_agent_run(
                &created.session_id,
                &run.id,
                None,
                Some(Some("cadastrophe-plan-commit".to_string())),
                None,
                Some(CadBridgeEventType::AgentToolStarted),
                Some(json!({"tool": "cadastrophe-plan-commit"})),
            )
            .unwrap();
        let cli_event = stale_cli_service
            .record_agent_tool_event(
                &created.session_id,
                &run.id,
                None,
                CadAgentRunEventType::AgentToolStarted,
                json!({"command": "cadastrophe-plan-commit", "status": "started"}),
            )
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let events = reloaded
            .get_session_state(&created.session_id)
            .unwrap()
            .agent_run_events
            .into_iter()
            .filter(|event| event.run_id == run.id)
            .collect::<Vec<_>>();
        let sequences = events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>();
        assert_eq!(sequences, vec![1, 2, 3]);
        assert_eq!(cli_event.sequence, 3);
    }

    #[test]
    fn refresh_session_from_repository_merges_external_workflow_state() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-workflow-refresh-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let app_service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = app_service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let (run, _) = app_service
            .create_agent_run(
                &created.session_id,
                "Create a workflow refresh fixture.".to_string(),
                created.state.session.active_revision_id.clone(),
                Some("codex".to_string()),
                None,
            )
            .unwrap();
        let external_cli_service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap();
        external_cli_service
            .save_workflow_plan(
                &created.session_id,
                CadWorkflowPlan {
                    run_id: run.id.clone(),
                    revision_id: created.state.session.active_revision_id.clone(),
                    source_language: plan.source_language.clone(),
                    plan,
                    created_at: "2026-07-29T00:00:00.000Z".to_string(),
                },
            )
            .unwrap();

        assert!(app_service
            .get_session_state(&created.session_id)
            .unwrap()
            .workflow
            .plans
            .is_empty());

        let refreshed = app_service
            .refresh_session_from_repository(&created.session_id)
            .unwrap();
        assert_eq!(refreshed.workflow.plans.len(), 1);
        assert_eq!(refreshed.workflow.plans[0].run_id, run.id);
        assert_eq!(
            refreshed.workflow.plans[0].plan.main_component.name,
            "wall_bracket"
        );
    }

    #[test]
    fn workflow_service_rejects_cross_session_and_missing_references() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-workflow-integrity-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let first = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let second = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let first_revision_id = first.state.session.active_revision_id.clone().unwrap();
        let second_revision_id = second.state.session.active_revision_id.clone().unwrap();
        let (run, _) = service
            .create_agent_run(
                &first.session_id,
                "Validate workflow references.".to_string(),
                Some(first_revision_id),
                Some("fake".to_string()),
                None,
            )
            .unwrap();
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap();
        let error = service
            .save_workflow_plan(
                &first.session_id,
                CadWorkflowPlan {
                    run_id: run.id.clone(),
                    revision_id: Some(second_revision_id),
                    source_language: plan.source_language.clone(),
                    plan,
                    created_at: "2026-07-29T00:00:00.000Z".to_string(),
                },
            )
            .expect_err("cross-session revision should be rejected");
        assert!(error.contains("does not belong to session"));

        let error = service
            .save_workflow_pending_vlm(
                &first.session_id,
                CadWorkflowPendingVlm {
                    run_id: run.id,
                    artifact_id: "missing-artifact".to_string(),
                    contract: json!({"contractType": "cadastrophe.vlm_judge.v1"}),
                    pass_threshold: 0.8,
                    created_at: "2026-07-29T00:00:02.000Z".to_string(),
                },
            )
            .expect_err("missing artifact should be rejected");
        assert!(error.contains("CAD artifact not found"));
    }

    #[test]
    fn sqlite_restart_restores_session_revision_artifacts_conversation_and_runs_together() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-restart-integrity-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput {
                title: Some("Restart fixture".to_string()),
                selected_runtime: None,
            })
            .unwrap();
        service.mark_session_viewed(&created.session_id).unwrap();
        let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
        let updated = service
            .update_model_source(UpdateModelSourceInput {
                session_id: created.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "sphere(r = 5);".to_string(),
                parent_revision_id: Some(root_revision_id.clone()),
                parameters: None,
            })
            .unwrap();
        let (export, _) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: Some(updated.revision_id.clone()),
                format: "metadata".to_string(),
            })
            .unwrap();
        let artifact = export.artifact.unwrap();
        let (run, _) = service
            .create_agent_run(
                &created.session_id,
                "Persist all restart surfaces.".to_string(),
                Some(updated.revision_id.clone()),
                Some("fake".to_string()),
                None,
            )
            .unwrap();
        service
            .create_conversation_message(
                &created.session_id,
                Some(updated.revision_id.clone()),
                CadConversationRole::Assistant,
                "Restart state is durable.".to_string(),
                Some(run.id.clone()),
                None,
            )
            .unwrap();
        service
            .link_agent_run_output_revision(
                &created.session_id,
                &run.id,
                updated.revision_id.clone(),
            )
            .unwrap();
        service
            .update_agent_run(
                &created.session_id,
                &run.id,
                Some(CadAgentRunStatus::Completed),
                Some(None),
                None,
                Some(CadBridgeEventType::AgentRunCompleted),
                Some(json!({"status": "completed"})),
            )
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let current = reloaded.get_current_session().unwrap();
        assert_eq!(
            current.session_id.as_deref(),
            Some(created.session_id.as_str())
        );
        let state = current.state.unwrap();
        assert_eq!(state.session.title.as_deref(), Some("Restart fixture"));
        assert_eq!(
            state.session.active_revision_id.as_deref(),
            Some(updated.revision_id.as_str())
        );
        assert_eq!(
            state
                .active_revision
                .as_ref()
                .map(|revision| revision.source.as_str()),
            Some("sphere(r = 5);")
        );
        assert!(state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .any(|candidate| candidate.id == artifact.id));
        assert!(state
            .conversation
            .iter()
            .any(|message| message.content == "Restart state is durable."));
        assert!(state
            .agent_runs
            .iter()
            .any(|candidate| candidate.id == run.id
                && candidate.status == CadAgentRunStatus::Completed
                && candidate.output_revision_id.as_deref()
                    == state.session.active_revision_id.as_deref()));
        assert!(state
            .agent_run_events
            .iter()
            .any(|event| event.run_id == run.id
                && event.event_type == CadAgentRunEventType::AgentRunCompleted));
    }

    #[test]
    fn sqlite_repository_recovers_interrupted_missing_corrupt_and_unknown_persistence_state() {
        let app_data_dir =
            std::env::temp_dir().join(format!("cadastrophe-recovery-test-{}", uuid()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir);
        storage::initialize_storage(&layout).unwrap();

        let service = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout.clone())),
        )
        .unwrap();
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let revision_id = created.state.session.active_revision_id.clone().unwrap();
        let (export, _) = service
            .export_artifact(ExportArtifactInput {
                session_id: created.session_id.clone(),
                revision_id: Some(revision_id.clone()),
                format: "metadata".to_string(),
            })
            .unwrap();
        let artifact = export.artifact.unwrap();
        let artifact_path = service.open_artifact(&artifact.id).unwrap().path;

        let orphan_path = layout
            .artifact_path(
                &created.session_id,
                &revision_id,
                "interrupted-write-without-manifest",
                "stl",
            )
            .unwrap();
        fs::create_dir_all(orphan_path.parent().unwrap()).unwrap();
        fs::write(&orphan_path, b"partial artifact").unwrap();
        fs::write(&artifact_path, b"tampered metadata artifact").unwrap();

        let connection = rusqlite::Connection::open(layout.database_path()).unwrap();
        connection
            .execute(
                "UPDATE sessions SET selected_runtime = 'runtime-from-the-future' WHERE id = ?1",
                rusqlite::params![created.session_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE revisions SET source_language = 'braincad' WHERE id = ?1",
                rusqlite::params![revision_id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE artifacts SET metadata_json = '{not-json' WHERE id = ?1",
                rusqlite::params![artifact.id],
            )
            .unwrap();

        let reloaded = SessionService::with_repository(
            layout.clone(),
            Arc::new(SqliteSessionRepository::new(layout)),
        )
        .unwrap();
        let state = reloaded.get_session_state(&created.session_id).unwrap();
        assert_eq!(state.session.selected_runtime, CadRuntimeKind::OpenscadWasm);
        assert!(state
            .session
            .recovery_diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("Unknown persisted runtime") }));
        let active_revision = state.active_revision.as_ref().unwrap();
        assert_eq!(active_revision.source_language, CadSourceLanguage::Openscad);
        assert!(active_revision.diagnostics.items.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Unknown persisted source language")
        }));

        let verified = reloaded
            .verify_artifact_files(Some(created.session_id.clone()))
            .unwrap();
        assert_eq!(verified.checked_count, 1);
        assert_eq!(
            verified.hash_mismatch_artifact_ids,
            vec![artifact.id.clone()]
        );
        assert_eq!(
            verified.size_mismatch_artifact_ids,
            vec![artifact.id.clone()]
        );
        assert_eq!(verified.corrupt_metadata_artifact_ids, vec![artifact.id]);
        assert!(verified
            .orphan_paths
            .iter()
            .any(|path| path.ends_with("interrupted-write-without-manifest.stl")));
        assert!(verified
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("Unknown persisted runtime") }));
        assert!(verified.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("Unknown persisted source language")
        }));
        assert!(verified
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("corrupt persisted metadata") }));
        assert!(verified
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("without a SQLite manifest") }));
    }
}
