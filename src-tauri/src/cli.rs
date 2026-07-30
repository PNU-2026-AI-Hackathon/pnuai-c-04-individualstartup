mod artifacts;
mod model_commands;
mod openscad;
mod session_commands;
mod structural;
mod support;
mod vlm;
mod workflow_commands;
mod workflow_support;

use crate::protocol::{
    CadAgentRunEventType, CadAgentRunStatus, CadArtifact, CadArtifactKind, CadExportResult,
    CadModelPlan, CadSourceLanguage, CadWorkflowOuterIteration, CadWorkflowPendingVlm,
    CadWorkflowPlan, ExportArtifactInput, PersistRuntimeArtifactInput, UpdateModelSourceInput,
};
use crate::session_repository::SqliteSessionRepository;
use crate::session_service::SessionService;
use crate::storage::{self, StorageLayout};
use artifacts::{artifact_paths, base64_encode};
use openscad::render_open_scad_wasm_cli;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use structural::{
    evaluate_structural_for_revision, structural_failure_report, validate_structural_report,
    StructuralEvaluation,
};
use support::*;
use vlm::{
    build_vlm_contract, render_vlm_images_for_artifact, validate_vlm_contract,
    validate_vlm_contract_value, validate_vlm_judge_report, vlm_failure_report,
};

pub fn session_current_main() -> i32 {
    run(
        "cadastrophe-session-current",
        session_commands::session_current,
    )
}

pub fn session_state_main() -> i32 {
    run("cadastrophe-session-state", session_commands::session_state)
}

pub fn plan_commit_main() -> i32 {
    run("cadastrophe-plan-commit", model_commands::plan_commit)
}

pub fn source_apply_main() -> i32 {
    run("cadastrophe-source-apply", model_commands::source_apply)
}

pub fn preview_render_main() -> i32 {
    run("cadastrophe-preview-render", model_commands::preview_render)
}

pub fn artifact_export_main() -> i32 {
    run(
        "cadastrophe-artifact-export",
        model_commands::artifact_export,
    )
}

pub fn evaluate_structural_main() -> i32 {
    run(
        "cadastrophe-evaluate-structural",
        workflow_commands::evaluate_structural,
    )
}

pub fn finalize_main() -> i32 {
    run("cadastrophe-finalize", workflow_commands::finalize)
}

pub fn vlm_submit_main() -> i32 {
    run("cadastrophe-vlm-submit", workflow_commands::vlm_submit)
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
            "Failed to initialize Cadastrophe storage at {}: {error}",
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
