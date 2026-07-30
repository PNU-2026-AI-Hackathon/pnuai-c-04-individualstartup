use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct CodexEventCollector {
    assistant_items: HashMap<String, String>,
    final_assistant_text: Option<String>,
}

impl CodexEventCollector {
    pub(super) fn ingest(
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

    pub(super) fn assistant_text(&self) -> String {
        self.final_assistant_text
            .clone()
            .or_else(|| self.assistant_items.values().last().cloned())
            .unwrap_or_default()
    }
}

pub(super) fn map_item_started(
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

pub(super) fn extract_text(value: &Value) -> Option<String> {
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
