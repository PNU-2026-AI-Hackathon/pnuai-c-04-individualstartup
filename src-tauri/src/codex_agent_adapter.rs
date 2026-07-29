use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent, AgentAdapterRunInput};
use crate::codex_process_client::{CodexProcessClient, CodexProcessConfig};
use crate::protocol::{CadConversationRole, CadSourceLanguage};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::timeout;

pub struct CodexAgentAdapter {
    client: CodexProcessClient,
    turn_timeout: Duration,
}

impl CodexAgentAdapter {
    pub fn new(client: CodexProcessClient) -> Self {
        Self {
            client,
            turn_timeout: duration_from_env("CADASTROPHE_CODEX_TURN_TIMEOUT_SECS", 180),
        }
    }

    pub fn from_env() -> Self {
        Self::new(CodexProcessClient::new(CodexProcessConfig::default()))
    }
}

#[async_trait::async_trait]
impl AgentAdapter for CodexAgentAdapter {
    fn external_agent(&self) -> &'static str {
        "codex"
    }

    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        self.client.initialize().await?;
        let mut notifications = self.client.subscribe_notifications();
        let cwd = codex_cwd();
        let thread = self
            .client
            .request("thread/start", build_thread_start_params(&cwd))
            .await?;
        let thread_id = thread
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex thread/start response did not include thread.id.".to_string())?;
        let prompt = build_cad_prompt(&input);
        let turn_input = build_turn_start_params(thread_id, &prompt, &cwd, &input.app_data_dir);
        let turn = self.client.request("turn/start", turn_input).await?;
        let turn_id = turn
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex turn/start response did not include turn.id.".to_string())?
            .to_string();

        let mut events = Vec::new();
        input.emit_event(
            &mut events,
            AgentAdapterEvent::RunMetadata {
                external_agent: Some("codex".to_string()),
                external_thread_id: Some(thread_id.to_string()),
                external_turn_id: Some(turn_id.clone()),
            },
        )?;
        input.emit_event(
            &mut events,
            AgentAdapterEvent::ToolStarted {
                name: "codex_turn".to_string(),
            },
        )?;
        input.emit_event(
            &mut events,
            AgentAdapterEvent::Progress {
                label: "Codex turn started".to_string(),
                message: Some("Waiting for Codex workflow events.".to_string()),
                metadata: Some(serde_json::Map::from_iter([
                    (
                        "codexThreadId".to_string(),
                        Value::String(thread_id.to_string()),
                    ),
                    ("codexTurnId".to_string(), Value::String(turn_id.clone())),
                ])),
            },
        )?;
        let mut collector = CodexEventCollector::default();
        let capture = timeout(self.turn_timeout, async {
            while let Ok(notification) = notifications.recv().await {
                collector.ingest(&notification, &input, &mut events)?;
                let method = notification
                    .get("method")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if method == "turn/failed" || method == "error" {
                    return Err(
                        extract_text(notification.get("params").unwrap_or(&Value::Null))
                            .unwrap_or_else(|| "Codex turn failed.".to_string()),
                    );
                }
                if method == "turn/completed" {
                    return Ok(());
                }
            }
            Err("Codex app-server notification stream closed before turn completion.".to_string())
        })
        .await;
        capture.map_err(|_| {
            let client = self.client.clone();
            let thread_id = thread_id.to_string();
            let turn_id = turn_id.clone();
            tokio::spawn(async move {
                let _ = client
                    .request(
                        "turn/interrupt",
                        json!({ "threadId": thread_id, "turnId": turn_id }),
                    )
                    .await;
            });
            format!(
                "Timed out waiting {} seconds for Codex turn completion.",
                self.turn_timeout.as_secs()
            )
        })??;

        let assistant_text = collector.assistant_text();
        input.emit_event(
            &mut events,
            AgentAdapterEvent::MessageCreated {
                role: CadConversationRole::Assistant,
                content: if assistant_text.trim().is_empty() {
                    "Codex workflow turn completed.".to_string()
                } else {
                    assistant_text.trim().to_string()
                },
                metadata: Some(serde_json::Map::from_iter([(
                    "codexThreadId".to_string(),
                    Value::String(thread_id.to_string()),
                )])),
            },
        )?;
        input.emit_event(
            &mut events,
            AgentAdapterEvent::ToolCompleted {
                name: "codex_turn".to_string(),
            },
        )?;
        Ok(events)
    }
}

