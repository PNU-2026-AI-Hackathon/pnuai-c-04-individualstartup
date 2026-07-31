pub mod agent_adapter;
mod agent_gateway;
pub mod cli;
pub mod codex_agent_adapter;
pub mod codex_process_client;
pub mod protocol;
mod runtime;
mod session_repository;
mod session_service;
mod storage;

use agent_adapter::AgentAdapter;
use agent_gateway::AgentGateway;
use codex_agent_adapter::CodexAgentAdapter;
use protocol::*;
use serde::Deserialize;
use serde_json::{Map, Value};
use session_service::SessionService;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    service: Arc<SessionService>,
    gateway: Arc<AgentGateway>,
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
        "[cadastrophe:delete-session] tauri command received session_id={}",
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

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_data_dir = app.path().app_data_dir().map_err(|error| {
                format!("Failed to resolve Cadastrophe app data directory: {error}")
            })?;
            let storage_layout = storage::StorageLayout::from_app_data_dir(app_data_dir);
            storage::initialize_storage(&storage_layout)
                .map_err(|error| format!("Failed to initialize Cadastrophe storage: {error}"))?;
            let service = Arc::new(
                SessionService::with_repository(
                    storage_layout.clone(),
                    Arc::new(session_repository::SqliteSessionRepository::new(
                        storage_layout,
                    )),
                )
                .map_err(|error| format!("Failed to load Cadastrophe sessions: {error}"))?,
            );
            let adapter: Arc<dyn AgentAdapter> = Arc::new(CodexAgentAdapter::from_env());
            let gateway = Arc::new(AgentGateway::new(Arc::clone(&service), adapter));
            forward_bridge_events(app.handle().clone(), Arc::clone(&service));
            app.manage(AppState { service, gateway });
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
            update_parameters,
            post_user_message,
            create_agent_run,
            list_agent_runs,
            get_agent_run,
            cancel_agent_run,
            export_artifact,
            read_artifact,
            open_artifact,
            reveal_artifact,
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
