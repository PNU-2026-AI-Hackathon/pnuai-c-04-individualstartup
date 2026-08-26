pub mod agent_adapter;
mod agent_gateway;
pub mod agent_thread_manager;
pub mod cli;
pub mod codex_agent_adapter;
pub mod codex_process_client;
pub mod dfm;
pub mod modeling_plane;
pub mod notification_router;
mod prompt_template;
pub mod protocol;
mod runtime;
mod session_repository;
mod session_service;
mod storage;
pub mod validation_plane;

use agent_adapter::AgentAdapter;
use agent_gateway::AgentGateway;
use codex_agent_adapter::CodexAgentAdapter;
use protocol::*;
#[cfg(test)]
use serde_json::Value;
use session_service::SessionService;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    service: Arc<SessionService>,
    gateway: Arc<AgentGateway>,
    codex_adapter: Arc<CodexAgentAdapter>,
}

#[tauri::command]
fn get_dfm_settings(state: State<'_, AppState>) -> Result<dfm::DfmSettings, String> {
    dfm::get_settings(state.service.app_data_dir())
}

#[tauri::command]
fn validate_prusaslicer_executable(
    input: dfm::PathInput,
) -> Result<dfm::ExecutableValidation, String> {
    dfm::validate_executable(&input.path)
}

#[tauri::command]
fn save_prusaslicer_executable(
    input: dfm::PathInput,
    state: State<'_, AppState>,
) -> Result<dfm::ExecutableValidation, String> {
    dfm::save_executable(state.service.app_data_dir(), &input.path)
}

#[tauri::command]
fn validate_dfm_profile(
    input: dfm::ProfileContentsInput,
) -> Result<dfm::ProfileValidation, String> {
    dfm::validate_profile(&input.contents)
}

#[tauri::command]
fn save_dfm_profile(
    input: dfm::ProfileContentsInput,
    state: State<'_, AppState>,
) -> Result<dfm::DfmProfileSettings, String> {
    dfm::save_profile(state.service.app_data_dir(), &input.contents)
}

#[tauri::command]
fn import_dfm_profile(input: dfm::PathInput) -> Result<dfm::ImportedProfile, String> {
    dfm::import_profile(&input.path)
}

#[tauri::command]
fn export_dfm_profile(input: dfm::ExportProfileInput) -> Result<dfm::ExportedProfile, String> {
    dfm::export_profile(&input.path, &input.contents)
}

#[tauri::command]
fn restore_default_dfm_profile(
    state: State<'_, AppState>,
) -> Result<dfm::DfmProfileSettings, String> {
    dfm::restore_default_profile(state.service.app_data_dir())
}

#[tauri::command]
fn create_session(
    input: Option<CreateCadSessionInput>,
    state: State<'_, AppState>,
) -> Result<CreateCadSessionResult, String> {
    state.service.create_session(input.unwrap_or_default())
}

#[tauri::command]
fn get_current_session(state: State<'_, AppState>) -> Result<CurrentCadSessionResult, String> {
    state.service.get_current_session()
}

#[tauri::command]
fn boot_session(state: State<'_, AppState>) -> Result<BootCadSessionResult, String> {
    state.service.boot_session()
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
    state.service.mark_session_viewed(&session_id)
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
    input: DeleteCadSessionInput,
    state: State<'_, AppState>,
) -> Result<DeleteCadSessionResult, String> {
    eprintln!(
        "[cadgen-ax:delete-session] tauri command received session_id={}",
        input.session_id
    );
    state.service.delete_session(&input.session_id)
}