#[derive(Default)]
struct CodexEventCollector {
    assistant_items: HashMap<String, String>,
    final_assistant_text: Option<String>,
}

impl CodexEventCollector {
    fn ingest(
        &mut self,
        notification: &Value,
        input: &AgentAdapterRunInput,
        events: &mut Vec<AgentAdapterEvent>,
    ) -> Result<(), String> {
        let method = notification
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = notification.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "item/agentMessage/delta" => {
                if let (Some(item_id), Some(delta)) = (
                    params.get("itemId").and_then(Value::as_str),
                    params.get("delta").and_then(Value::as_str),
                ) {
                    let assistant_item =
                        self.assistant_items.entry(item_id.to_string()).or_default();
                    let previous_len = assistant_item.chars().count();
                    assistant_item.push_str(delta);
                    let current_len = assistant_item.chars().count();
                    if previous_len == 0 || current_len / 500 > previous_len / 500 {
                        input.emit_event(
                            events,
                            AgentAdapterEvent::Progress {
                                label: "Receiving assistant response".to_string(),
                                message: Some(format!("{current_len} characters received")),
                                metadata: Some(serde_json::Map::from_iter([
                                    ("codexMethod".to_string(), Value::String(method.to_string())),
                                    ("itemId".to_string(), Value::String(item_id.to_string())),
                                ])),
                            },
                        )?;
                    }
                }
            }
            "item/started" => map_item_started(&params, input, events)?,
            "item/completed" => self.map_item_completed(&params, input, events)?,
            "item/mcpToolCall/progress" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    input.emit_event(
                        events,
                        AgentAdapterEvent::Progress {
                            label: message.to_string(),
                            message: Some(message.to_string()),
                            metadata: Some(serde_json::Map::from_iter([(
                                "codexMethod".to_string(),
                                Value::String(method.to_string()),
                            )])),
                        },
                    )?;
                }
            }
            "turn/completed" => {
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Codex turn completed".to_string(),
                        message: None,
                        metadata: Some(serde_json::Map::from_iter([(
                            "codexMethod".to_string(),
                            Value::String(method.to_string()),
                        )])),
                    },
                )?;
            }
            "turn/failed" | "error" => {
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Codex turn failed".to_string(),
                        message: extract_text(&params),
                        metadata: Some(serde_json::Map::from_iter([(
                            "codexMethod".to_string(),
                            Value::String(method.to_string()),
                        )])),
                    },
                )?;
            }
            _ => {
                if method.starts_with("turn/") {
                    input.emit_event(
                        events,
                        AgentAdapterEvent::Progress {
                            label: method.replace('/', " "),
                            message: None,
                            metadata: Some(serde_json::Map::from_iter([(
                                "codexMethod".to_string(),
                                Value::String(method.to_string()),
                            )])),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn map_item_completed(
        &mut self,
        params: &Value,
        input: &AgentAdapterRunInput,
        events: &mut Vec<AgentAdapterEvent>,
    ) -> Result<(), String> {
        let Some(item) = params.get("item") else {
            return Ok(());
        };
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        match item_type {
            "agentMessage" => {
                let completed_text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        item.get("id")
                            .and_then(Value::as_str)
                            .and_then(|item_id| self.assistant_items.get(item_id).cloned())
                    });
                if let Some(text) = completed_text {
                    if !text.trim().is_empty() {
                        self.final_assistant_text = Some(text);
                    }
                }
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Assistant response ready".to_string(),
                        message: self
                            .final_assistant_text
                            .as_ref()
                            .map(|text| format!("{} characters", text.chars().count())),
                        metadata: item_metadata("item/completed", item),
                    },
                )?;
            }
            "mcpToolCall" => {
                let name = item
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("mcp_tool")
                    .to_string();
                input.emit_event(events, AgentAdapterEvent::ToolCompleted { name })?;
            }
            "commandExecution" => {
                let name = command_execution_label(item).unwrap_or_else(|| "command".to_string());
                input.emit_event(events, AgentAdapterEvent::ToolCompleted { name })?;
            }
            "reasoning" => {
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Reasoning completed".to_string(),
                        message: None,
                        metadata: item_metadata("item/completed", item),
                    },
                )?;
            }
            _ => {
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: format!("{} completed", item_type.replace('_', " ")),
                        message: None,
                        metadata: item_metadata("item/completed", item),
                    },
                )?;
            }
        }
        Ok(())
    }

    fn assistant_text(&self) -> String {
        self.final_assistant_text
            .clone()
            .or_else(|| self.assistant_items.values().last().cloned())
            .unwrap_or_default()
    }
}

