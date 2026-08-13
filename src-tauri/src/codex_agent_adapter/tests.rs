use super::events::{map_item_started, CodexEventCollector};
use super::prompt::{build_cad_prompt, build_thread_start_params, build_turn_start_params};
use super::terminal_result;
use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};
use crate::notification_router::{NotificationIdentifiers, RoutedNotification};
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
    })
    .unwrap();

    assert!(prompt.contains("--app-data-dir '/tmp/Cad App Data'"));
    assert!(prompt.contains(
        "cadastrophe-plan-commit --app-data-dir '/tmp/Cad App Data' --session 'session-1' --run 'run-1' --revision 'revision-1' --plan <file>"
    ));
    assert!(prompt.contains("Do not call `cadastrophe-source-apply`"));
    assert!(prompt.contains(
        "cadastrophe-finalize --app-data-dir '/tmp/Cad App Data' --session 'session-1' --run 'run-1' --revision <revision_id_from_source_apply>"
    ));
    assert!(prompt.contains(
        "cadastrophe-source-apply --app-data-dir '/tmp/Cad App Data' --session 'session-1' --run 'run-1' --revision 'revision-1' --source <file>"
    ));
    assert!(prompt.contains("--session 'session-1'"));
    assert!(prompt.contains("--run 'run-1'"));
    for scoped_command in [
        "cadastrophe-session-state --app-data-dir '/tmp/Cad App Data'",
        "cadastrophe-plan-commit --app-data-dir '/tmp/Cad App Data'",
        "cadastrophe-source-apply --app-data-dir '/tmp/Cad App Data'",
        "cadastrophe-finalize --app-data-dir '/tmp/Cad App Data'",
    ] {
        for line in prompt.lines().filter(|line| line.contains(scoped_command)) {
            assert!(line.contains("--session 'session-1'"), "{line}");
            assert!(line.contains("--run 'run-1'"), "{line}");
        }
    }
    assert!(!prompt.contains("--language openscad"));
    assert!(prompt.contains("--revision 'revision-1' --source <file>"));
    assert!(prompt.contains("\"sessionId\": \"session-1\""));
    assert!(prompt.contains("\"runId\": \"run-1\""));
    assert!(prompt.contains("enqueues app-owned VLM evaluation"));
    assert!(prompt.contains("end this modeling"));
    assert!(!prompt.contains("cadastrophe-vlm-judge"));
    assert!(!prompt.contains("separate subagent"));
    assert!(prompt.contains("Do not call `cadastrophe-preview-render`"));
    assert!(prompt.contains("CadModelPlanDraft"));
    assert!(prompt.contains(
        "`CadModelPlanDraft` with `summary`, `mainComponent`,\n`supportingComponents`, and `expectedAspectRatio`"
    ));
    assert!(!prompt.contains("full persisted `CadModelPlan`"));
    assert!(!prompt.contains("Do not include `schemaVersion`"));
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

    let turn = build_turn_start_params("prompt", &cwd, &app_data_dir);
    assert!(turn.get("threadId").is_none());
    assert_eq!(turn["cwd"], "/Users/example/cadastrophe");
    assert_eq!(turn["approvalPolicy"], "never");
    assert_eq!(turn["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(
        turn["input"],
        json!([{
            "type": "text",
            "text": "prompt",
            "text_elements": []
        }])
    );
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
            &routed(
                1,
                json!({
                    "method": "item/agentMessage/delta",
                    "params": {
                        "itemId": "msg-1",
                        "delta": "{\"sourceLanguage\":\"openscad\","
                    }
                }),
            ),
            "agent-thread-1",
            &input,
            &mut events,
        )
        .unwrap();
    collector.ingest(
            &routed(2, json!({
                "method": "item/completed",
                "params": {
                    "item": {
                        "type": "agentMessage",
                        "id": "msg-1",
                        "text": "{\"sourceLanguage\":\"openscad\",\"source\":\"sphere(r = 3);\",\"message\":\"Done.\"}"
                    }
                }
            })),
            "agent-thread-1",
            &input,
            &mut events,
        )
        .unwrap();

    collector
        .finalize_legacy_messages(&input, &mut events)
        .unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        AgentAdapterEvent::AgentMessageCompleted { content, is_final: true, .. }
            if content.contains("sphere(r = 3)")
    )));
}

