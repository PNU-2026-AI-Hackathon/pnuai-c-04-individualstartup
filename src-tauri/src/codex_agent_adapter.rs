use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent, AgentAdapterRunInput};
use crate::codex_process_client::{CodexProcessClient, CodexProcessConfig};
use crate::protocol::{CadConversationRole, CadSourceLanguage};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
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
        let thread = self
            .client
            .request(
                "thread/start",
                json!({
                    "approvalPolicy": "never",
                    "personality": "pragmatic",
                    "serviceName": "cadastrophe-tauri-backend",
                    "sessionStartSource": "startup"
                }),
            )
            .await?;
        let thread_id = thread
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex thread/start response did not include thread.id.".to_string())?;
        let prompt = build_cad_prompt(&input);
        let turn_input = json!({
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
            "outputSchema": cad_output_schema()
        });
        let turn = self.client.request("turn/start", turn_input).await?;
        let turn_id = turn
            .get("turn")
            .and_then(|turn| turn.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex turn/start response did not include turn.id.".to_string())?
            .to_string();

        let mut events = vec![
            AgentAdapterEvent::RunMetadata {
                external_agent: Some("codex".to_string()),
                external_thread_id: Some(thread_id.to_string()),
                external_turn_id: Some(turn_id.clone()),
            },
            AgentAdapterEvent::ToolStarted {
                name: "codex_turn".to_string(),
            },
        ];
        let mut collector = CodexEventCollector::default();
        let capture = timeout(self.turn_timeout, async {
            while let Ok(notification) = notifications.recv().await {
                collector.ingest(&notification, &mut events);
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

        let output = parse_cad_output(&collector.assistant_text())?;
        let source_language = parse_source_language(&output.source_language)?;
        events.push(AgentAdapterEvent::SourceUpdated {
            source_language,
            source: output.source.trim().to_string(),
        });
        events.push(AgentAdapterEvent::MessageCreated {
            role: CadConversationRole::Assistant,
            content: output
                .message
                .unwrap_or_else(|| "Updated the CAD source with Codex.".to_string()),
            metadata: Some(serde_json::Map::from_iter([(
                "codexThreadId".to_string(),
                Value::String(thread_id.to_string()),
            )])),
        });
        events.push(AgentAdapterEvent::ToolCompleted {
            name: "codex_turn".to_string(),
        });
        Ok(events)
    }
}

#[derive(Default)]
struct CodexEventCollector {
    assistant_items: HashMap<String, String>,
    final_assistant_text: Option<String>,
}

impl CodexEventCollector {
    fn ingest(&mut self, notification: &Value, events: &mut Vec<AgentAdapterEvent>) {
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
                    self.assistant_items
                        .entry(item_id.to_string())
                        .or_default()
                        .push_str(delta);
                }
            }
            "item/started" => map_item_started(&params, events),
            "item/completed" => self.map_item_completed(&params, events),
            "item/mcpToolCall/progress" => {
                if let Some(message) = params.get("message").and_then(Value::as_str) {
                    events.push(AgentAdapterEvent::ToolStarted {
                        name: message.to_string(),
                    });
                    events.push(AgentAdapterEvent::ToolCompleted {
                        name: message.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    fn map_item_completed(&mut self, params: &Value, events: &mut Vec<AgentAdapterEvent>) {
        let Some(item) = params.get("item") else {
            return;
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
            }
            "mcpToolCall" => {
                let name = item
                    .get("tool")
                    .and_then(Value::as_str)
                    .unwrap_or("mcp_tool")
                    .to_string();
                events.push(AgentAdapterEvent::ToolCompleted { name });
            }
            "commandExecution" => {
                let name = item
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or("command")
                    .to_string();
                events.push(AgentAdapterEvent::ToolCompleted { name });
            }
            _ => {}
        }
    }

    fn assistant_text(&self) -> String {
        self.final_assistant_text
            .clone()
            .or_else(|| self.assistant_items.values().last().cloned())
            .unwrap_or_default()
    }
}

fn map_item_started(params: &Value, events: &mut Vec<AgentAdapterEvent>) {
    let Some(item) = params.get("item") else {
        return;
    };
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    let name = match item_type {
        "mcpToolCall" => item.get("tool").and_then(Value::as_str),
        "commandExecution" => item.get("command").and_then(Value::as_str),
        "reasoning" => Some("reasoning"),
        _ => None,
    };
    if let Some(name) = name {
        events.push(AgentAdapterEvent::ToolStarted {
            name: name.to_string(),
        });
    }
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CadOutput {
    source_language: String,
    source: String,
    message: Option<String>,
}

fn parse_cad_output(text: &str) -> Result<CadOutput, String> {
    let candidate = extract_json_object(text)
        .ok_or_else(|| "Codex response did not include a JSON CAD output object.".to_string())?;
    let output: CadOutput = serde_json::from_str(&candidate)
        .map_err(|error| format!("Codex CAD output was not valid JSON: {error}"))?;
    if output.source.trim().is_empty() {
        return Err("Codex CAD output did not include source.".to_string());
    }
    Ok(output)
}

fn parse_source_language(value: &str) -> Result<CadSourceLanguage, String> {
    match value {
        "openscad" => Ok(CadSourceLanguage::Openscad),
        unsupported => Err(format!(
            "Codex returned unsupported sourceLanguage {:?}; expected \"openscad\".",
            unsupported
        )),
    }
}

fn extract_json_object(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }
    if let Some(block) = extract_fenced_json(trimmed) {
        return Some(block);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if start < end {
        Some(trimmed[start..=end].to_string())
    } else {
        None
    }
}

fn extract_fenced_json(text: &str) -> Option<String> {
    let fence_start = text.find("```")?;
    let after_fence = &text[fence_start + 3..];
    let after_language = after_fence
        .strip_prefix("json")
        .or_else(|| after_fence.strip_prefix("JSON"))
        .unwrap_or(after_fence);
    let content_start = after_language
        .find('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let rest = &after_language[content_start..];
    let fence_end = rest.find("```")?;
    Some(rest[..fence_end].trim().to_string())
}

fn build_cad_prompt(input: &AgentAdapterRunInput) -> String {
    format!(
        r#"You are the Cadastrophe CAD generation adapter.

Create or revise a concise OpenSCAD model for this user request:
{prompt}

Return only a JSON object that matches the provided schema. The JSON fields are:
- sourceLanguage: exactly "openscad"
- source: complete OpenSCAD source code
- message: one concise assistant message for the UI

OpenSCAD constraints for the current Cadastrophe preview runtime:
- Prefer cube, sphere, cylinder, translate, rotate, union, and difference.
- Include simple numeric parameters with comments like `width = 40; // @param min=8 max=80 step=1 label=Width` when useful.
- Do not include Markdown fences or explanatory prose outside the JSON object.

Cadastrophe context:
- sessionId: {session_id}
- runId: {run_id}
- activeRevisionId: {revision_id}
- activeRevisionSourceLanguage: {source_language}
- activeRevisionSource:
```openscad
{source}
```
"#,
        prompt = input.prompt,
        session_id = input.session_id,
        run_id = input.run_id,
        revision_id = input.revision_id.as_deref().unwrap_or(""),
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

fn cad_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["sourceLanguage", "source", "message"],
        "properties": {
            "sourceLanguage": {
                "type": "string",
                "enum": ["openscad"]
            },
            "source": {
                "type": "string",
                "minLength": 1
            },
            "message": {
                "type": "string",
                "minLength": 1
            }
        }
    })
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
    fn parses_structured_cad_output() {
        let output = parse_cad_output(
            r#"{"sourceLanguage":"openscad","source":"sphere(r = 10);","message":"Created a sphere."}"#,
        )
        .unwrap();

        assert_eq!(output.source_language, "openscad");
        assert_eq!(output.source, "sphere(r = 10);");
        assert_eq!(
            parse_source_language(&output.source_language).unwrap(),
            CadSourceLanguage::Openscad
        );
    }

    #[test]
    fn parses_fenced_json_fallback() {
        let output = parse_cad_output(
            r#"```json
{"sourceLanguage":"openscad","source":"cube([1, 2, 3]);","message":"Created a box."}
```"#,
        )
        .unwrap();

        assert_eq!(output.source, "cube([1, 2, 3]);");
    }

    #[test]
    fn collects_current_codex_agent_message_events() {
        let mut collector = CodexEventCollector::default();
        let mut events = Vec::new();
        collector.ingest(
            &json!({
                "method": "item/agentMessage/delta",
                "params": {
                    "itemId": "msg-1",
                    "delta": "{\"sourceLanguage\":\"openscad\","
                }
            }),
            &mut events,
        );
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
            &mut events,
        );

        assert!(collector.assistant_text().contains("sphere(r = 3)"));
    }
}