fn map_item_started(
    params: &Value,
    input: &AgentAdapterRunInput,
    events: &mut Vec<AgentAdapterEvent>,
) -> Result<(), String> {
    let Some(item) = params.get("item") else {
        return Ok(());
    };
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let name = match item_type {
        "mcpToolCall" => item
            .get("tool")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        "commandExecution" => command_execution_label(item),
        "reasoning" => Some("reasoning".to_string()),
        _ => None,
    };
    if let Some(name) = name {
        input.emit_event(events, AgentAdapterEvent::ToolStarted { name })?;
    } else if !item_type.is_empty() {
        input.emit_event(
            events,
            AgentAdapterEvent::Progress {
                label: format!("{} started", item_type.replace('_', " ")),
                message: None,
                metadata: item_metadata("item/started", item),
            },
        )?;
    }
    Ok(())
}

fn command_execution_label(item: &Value) -> Option<String> {
    if let Some(command) = item.get("command").and_then(Value::as_str) {
        return Some(command.to_string());
    }
    if let Some(command) = item.get("command").and_then(Value::as_array) {
        let parts: Vec<String> = command
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect();
        if !parts.is_empty() {
            return Some(parts.join(" "));
        }
    }
    item.get("cmd")
        .or_else(|| item.get("commandLine"))
        .or_else(|| item.get("displayCommand"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn extract_text(value: &Value) -> Option<String> {
    value
        .get("text")
        .or_else(|| value.get("content"))
        .or_else(|| value.get("message"))
        .or_else(|| value.get("delta"))
        .or_else(|| value.get("item").and_then(|item| item.get("text")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

fn item_metadata(method: &str, item: &Value) -> Option<serde_json::Map<String, Value>> {
    let mut metadata = serde_json::Map::from_iter([(
        "codexMethod".to_string(),
        Value::String(method.to_string()),
    )]);
    if let Some(item_id) = item.get("id").and_then(Value::as_str) {
        metadata.insert("itemId".to_string(), Value::String(item_id.to_string()));
    }
    if let Some(item_type) = item.get("type").and_then(Value::as_str) {
        metadata.insert("itemType".to_string(), Value::String(item_type.to_string()));
    }
    if let Some(command) = command_execution_label(item) {
        metadata.insert("command".to_string(), Value::String(command));
    }
    if let Some(tool) = item.get("tool").and_then(Value::as_str) {
        metadata.insert("tool".to_string(), Value::String(tool.to_string()));
    }
    Some(metadata)
}

fn build_thread_start_params(cwd: &Path) -> Value {
    json!({
        "approvalPolicy": "never",
        "cwd": cwd,
        "personality": "pragmatic",
        "sandbox": "workspace-write",
        "serviceName": "cadastrophe-tauri-backend",
        "sessionStartSource": "startup"
    })
}

fn build_turn_start_params(
    thread_id: &str,
    prompt: &str,
    cwd: &Path,
    app_data_dir: &Path,
) -> Value {
    json!({
        "threadId": thread_id,
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

fn build_cad_prompt(input: &AgentAdapterRunInput) -> String {
    let latest_failure_report = input
        .latest_workflow_failure_report
        .as_ref()
        .map(|report| serde_json::to_string_pretty(report).unwrap_or_else(|_| report.to_string()))
        .unwrap_or_else(|| "null".to_string());
    let app_data_dir = shell_quote_path(&input.app_data_dir);
    format!(
        r#"You are the Cadastrophe Milestone 3 workflow agent.

Drive this Cadastrophe CAD request through the repository CLI workflow. Do not return a standalone JSON source object. The CLI tools own persisted plan, revision, preview, finalization, structural, and VLM workflow state.

User request:
{prompt}

Required order:
1. Inspect current state with `cadastrophe-session-state --app-data-dir {app_data_dir} --session {session_id}` when useful, especially on retries.
2. Create a CadModelPlan JSON file under the Cadastrophe app data directory, then commit it with `cadastrophe-plan-commit --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --plan <file>`.
3. Do not call `cadastrophe-source-apply`, `cadastrophe-preview-render`, `cadastrophe-evaluate-structural`, `cadastrophe-finalize`, or `cadastrophe-vlm-submit` until the plan commit succeeds for this run.
4. Write complete OpenSCAD source to a file under the Cadastrophe app data directory, then call `cadastrophe-source-apply --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --source <file> --language openscad`.
5. Render and inspect diagnostics with `cadastrophe-preview-render --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --revision <revision_id>`.
6. If runtime diagnostics contain errors, explain the cause briefly, repair the source, and repeat source apply then preview render.
7. When runtime diagnostics pass, call `cadastrophe-finalize --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --revision <revision_id>`.
8. Finalize runs the deterministic structural anchor. If it returns a `failure_report` or `next_action`/`nextAction` of `outer_loop_refine_source`, do not call VLM. Include that failure report in the next plan/source attempt and repeat from plan commit.
9. If finalize returns a `cadastrophe.vlm_judge.v1` contract or pending VLM state, hand that exact contract to a separate subagent using the `cadastrophe-vlm-judge` skill and request strict JSON only.
10. Save the judge JSON under the Cadastrophe app data directory, then submit it with `cadastrophe-vlm-submit --app-data-dir {app_data_dir} --session {session_id} --run {run_id} --artifact <artifact_id> --report <file>`.
11. If VLM submit fails, use its `failure_report` for outer-loop refinement and repeat from plan commit. If it passes, finish the run with a concise assistant message naming the final revision/artifact.

CLI contract reminders:
- JSON output is the default; use `--pretty` only for human inspection.
- Always pass `--app-data-dir {app_data_dir}` to Cadastrophe CLI commands. The UI and CLI must operate on the same SQLite database and artifacts.
- Treat non-zero exits and JSON error envelopes as authoritative.
- Preserve command outputs that include `next_action`, `nextAction`, `diagnostics`, `failure_report`, `failureReport`, `artifact_paths`, `artifactPaths`, `contract_type`, or `contractType`; these fields are shown in the UI run log.
- A retry must inspect previous workflow failures via `cadastrophe-session-state --app-data-dir {app_data_dir} --session {session_id}` and carry the latest structural or VLM failure report into the new attempt.

OpenSCAD constraints for the current Cadastrophe openscad-wasm runtime:
- Use OpenSCAD CSG normally, including cube, sphere, cylinder, translate, rotate, union, and difference.
- Include simple numeric parameters with comments like `width = 40; // @param min=8 max=80 step=1 label=Width` when useful.
- Include a `// @main_component <name>` header matching the committed plan's `mainComponent.name`.
- Do not include file IO, `include`, `use`, or host-dependent paths.

Cadastrophe context:
- appDataDir: {app_data_dir}
- sessionId: {session_id}
- runId: {run_id}
- activeRevisionId: {revision_id}
- activeRevisionSourceLanguage: {source_language}
- latestWorkflowFailureReport:
```json
{latest_failure_report}
```
- activeRevisionSource:
```openscad
{source}
```
"#,
        prompt = input.prompt,
        app_data_dir = app_data_dir,
        session_id = input.session_id,
        run_id = input.run_id,
        revision_id = input.revision_id.as_deref().unwrap_or(""),
        latest_failure_report = latest_failure_report,
        source_language = input
            .revision_source_language
            .as_ref()
            .map(|language| match language {
                CadSourceLanguage::Openscad => "openscad",
                CadSourceLanguage::Cadquery => "cadquery",
                CadSourceLanguage::FreecadPython => "freecad-python",
                CadSourceLanguage::CadastropheIr => "cadastrophe-ir",
            })
            .unwrap_or(""),
        source = input.revision_source.as_deref().unwrap_or("")
    )
}

fn shell_quote_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn codex_cwd() -> PathBuf {
    std::env::var("CADASTROPHE_CODEX_CWD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn duration_from_env(name: &str, default_secs: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_requires_milestone_3_cli_workflow_order() {
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
        assert!(prompt.contains(
            "cadastrophe-vlm-submit --app-data-dir '/tmp/Cad App Data' --session session-1 --run run-1"
        ));
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
}
