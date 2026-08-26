pub(crate) mod artifacts;
mod model_commands;
mod session_commands;
pub(crate) mod structural;
mod support;
pub(crate) mod vlm;
mod workflow_commands;
mod workflow_support;

use crate::protocol::{
    CadAgentRunEventType, CadArtifact, CadArtifactKind, CadExportResult, CadSourceLanguage,
    CadWorkflowPlan, ExportArtifactInput, RenderPreviewInput, UpdateModelSourceInput,
};
use crate::session_repository::SqliteSessionRepository;
use crate::session_service::SessionService;
use crate::storage::{self, StorageLayout};
use artifacts::artifact_paths;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

pub fn session_current_main() -> i32 {
    run(
        "cadgen-ax-session-current",
        session_commands::session_current,
    )
}

pub fn session_state_main() -> i32 {
    run("cadgen-ax-session-state", session_commands::session_state)
}

pub fn plan_commit_main() -> i32 {
    run("cadgen-ax-plan-commit", model_commands::plan_commit)
}

pub fn source_apply_main() -> i32 {
    run("cadgen-ax-source-apply", model_commands::source_apply)
}

pub fn finalize_main() -> i32 {
    run("cadgen-ax-finalize", workflow_commands::finalize)
}

pub fn vlm_submit_main() -> i32 {
    const COMMAND: &str = "cadgen-ax-vlm-submit";
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => return emit_error(COMMAND, false, error),
    };
    match vlm::build_vlm_submission(&parsed) {
        Ok(output) => emit_success(COMMAND, parsed.pretty, output),
        Err(error) => emit_error(COMMAND, parsed.pretty, error),
    }
}

fn run(
    command: &'static str,
    handler: fn(&ParsedArgs, &SessionService, &PathBuf) -> CliResult<CommandOutput>,
) -> i32 {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(parsed) => parsed,
        Err(error) => return emit_error(command, false, error),
    };
    let pretty = parsed.pretty;
    let app_data_dir = match parsed.app_data_dir() {
        Ok(path) => path,
        Err(error) => return emit_error(command, pretty, error),
    };
    let service = match load_service(app_data_dir.clone()) {
        Ok(service) => service,
        Err(error) => return emit_error(command, pretty, error),
    };
    match handler(&parsed, &service, &app_data_dir) {
        Ok(output) => emit_success(command, pretty, output.data),
        Err(error) => emit_error(command, pretty, error),
    }
}

fn load_service(app_data_dir: PathBuf) -> CliResult<SessionService> {
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).map_err(|error| {
        CliError::storage(format!(
            "Failed to initialize CADGEN-AX storage at {}: {error}",
            layout.app_data_dir().display()
        ))
    })?;
    SessionService::with_repository_without_startup_verification(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .map_err(CliError::storage)
}

#[cfg(test)]
mod tests;
