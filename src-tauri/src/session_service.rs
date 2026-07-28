use crate::protocol::*;
use crate::runtime::{
    extract_open_scad_parameters, ok_diagnostics, render_open_scad_preview, DEFAULT_SAMPLE_SOURCE,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Default)]
struct ServiceState {
    sessions: HashMap<String, CadSession>,
    revisions: HashMap<String, CadRevision>,
    artifacts: HashMap<String, CadArtifact>,
    messages: HashMap<String, Vec<CadUserMessage>>,
    conversation: HashMap<String, Vec<CadConversationMessage>>,
    agent_runs: HashMap<String, Vec<CadAgentRun>>,
    current_interactive_session_id: Option<String>,
}

pub struct SessionService {
    inner: Mutex<ServiceState>,
    artifact_root: PathBuf,
    event_sender: broadcast::Sender<CadBridgeEvent>,
}

impl SessionService {
    pub fn new(artifact_root: PathBuf) -> Self {
        let (event_sender, _) = broadcast::channel(256);
        Self {
            inner: Mutex::new(ServiceState::default()),
            artifact_root,
            event_sender,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CadBridgeEvent> {
        self.event_sender.subscribe()
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
            revisions: Vec::new(),
        };
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            state.sessions.insert(session_id.clone(), session);
            state.messages.insert(session_id.clone(), Vec::new());
            state.conversation.insert(session_id.clone(), Vec::new());
            state.agent_runs.insert(session_id.clone(), Vec::new());
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
            state.current_interactive_session_id = Some(session_id.to_string());
        }
        self.get_session_state(session_id)
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
                source_language: input.source_language,
                source: input.source,
                parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
            };
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(revision_id.clone());
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
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