#[tauri::command]
fn duplicate_session(
    input: DuplicateCadSessionInput,
    state: State<'_, AppState>,
) -> Result<CreateCadSessionResult, String> {
    state.service.duplicate_session(input)
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
fn persist_runtime_artifact(
    input: PersistRuntimeArtifactInput,
    state: State<'_, AppState>,
) -> Result<PersistRuntimeArtifactResult, String> {
    state.service.persist_runtime_artifact(input)
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
fn export_artifact_file(
    input: ExportArtifactFileInput,
    state: State<'_, AppState>,
) -> Result<ExportArtifactFileResult, String> {
    state.service.export_artifact_file(input)
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
fn reveal_artifact(
    artifact_id: String,
    state: State<'_, AppState>,
) -> Result<RevealArtifactResult, String> {
    state.service.reveal_artifact(&artifact_id)
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

#[tauri::command]
async fn start_new_agent_conversation(
    input: StartNewAgentConversationInput,
    state: State<'_, AppState>,
) -> Result<StartNewAgentConversationResult, String> {
    state
        .codex_adapter
        .start_new_conversation(&input.session_id)
        .await
}

#[tauri::command]
fn get_agent_session_diagnostics(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<CadAgentSessionDiagnostics, String> {
    state.service.agent_session_diagnostics(&session_id)
}

#[tauri::command]
fn cleanup_agent_transport_events(
    input: CadAgentTransportCleanupInput,
    state: State<'_, AppState>,
) -> Result<CadAgentTransportCleanupResult, String> {
    state.service.cleanup_agent_transport_events(input)
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().map_err(|error| {
                format!("Failed to resolve CADGEN-AX app data directory: {error}")
            })?;
            let storage_layout = storage::StorageLayout::from_app_data_dir(app_data_dir);
            storage::initialize_storage(&storage_layout)
                .map_err(|error| format!("Failed to initialize CADGEN-AX storage: {error}"))?;
            let service = Arc::new(
                SessionService::with_repository(
                    storage_layout.clone(),
                    Arc::new(session_repository::SqliteSessionRepository::new(
                        storage_layout,
                    )),
                )
                .map_err(|error| format!("Failed to load CADGEN-AX sessions: {error}"))?,
            );
            let codex_adapter = Arc::new(
                CodexAgentAdapter::from_env(Arc::clone(&service))
                    .map_err(|error| format!("Failed to initialize Codex adapter: {error}"))?,
            );
            let adapter: Arc<dyn AgentAdapter> = codex_adapter.clone();
            let gateway = Arc::new(AgentGateway::new(Arc::clone(&service), adapter));
            let evaluator = Arc::new(
                validation_plane::codex_vlm_evaluator::CodexVlmEvaluator::new(
                    Arc::clone(&service),
                    codex_adapter.process_client(),
                    validation_plane::codex_vlm_evaluator::CodexVlmEvaluatorConfig::default(),
                )
                .map_err(|error| format!("Failed to initialize VLM evaluator: {error}"))?,
            );
            let weak_gateway = Arc::downgrade(&gateway);
            let refinement_enqueue: validation_plane::coordinator::RefinementEnqueue =
                Arc::new(move |session_id, run_id| {
                    let gateway = weak_gateway.upgrade().ok_or_else(|| {
                        "Agent gateway was dropped before validation refinement.".to_string()
                    })?;
                    gateway.enqueue_refinement(session_id, run_id)
                });
            let validation_coordinator = validation_plane::coordinator::ValidationCoordinator::new(
                Arc::clone(&service),
                evaluator,
                refinement_enqueue,
                codex_agent_adapter::codex_cwd()?,
            )?;
            gateway.attach_validation_coordinator(validation_coordinator.clone())?;
            forward_bridge_events(app.handle().clone(), Arc::clone(&service));
            forward_agent_stream_events(app.handle().clone(), Arc::clone(&service));
            gateway
                .recover_startup_runs()
                .map_err(|error| format!("Failed to start agent run recovery: {error}"))?;
            validation_coordinator
                .recover_startup()
                .map_err(|error| format!("Failed to start validation recovery: {error}"))?;
            app.manage(AppState {
                service,
                gateway,
                codex_adapter,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_session,
            get_current_session,
            boot_session,
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
            persist_runtime_artifact,
            post_user_message,
            create_agent_run,
            list_agent_runs,
            get_agent_run,
            cancel_agent_run,
            export_artifact,
            export_artifact_file,
            read_artifact,
            open_artifact,
            reveal_artifact,
            delete_artifact,
            verify_artifact_files,
            cleanup_orphan_artifacts,
            start_new_agent_conversation,
            get_agent_session_diagnostics,
            cleanup_agent_transport_events,
            get_dfm_settings,
            validate_prusaslicer_executable,
            save_prusaslicer_executable,
            validate_dfm_profile,
            save_dfm_profile,
            import_dfm_profile,
            export_dfm_profile,
            restore_default_dfm_profile
        ])
        .build(tauri::generate_context!())
        .expect("error while building CADGEN-AX Tauri app");

    app.run(|app_handle, event| {
        if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
            let adapter = Arc::clone(&app_handle.state::<AppState>().codex_adapter);
            if let Err(error) = tauri::async_runtime::block_on(adapter.shutdown()) {
                eprintln!("[cadgen-ax:shutdown] failed to stop Codex app-server: {error}");
            }
        }
    });
}

fn forward_bridge_events(app: AppHandle, service: Arc<SessionService>) {
    let mut receiver = service.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Err(error) = app.emit("cad_bridge_event", event) {
                        eprintln!("[cadgen-ax:bridge] emit failed: {error}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    eprintln!("[cadgen-ax:bridge] receiver lagged by {count} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn forward_agent_stream_events(app: AppHandle, service: Arc<SessionService>) {
    let mut receiver = service.subscribe_agent_stream();
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if let Err(error) = app.emit("agent_stream_event", event) {
                        eprintln!("[cadgen-ax:agent-stream] emit failed: {error}");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                    eprintln!("[cadgen-ax:agent-stream] receiver lagged by {count} events");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
