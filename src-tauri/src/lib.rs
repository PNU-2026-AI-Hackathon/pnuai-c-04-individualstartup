pub mod agent_adapter;
mod agent_gateway;
pub mod codex_agent_adapter;
pub mod codex_process_client;
mod fake_agent_adapter;
pub mod protocol;
mod runtime;
mod session_repository;
mod session_service;
mod storage;

use agent_adapter::AgentAdapter;
use agent_gateway::AgentGateway;
use codex_agent_adapter::CodexAgentAdapter;
use fake_agent_adapter::FakeAgentAdapter;
use protocol::*;
use serde::Deserialize;
use serde_json::{Map, Value};
use session_service::SessionService;
use std::sync::Arc;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

struct AppState {
    service: Arc<SessionService>,
    gateway: Arc<AgentGateway>,
    smoke_ui_loaded: Mutex<Option<oneshot::Sender<String>>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateParametersInput {
    session_id: String,
    values: Map<String, Value>,
}

#[tauri::command]
fn create_session(
    input: Option<CreateCadSessionInput>,
    state: State<'_, AppState>,
) -> Result<CreateCadSessionResult, String> {
    let result = state.service.create_session(input.unwrap_or_default())?;
    signal_smoke_ui_loaded(&state, result.session_id.clone())?;
    Ok(result)
}

#[tauri::command]
fn get_current_session(state: State<'_, AppState>) -> Result<CurrentCadSessionResult, String> {
    state.service.get_current_session()
}

#[tauri::command]
fn get_session_state(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    state.service.get_session_state(&session_id)
}

#[tauri::command]
fn mark_session_viewed(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    let result = state.service.mark_session_viewed(&session_id)?;
    signal_smoke_ui_loaded(&state, session_id)?;
    Ok(result)
}

#[tauri::command]
fn list_sessions(
    input: Option<ListCadSessionsInput>,
    state: State<'_, AppState>,
) -> Result<ListCadSessionsResult, String> {
    state
        .service
        .list_sessions_for_input(input.unwrap_or_default())
}

#[tauri::command]
fn rename_session(
    input: RenameCadSessionInput,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    state.service.rename_session(input)
}

#[tauri::command]
fn archive_session(
    input: ArchiveCadSessionInput,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    state.service.archive_session(input)
}

#[tauri::command]
fn delete_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<DeleteCadSessionResult, String> {
    state.service.delete_session(&session_id)
}

#[tauri::command]
fn duplicate_session(
    input: DuplicateCadSessionInput,
    state: State<'_, AppState>,
) -> Result<CreateCadSessionResult, String> {
    state.service.duplicate_session(input)
}

fn signal_smoke_ui_loaded(state: &State<'_, AppState>, session_id: String) -> Result<(), String> {
    if let Some(sender) = state
        .smoke_ui_loaded
        .lock()
        .map_err(|_| "Smoke signal lock is poisoned.".to_string())?
        .take()
    {
        let _ = sender.send(session_id);
    }
    Ok(())
}

#[tauri::command]
fn update_model_source(
    input: UpdateModelSourceInput,
    state: State<'_, AppState>,
) -> Result<UpdateModelSourceResult, String> {
    state.service.update_model_source(input)
}

#[tauri::command]
fn set_active_revision(
    input: SetActiveRevisionInput,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    state.service.set_active_revision(input)
}

#[tauri::command]
fn restore_revision(
    input: RestoreRevisionInput,
    state: State<'_, AppState>,
) -> Result<RestoreRevisionResult, String> {
    state.service.restore_revision(input)
}

#[tauri::command]
fn render_preview(
    input: RenderPreviewInput,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let (result, state) = state.service.render_preview(input)?;
    Ok(serde_json::json!({ "result": result, "state": state }))
}

#[tauri::command]
fn update_parameters(
    input: UpdateParametersInput,
    state: State<'_, AppState>,
) -> Result<CadSessionState, String> {
    state
        .service
        .update_parameters(&input.session_id, input.values)
}

#[tauri::command]
fn post_user_message(
    input: PostUserMessageInput,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let (message, session_state) = state.service.post_user_message(input)?;
    Ok(serde_json::json!({ "message": message, "state": session_state }))
}

#[tauri::command]
fn create_agent_run(
    input: CreateAgentRunInput,
    state: State<'_, AppState>,
) -> Result<CreateAgentRunResult, String> {
    state.gateway.start_run(input)
}

#[tauri::command]
fn list_agent_runs(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "runs": state.gateway.list_runs(&session_id)? }))
}