        let (revision_id, mesh, diagnostics) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone())
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let revision = require_revision(&state, &revision_id)?;
            let (mesh, diagnostics) = match revision.source_language {
                CadSourceLanguage::Openscad => {
                    render_open_scad_preview(&revision.source, &revision.parameters)
                }
                _ => (
                    None,
                    CadDiagnostics {
                        ok: false,
                        elapsed_ms: 0,
                        items: vec![CadDiagnostic {
                            severity: "error".to_string(),
                            message: "Rust preview currently supports OpenSCAD source only."
                                .to_string(),
                            line: None,
                            column: None,
                        }],
                    },
                ),
            };
            (revision_id, mesh, diagnostics)
        };
        let artifact = if let Some(mesh) = &mesh {
            Some(self.write_artifact(
                &revision_id,
                CadArtifactKind::PreviewMesh,
                "json",
                &serde_json::to_string(mesh).map_err(|error| error.to_string())?,
                Some(json!({
                    "vertices": mesh.vertices.len() / 3,
                    "triangles": mesh.indices.len() / 3,
                    "sourceLanguage": "openscad"
                })),
            )?)
        } else {
            None
        };
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &revision_id)?;
            revision.diagnostics = diagnostics.clone();
            revision
                .artifacts
                .retain(|candidate| candidate.kind != CadArtifactKind::PreviewMesh);
            if let Some(artifact) = artifact {
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
                artifacts: Vec::new(),
            },
            state_snapshot,
        ))
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
            let revision = require_revision_mut(&mut state, &active_revision_id)?;
            for parameter in &mut revision.parameters {
                if let Some(value) = values.get(&parameter.name) {
                    parameter.value = json_to_parameter_value(value.clone());
                }
            }
            add_user_event(revision, "parameter.updated", json!({ "values": values }));
            let session = require_session_mut(&mut state, session_id)?;
            session.updated_at = timestamp();
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
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
                .push(conversation_message);
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
                .push(message);
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
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let run_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, session_id)?;
            let now = timestamp();
            let run = CadAgentRun {
                id: run_id.clone(),
                session_id: session_id.to_string(),
                status: CadAgentRunStatus::Queued,
                prompt,
                created_at: now.clone(),
                updated_at: now,
                started_at: None,
                completed_at: None,
                error: None,
                active_step: None,
            };
            state
                .agent_runs
                .entry(session_id.to_string())
                .or_default()
                .push(run);
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

    pub fn update_agent_run(
        &self,
        session_id: &str,
        run_id: &str,
        status: Option<CadAgentRunStatus>,
        active_step: Option<Option<String>>,
        error: Option<String>,
        event_type: Option<CadBridgeEventType>,
    ) -> Result<(CadAgentRun, CadSessionState), String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let now = timestamp();
            let run = require_agent_run_mut(&mut state, session_id, run_id)?;
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
            if let Some(error) = error {
                run.error = Some(error);
            }
            run.updated_at = now.clone();
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
            (
                input
                    .revision_id
                    .clone()
                    .or_else(|| session.active_revision_id.clone())
                    .ok_or_else(|| "No active revision is available.".to_string())?,
                input.format.clone(),
            )
        };
        let contents = if format == "metadata" {
            serde_json::to_string_pretty(
                &json!({"revisionId": revision_id, "runtime": "openscad-wasm"}),
            )
            .map_err(|error| error.to_string())?
        } else {
            format!("solid cadastrophe-{revision_id}\nendsolid cadastrophe-{revision_id}\n")
        };
        let artifact_kind = if format == "metadata" {
            CadArtifactKind::Metadata
        } else {
            CadArtifactKind::Stl
        };
        let artifact =
            self.write_artifact(&revision_id, artifact_kind, &format, &contents, None)?;
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &revision_id)?;
            revision.artifacts.push(artifact.clone());
            revision.artifact_count = revision.artifacts.len();
            rebuild_revision_summaries(&mut state, &input.session_id);
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::ArtifactExported,
            &input.session_id,
            snapshot.clone(),
        );
        Ok((
            CadExportResult {
                diagnostics: ok_diagnostics(1),
                artifact: Some(artifact),
            },
            snapshot,
        ))
    }

    pub fn read_artifact(&self, artifact_id: &str) -> Result<String, String> {
        let path = {
            let state = self.inner.lock().map_err(lock_error)?;
            let artifact = state
                .artifacts
                .get(artifact_id)
                .ok_or_else(|| "Artifact not found.".to_string())?;
            artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("path"))
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or_else(|| "Artifact path missing.".to_string())?
        };
        fs::read_to_string(path).map_err(|error| error.to_string())
    }

    pub fn get_session_state(&self, session_id: &str) -> Result<CadSessionState, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        build_state(&state, session_id)
    }

    fn write_artifact(
        &self,
        revision_id: &str,
        kind: CadArtifactKind,
        format: &str,
        contents: &str,
        metadata: Option<Value>,
    ) -> Result<CadArtifact, String> {
        fs::create_dir_all(&self.artifact_root).map_err(|error| error.to_string())?;
        let id = uuid();
        let path = self.artifact_root.join(format!("{id}.{format}"));
        fs::write(&path, contents).map_err(|error| error.to_string())?;
        let mut metadata_map = metadata.map(metadata_from_value).unwrap_or_default();
        metadata_map.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().to_string()),
        );
        let artifact = CadArtifact {
            id: id.clone(),
            revision_id: revision_id.to_string(),
            kind,
            format: format.to_string(),
            uri: format!("tauri://artifact/{id}"),
            bytes: Some(contents.len() as u64),
            created_at: timestamp(),
            metadata: Some(metadata_map),
        };
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
}

pub fn metadata_from_value(value: Value) -> Metadata {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
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

fn rebuild_revision_summaries(state: &mut ServiceState, session_id: &str) {
    let mut summaries: Vec<CadRevisionSummary> = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| CadRevisionSummary {
            id: revision.id.clone(),
            source_language: revision.source_language.clone(),
            created_at: revision.created_at.clone(),
            diagnostics: revision.diagnostics.clone(),
            artifact_count: revision.artifact_count,
        })
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
        .map(|revision| CadRevisionSummary {
            id: revision.id.clone(),
            source_language: revision.source_language.clone(),
            created_at: revision.created_at.clone(),
            diagnostics: revision.diagnostics.clone(),
            artifact_count: revision.artifact_count,
        })
        .collect();
    session
        .revisions
        .sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let active_revision = session
        .active_revision_id
        .as_ref()
        .and_then(|revision_id| state.revisions.get(revision_id))
        .cloned();
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
    })
}

fn require_session<'a>(
    state: &'a ServiceState,
    session_id: &str,
) -> Result<&'a CadSession, String> {
    state
        .sessions
        .get(session_id)
        .ok_or_else(|| format!("CAD session not found: {session_id}"))
}

fn require_session_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
) -> Result<&'a mut CadSession, String> {
    state
        .sessions
        .get_mut(session_id)
        .ok_or_else(|| format!("CAD session not found: {session_id}"))
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
}