#[test]
fn completed_agent_message_without_text_is_a_protocol_error() {
    let mut collector = CodexEventCollector::default();
    let mut events = Vec::new();
    let input = test_input();
    collector
        .ingest(
            &routed(
                1,
                json!({
                    "method": "item/agentMessage/delta",
                    "params": {"itemId": "msg-1", "delta": "ephemeral only"}
                }),
            ),
            "agent-thread-1",
            &input,
            &mut events,
        )
        .unwrap();
    let error = collector
        .ingest(
            &routed(
                2,
                json!({
                    "method": "item/completed",
                    "params": {"item": {"type": "agentMessage", "id": "msg-1"}}
                }),
            ),
            "agent-thread-1",
            &input,
            &mut events,
        )
        .unwrap_err();
    assert_eq!(error, "Completed agentMessage msg-1 contains no text.");
    assert!(!events
        .iter()
        .any(|event| matches!(event, AgentAdapterEvent::AgentMessageCompleted { .. })));
}

#[test]
fn phase_less_completed_messages_classify_only_the_last_as_final_answer() {
    let mut collector = CodexEventCollector::default();
    let mut events = Vec::new();
    let input = test_input();
    for (sequence, item_id, text) in [(1, "msg-1", "Working."), (2, "msg-2", "Done.")] {
        collector
            .ingest(
                &routed(sequence, json!({
                    "method": "item/completed",
                    "params": { "item": { "type": "agentMessage", "id": item_id, "text": text } }
                })),
                "agent-thread-1",
                &input,
                &mut events,
            )
            .unwrap();
    }
    collector
        .finalize_legacy_messages(&input, &mut events)
        .unwrap();
    let phases = events
        .iter()
        .filter_map(|event| match event {
            AgentAdapterEvent::AgentMessageCompleted {
                phase, is_final, ..
            } => Some((phase.clone(), *is_final)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(phases.len(), 2);
    assert_eq!(
        phases[0].0,
        crate::protocol::CadConversationPhase::Commentary
    );
    assert_eq!(
        phases[1].0,
        crate::protocol::CadConversationPhase::FinalAnswer
    );
    assert!(phases.iter().all(|(_, is_final)| *is_final));
}

#[test]
fn turn_completed_uses_authoritative_turn_status() {
    let completed = routed(
        1,
        json!({
            "method": "turn/completed",
            "params": { "threadId": "thread-1", "turn": { "id": "turn-1", "status": "completed" } }
        }),
    );
    assert!(matches!(terminal_result(&completed), Ok(Some(Ok(())))));

    let failed = routed(
        2,
        json!({
            "method": "turn/completed",
            "params": { "threadId": "thread-1", "turn": {
                "id": "turn-1", "status": "failed", "error": { "message": "boom" }
            } }
        }),
    );
    assert!(matches!(terminal_result(&failed), Ok(Some(Err(error))) if error == "boom"));

    let interrupted = routed(
        3,
        json!({
            "method": "turn/completed",
            "params": { "threadId": "thread-1", "turn": { "id": "turn-1", "status": "interrupted" } }
        }),
    );
    assert!(
        matches!(terminal_result(&interrupted), Ok(Some(Err(error))) if error.contains("interrupted"))
    );
}

#[test]
fn duplicate_completed_item_keeps_raw_transport_but_suppresses_normalized_replay() {
    let mut collector = CodexEventCollector::default();
    let mut events = Vec::new();
    let input = test_input();
    let completed = json!({
        "method": "item/completed",
        "params": { "item": {
            "type": "commandExecution", "id": "command-1",
            "command": "cadastrophe-plan-commit", "status": "completed"
        } }
    });
    for sequence in [1, 2] {
        collector
            .ingest(
                &routed(sequence, completed.clone()),
                "agent-thread-1",
                &input,
                &mut events,
            )
            .unwrap();
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentAdapterEvent::TransportNotification { .. }))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, AgentAdapterEvent::ToolCompleted { .. }))
            .count(),
        1
    );
}

fn routed(sequence: u64, raw: serde_json::Value) -> RoutedNotification {
    let params = &raw["params"];
    RoutedNotification {
        transport_sequence: sequence,
        method: raw["method"].as_str().unwrap().to_string(),
        identifiers: NotificationIdentifiers {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: params
                .get("itemId")
                .or_else(|| params.get("item").and_then(|item| item.get("id")))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
        },
        raw,
    }
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
