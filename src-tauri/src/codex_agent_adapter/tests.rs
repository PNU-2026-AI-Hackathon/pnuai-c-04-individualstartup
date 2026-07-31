use super::events::{map_item_started, CodexEventCollector};
use super::prompt::{build_cad_prompt, build_thread_start_params, build_turn_start_params};
use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};
use crate::protocol::CadSourceLanguage;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn prompt_limits_agent_to_modeling_cli_surface() {
    let prompt = build_cad_prompt(&AgentAdapterRunInput {
        session_id: "session-1".to_string(),
        run_id: "run-1".to_string(),
        app_data_dir: PathBuf::from("/tmp/Cad App Data"),
        prompt: "Create a wall bracket.".to_string(),
        revision_id: Some("revision-1".to_string()),
        revision_source_language: Some(CadSourceLanguage::Openscad),
        revision_source: Some("cube([1, 1, 1]);".to_string()),
        latest_workflow_failure_report: Some(json!({
            "contractType": "cadastrophe.failure_report.v1",
            "reason": "missing_support_tab",
            "nextAction": "outer_loop_refine_source"
        })),
        event_sink: None,
    });

    assert!(prompt.contains("--app-data-dir '/tmp/Cad App Data'"));
    assert!(prompt.contains(
            "cadastrophe-plan-commit --app-data-dir '/tmp/Cad App Data' --session session-1 --run run-1"
        ));
    assert!(prompt.contains("Do not call `cadastrophe-source-apply`"));
    assert!(prompt.contains(
        "cadastrophe-finalize --app-data-dir '/tmp/Cad App Data' --session session-1 --run run-1"
    ));
    assert!(prompt.contains("cadastrophe.vlm_judge.v1"));
    assert!(prompt.contains("the app will consume it and record the VLM result automatically"));
    assert!(prompt.contains("Do not call `cadastrophe-preview-render`"));
    assert!(prompt.contains("CadModelPlanDraft"));
    assert!(prompt.contains("Include only `summary`, `mainComponent`, `supportingComponents`, and `expectedAspectRatio`"));
    assert!(prompt
        .contains("Do not include `schemaVersion`, `sourceLanguage`, or `runtimeConstraints`"));
    assert!(!prompt.contains("cadastrophe-preview-render --app-data-dir"));
    assert!(!prompt.contains("cadastrophe-artifact-export --app-data-dir"));
    assert!(!prompt.contains("cadastrophe-evaluate-structural --app-data-dir"));
    assert!(!prompt.contains("cadastrophe-vlm-submit --app-data-dir"));
    assert!(prompt.contains("// @main_component <name>"));
    assert!(prompt.contains("latestWorkflowFailureReport"));
    assert!(prompt.contains("missing_support_tab"));
    assert!(!prompt.contains("Return only a JSON object"));
}

#[test]
fn codex_turn_uses_workspace_write_with_app_data_writable_root() {
    let cwd = PathBuf::from("/Users/example/cadastrophe");
    let app_data_dir = PathBuf::from("/Users/example/Library/Application Support/Cadastrophe");

    let thread = build_thread_start_params(&cwd);
    assert_eq!(thread["cwd"], "/Users/example/cadastrophe");
    assert_eq!(thread["sandbox"], "workspace-write");
    assert_eq!(thread["approvalPolicy"], "never");

    let turn = build_turn_start_params("thread-1", "prompt", &cwd, &app_data_dir);
    assert_eq!(turn["threadId"], "thread-1");
    assert_eq!(turn["cwd"], "/Users/example/cadastrophe");
    assert_eq!(turn["approvalPolicy"], "never");
    assert_eq!(turn["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(
        turn["sandboxPolicy"]["writableRoots"][0],
        "/Users/example/Library/Application Support/Cadastrophe"
    );
}

#[test]
fn labels_command_execution_array_events() {
    let mut events = Vec::new();
    let input = test_input();
    map_item_started(
        &json!({
            "item": {
                "type": "commandExecution",
                "command": ["cadastrophe-plan-commit", "--session", "session-1"]
            }
        }),
        &input,
        &mut events,
    )
    .unwrap();

    assert!(matches!(
        events.first(),
        Some(AgentAdapterEvent::ToolStarted { name })
            if name == "cadastrophe-plan-commit --session session-1"
    ));
}

#[test]
fn collects_current_codex_agent_message_events() {
    let mut collector = CodexEventCollector::default();
    let mut events = Vec::new();
    let input = test_input();
    collector
        .ingest(
            &json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "itemId": "msg-1",
                    "delta": "{\"sourceLanguage\":\"openscad\","
                }
            }),
            &input,
            &mut events,
        )
        .unwrap();
    collector.ingest(
            &json!({
                "method": "item/completed",
                "params": {
                    "item": {
                        "type": "agentMessage",
                        "id": "msg-1",
                        "text": "{\"sourceLanguage\":\"openscad\",\"source\":\"sphere(r = 3);\",\"message\":\"Done.\"}"
                    }
                }
            }),
            &input,
            &mut events,
        )
        .unwrap();

    assert!(collector.assistant_text().contains("sphere(r = 3)"));
}

fn test_input() -> AgentAdapterRunInput {
    AgentAdapterRunInput {
        session_id: "session-1".to_string(),
        run_id: "run-1".to_string(),
        app_data_dir: PathBuf::from("/tmp/cadastrophe"),
        prompt: "Create a cube.".to_string(),
        revision_id: None,
        revision_source_language: None,
        revision_source: None,
        latest_workflow_failure_report: None,
        event_sink: None,
    }
}