#[tauri::command]
fn get_agent_run(
    session_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<CadAgentRun, String> {
    state
        .gateway
        .get_run(&session_id, &run_id)?
        .ok_or_else(|| "Agent run not found.".to_string())
}

#[tauri::command]
fn cancel_agent_run(
    session_id: String,
    run_id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let (run, session_state) = state.gateway.cancel_run(&session_id, &run_id)?;
    Ok(serde_json::json!({ "run": run, "state": session_state }))
}

#[tauri::command]
fn export_artifact(
    input: ExportArtifactInput,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let (result, session_state) = state.service.export_artifact(input)?;
    Ok(serde_json::json!({ "result": result, "state": session_state }))
}

#[tauri::command]
fn read_artifact(artifact_id: String, state: State<'_, AppState>) -> Result<String, String> {
    state.service.read_artifact(&artifact_id)
}

#[tauri::command]
fn open_artifact(
    artifact_id: String,
    state: State<'_, AppState>,
) -> Result<OpenArtifactResult, String> {
    state.service.open_artifact(&artifact_id)
}

#[tauri::command]
fn delete_artifact(
    input: DeleteArtifactInput,
    state: State<'_, AppState>,
) -> Result<DeleteArtifactResult, String> {
    state.service.delete_artifact(input)
}

#[tauri::command]
fn verify_artifact_files(
    input: Option<VerifyArtifactFilesInput>,
    state: State<'_, AppState>,
) -> Result<VerifyArtifactFilesResult, String> {
    state
        .service
        .verify_artifact_files(input.and_then(|input| input.session_id))
}

#[tauri::command]
fn cleanup_orphan_artifacts(
    input: Option<CleanupOrphanArtifactsInput>,
    state: State<'_, AppState>,
) -> Result<CleanupOrphanArtifactsResult, String> {
    state
        .service
        .cleanup_orphan_artifacts(input.unwrap_or_default())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("Failed to resolve Cadastrophe app data directory: {error}"))?;
            let storage_layout = storage::StorageLayout::from_app_data_dir(app_data_dir);
            storage::initialize_storage(&storage_layout)
                .map_err(|error| format!("Failed to initialize Cadastrophe storage: {error}"))?;
            let service = Arc::new(
                SessionService::with_repository(
                    storage_layout.clone(),
                    Arc::new(session_repository::SqliteSessionRepository::new(storage_layout)),
                )
                .map_err(|error| format!("Failed to load Cadastrophe sessions: {error}"))?,
            );
            let (smoke_tx, smoke_rx) = oneshot::channel();
            let smoke_enabled = std::env::var("CADASTROPHE_TAURI_SMOKE").is_ok_and(|value| value == "1");
            let adapter: Arc<dyn AgentAdapter> = match std::env::var("CADASTROPHE_AGENT_ADAPTER").as_deref() {
                Ok("fake") => Arc::new(FakeAgentAdapter),
                Ok("codex") | Err(_) => Arc::new(CodexAgentAdapter::from_env()),
                Ok(other) => {
                    eprintln!(
                        "Unknown CADASTROPHE_AGENT_ADAPTER={other:?}; using real Codex adapter. Set it to \"fake\" for deterministic tests."
                    );
                    Arc::new(CodexAgentAdapter::from_env())
                }
            };
            let gateway = Arc::new(AgentGateway::new(Arc::clone(&service), adapter));
            forward_bridge_events(app.handle().clone(), Arc::clone(&service));
            if smoke_enabled {
                run_tauri_smoke(app.handle().clone(), Arc::clone(&service), smoke_rx);
            }
            app.manage(AppState {
                service,
                gateway,
                smoke_ui_loaded: Mutex::new(smoke_enabled.then_some(smoke_tx)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_session,
            get_current_session,
            get_session_state,
            mark_session_viewed,
            list_sessions,
            rename_session,
            archive_session,
            delete_session,
            duplicate_session,
            update_model_source,
            set_active_revision,
            restore_revision,
            render_preview,
            update_parameters,
            post_user_message,
            create_agent_run,
            list_agent_runs,
            get_agent_run,
            cancel_agent_run,
            export_artifact,
            read_artifact,
            open_artifact,
            delete_artifact,
            verify_artifact_files,
            cleanup_orphan_artifacts
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cadastrophe Tauri app");
}

fn forward_bridge_events(app: AppHandle, service: Arc<SessionService>) {
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        while let Ok(event) = receiver.recv().await {
            let _ = app.emit("cad_bridge_event", event);
        }
    });
}

fn run_tauri_smoke(
    app: AppHandle,
    service: Arc<SessionService>,
    smoke_ui_loaded: oneshot::Receiver<String>,
) {
    tauri::async_runtime::spawn(async move {
        let result = async {
            let session_id =
                tokio::time::timeout(std::time::Duration::from_secs(20), smoke_ui_loaded)
                    .await
                    .map_err(|_| {
                        "Timed out waiting for UI to call mark_session_viewed.".to_string()
                    })?
                    .map_err(|_| "UI smoke signal channel closed.".to_string())?;
            let state = service.get_session_state(&session_id)?;
            let prompt = "Tauri smoke: create a simple bracket preview.";
            trigger_ui_agent_run(
                &app,
                &session_id,
                prompt,
                state.session.active_revision_id.as_deref(),
            )?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let state = service.get_session_state(&session_id)?;
                let Some(run) = state.agent_runs.iter().find(|run| run.prompt == prompt) else {
                    if std::time::Instant::now() > deadline {
                        return Err(
                            "Timed out waiting for WebView IPC to create an agent run.".to_string()
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                };
                match run.status {
                    CadAgentRunStatus::Completed => {
                        if state.active_revision.as_ref().is_some_and(|revision| {
                            revision
                                .artifacts
                                .iter()
                                .any(|artifact| artifact.kind == CadArtifactKind::PreviewMesh)
                        }) {
                            return Ok(());
                        }
                        return Err(
                            "Smoke run completed without preview mesh artifact.".to_string()
                        );
                    }
                    CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled => {
                        return Err(format!("Smoke run ended with status {:?}.", run.status));
                    }
                    _ => {
                        if std::time::Instant::now() > deadline {
                            return Err("Timed out waiting for smoke run completion.".to_string());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
        .await;
        match result {
            Ok(()) => {
                println!(
                    "Tauri smoke passed: UI loaded, backend IPC connected, fake run completed."
                );
                app.exit(0);
                std::process::exit(0);
            }
            Err(error) => {
                eprintln!("Tauri smoke failed: {error}");
                app.exit(1);
                std::process::exit(1);
            }
        }
    });
}

fn trigger_ui_agent_run(
    app: &AppHandle,
    session_id: &str,
    prompt: &str,
    revision_id: Option<&str>,
) -> Result<(), String> {
    let session_id_json = serde_json::to_string(session_id).map_err(|error| error.to_string())?;
    let prompt_json = serde_json::to_string(prompt).map_err(|error| error.to_string())?;
    let revision_id_json =
        serde_json::to_string(&revision_id).map_err(|error| error.to_string())?;
    let script = format!(
        r#"
(() => {{
  const invoke = window.__TAURI__?.core?.invoke ?? window.__TAURI_INTERNALS__?.invoke;
  if (!invoke) {{
    window.__cadastropheSmokeError = "Tauri invoke is not available in the WebView.";
    return;
  }}
  invoke("create_agent_run", {{
    input: {{
      sessionId: {session_id_json},
      prompt: {prompt_json},
      revisionId: {revision_id_json}
    }}
  }}).catch((error) => {{
    window.__cadastropheSmokeError = String(error);
  }});
}})();
"#
    );
    let app = app.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    app.clone()
        .run_on_main_thread(move || {
            let result = app
                .get_webview_window("main")
                .ok_or_else(|| "Main webview window is not available for Tauri smoke.".to_string())
                .and_then(|window| window.eval(script).map_err(|error| error.to_string()));
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;
    receiver
        .recv_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "Timed out scheduling WebView IPC smoke script.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};

    #[test]
    fn gateway_start_run_is_safe_from_sync_tauri_command_context() {
        let service = Arc::new(SessionService::new(
            std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
        ));
        let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(FakeAgentAdapter));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();

        let started = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "Create a sync-command launch regression fixture.".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(100));
        let state = service.get_session_state(&created.session_id).unwrap();
        let run = state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap();
        assert_eq!(run.status, CadAgentRunStatus::Completed);
    }

    #[tokio::test]
    async fn fake_gateway_completes_prompt_to_preview_loop() {
        let service = Arc::new(SessionService::new(
            std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
        ));
        let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(FakeAgentAdapter));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let started = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "Create a slotted fixture plate.".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let state = service.get_session_state(&created.session_id).unwrap();
        let completed = state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap();
        assert_eq!(completed.status, CadAgentRunStatus::Completed);
        assert!(state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == CadArtifactKind::PreviewMesh));
        let run = state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap();
        assert_eq!(
            run.input_revision_id.as_deref(),
            created.state.session.active_revision_id.as_deref()
        );
        assert_eq!(
            run.output_revision_id.as_deref(),
            state.session.active_revision_id.as_deref()
        );
        let active_summary = state
            .session
            .revisions
            .iter()
            .find(|revision| {
                Some(revision.id.as_str()) == state.session.active_revision_id.as_deref()
            })
            .unwrap();
        assert!(active_summary
            .run_links
            .iter()
            .any(|link| link.run_id == started.run.id && link.role == "output"));
    }

    #[tokio::test]
    async fn fake_gateway_records_adapter_failure() {
        let service = Arc::new(SessionService::new(
            std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
        ));
        let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(FakeAgentAdapter));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let started = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "fail adapter".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let state = service.get_session_state(&created.session_id).unwrap();
        let failed = state
            .agent_runs
            .iter()
            .find(|run| run.id == started.run.id)
            .unwrap();
        assert_eq!(failed.status, CadAgentRunStatus::Failed);
        assert!(failed
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Fake agent"));
        assert_eq!(state.session.status, CadSessionStatus::Failed);
    }

    #[tokio::test]
    async fn fake_gateway_cancel_marks_running_run_cancelled() {
        let service = Arc::new(SessionService::new(
            std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
        ));
        let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(FakeAgentAdapter));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let started = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "Create a slow-ish fake run.".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();
        let (cancelled, state) = gateway
            .cancel_run(&created.session_id, &started.run.id)
            .unwrap();
        assert_eq!(cancelled.status, CadAgentRunStatus::Cancelled);
        assert_eq!(
            state
                .agent_runs
                .iter()
                .find(|run| run.id == started.run.id)
                .unwrap()
                .status,
            CadAgentRunStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn gateway_serializes_runs_per_session() {
        let service = Arc::new(SessionService::new(
            std::env::temp_dir().join(format!("cadastrophe-tauri-test-{}", uuid::Uuid::new_v4())),
        ));
        let gateway = AgentGateway::new(Arc::clone(&service), Arc::new(OutOfOrderAdapter));
        let created = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let first = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "first slow source".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();
        let second = gateway
            .start_run(CreateAgentRunInput {
                session_id: created.session_id.clone(),
                prompt: "second fast source".to_string(),
                revision_id: created.state.session.active_revision_id.clone(),
                retry_of_run_id: None,
            })
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let state = service.get_session_state(&created.session_id).unwrap();
        assert_eq!(
            state
                .agent_runs
                .iter()
                .find(|run| run.id == first.run.id)
                .unwrap()
                .status,
            CadAgentRunStatus::Completed
        );
        assert_eq!(
            state
                .agent_runs
                .iter()
                .find(|run| run.id == second.run.id)
                .unwrap()
                .status,
            CadAgentRunStatus::Completed
        );
        assert!(state
            .active_revision
            .as_ref()
            .unwrap()
            .source
            .contains("second fast source"));
    }

    struct OutOfOrderAdapter;

    #[async_trait::async_trait]
    impl AgentAdapter for OutOfOrderAdapter {
        async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
            if input.prompt.contains("first") {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Ok(vec![AgentAdapterEvent::SourceUpdated {
                source_language: CadSourceLanguage::Openscad,
                source: format!("// {}\ncube([1, 1, 1]);", input.prompt),
            }])
        }
    }
}
