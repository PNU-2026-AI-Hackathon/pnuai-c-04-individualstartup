use crate::agent_adapter::AgentAdapterRunInput;
use crate::modeling_plane::prompt::{
    render_modeling_developer_instructions, render_modeling_turn_input,
};
use serde_json::{json, Value};
use std::path::Path;

pub(super) fn build_thread_start_params(cwd: &Path) -> Result<Value, String> {
    Ok(json!({
        "approvalPolicy": "never",
        "cwd": cwd,
        "developerInstructions": render_modeling_developer_instructions()?,
        "personality": "pragmatic",
        "sandbox": "workspace-write",
        "serviceName": "cadastrophe-tauri-backend",
        "sessionStartSource": "startup"
    }))
}

pub(super) fn build_turn_start_params(prompt: &str, cwd: &Path, app_data_dir: &Path) -> Value {
    json!({
        "input": [
            {
                "type": "text",
                "text": prompt,
                "text_elements": []
            }
        ],
        "personality": "pragmatic",
        "approvalPolicy": "never",
        "cwd": cwd,
        "sandboxPolicy": {
            "type": "workspaceWrite",
            "writableRoots": [app_data_dir],
            "networkAccess": false
        }
    })
}

pub(super) fn build_modeling_turn_input(input: &AgentAdapterRunInput) -> Result<String, String> {
    render_modeling_turn_input(input)
}
