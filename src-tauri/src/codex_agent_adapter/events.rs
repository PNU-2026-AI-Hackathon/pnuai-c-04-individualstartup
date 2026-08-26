use crate::agent_adapter::{AgentAdapterEvent, AgentAdapterRunInput};
use crate::notification_router::RoutedNotification;
use crate::protocol::CadConversationPhase;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct CodexEventCollector {
    assistant_items: HashMap<String, String>,
    assistant_phases: HashMap<String, CadConversationPhase>,
    legacy_completed: Vec<LegacyCompletedMessage>,
    completed_agent_message_count: usize,
    started_item_ids: HashSet<String>,
    completed_item_ids: HashSet<String>,
}

struct LegacyCompletedMessage {
    thread_id: String,
    turn_id: String,
    item_id: String,
    content: String,
    sequence: u64,
    metadata: Option<serde_json::Map<String, Value>>,
}

impl CodexEventCollector {
    pub(super) fn ingest(
        &mut self,
        notification: &RoutedNotification,
        agent_thread_id: &str,
        input: &AgentAdapterRunInput,
        events: &mut Vec<AgentAdapterEvent>,
    ) -> Result<(), String> {
        let method = notification.method.as_str();
        let params = notification
            .raw
            .get("params")
            .cloned()
            .unwrap_or(Value::Null);
        let external_turn_id = notification
            .identifiers
            .turn_id
            .clone()
            .ok_or_else(|| format!("Routed notification {method} is missing turnId."))?;
        input.emit_event(
            events,
            AgentAdapterEvent::TransportNotification {
                agent_thread_id: agent_thread_id.to_string(),
                external_turn_id: external_turn_id.clone(),
                external_item_id: notification.identifiers.item_id.clone(),
                method: method.to_string(),
                sequence: notification.transport_sequence,
                payload: notification.raw.clone(),
            },
        )?;
        match method {
            "item/agentMessage/delta" => {
                if let (Some(item_id), Some(delta)) = (
                    params.get("itemId").and_then(Value::as_str),
                    params.get("delta").and_then(Value::as_str),
                ) {
                    let assistant_item =
                        self.assistant_items.entry(item_id.to_string()).or_default();
                    assistant_item.push_str(delta);
                    let phase = self
                        .assistant_phases
                        .get(item_id)
                        .cloned()
                        .unwrap_or(CadConversationPhase::Commentary);
                    let thread_id =
                        notification.identifiers.thread_id.clone().ok_or_else(|| {
                            "Routed agent message delta is missing threadId.".to_string()
                        })?;
                    input.emit_event(
                        events,
                        AgentAdapterEvent::AgentMessageDelta {
                            external_thread_id: thread_id,
                            external_turn_id,
                            external_item_id: item_id.to_string(),
                            phase,
                            delta: delta.to_string(),
                            sequence: notification.transport_sequence,
                        },
                    )?;
                }
            }
            "item/started" => {
                let item_id = params
                    .get("item")
                    .and_then(|item| item.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| "item/started is missing item.id.".to_string())?;
                if !self.started_item_ids.insert(item_id.to_string()) {
                    return Ok(());
                }
                self.capture_agent_message_phase(&params)?;
                map_item_started(&params, input, events)?;
            }
            "item/completed" => self.map_item_completed(&params, notification, input, events)?,
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
        notification: &RoutedNotification,
        input: &AgentAdapterRunInput,
        events: &mut Vec<AgentAdapterEvent>,
    ) -> Result<(), String> {
        let Some(item) = params.get("item") else {
            return Ok(());
        };
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "item/completed is missing item.id.".to_string())?;
        if !self.completed_item_ids.insert(item_id.to_string()) {
            return Ok(());
        }
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        match item_type {
            "agentMessage" => {
                self.completed_agent_message_count += 1;
                let completed_text = item
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("Completed agentMessage {item_id} contains no text."))?;
                let phase = parse_message_phase(item.get("phase"))?
                    .or_else(|| self.assistant_phases.get(item_id).cloned());
                let thread_id = notification.identifiers.thread_id.clone().ok_or_else(|| {
                    "Routed completed agent message is missing threadId.".to_string()
                })?;
                let turn_id = notification.identifiers.turn_id.clone().ok_or_else(|| {
                    "Routed completed agent message is missing turnId.".to_string()
                })?;
                let metadata = item_metadata("item/completed", item);
                if let Some(phase) = phase {
                    input.emit_event(
                        events,
                        AgentAdapterEvent::AgentMessageCompleted {
                            external_thread_id: thread_id,
                            external_turn_id: turn_id,
                            external_item_id: item_id.to_string(),
                            phase,
                            content: completed_text.clone(),
                            sequence: notification.transport_sequence,
                            is_final: true,
                            metadata,
                        },
                    )?;
                } else {
                    self.legacy_completed.push(LegacyCompletedMessage {
                        thread_id,
                        turn_id,
                        item_id: item_id.to_string(),
                        content: completed_text.clone(),
                        sequence: notification.transport_sequence,
                        metadata,
                    });
                }
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Assistant response ready".to_string(),
                        message: Some(format!("{} characters", completed_text.chars().count())),
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
                emit_completed_trace("MCP tool completed", item, input, events)?;
            }
            "commandExecution" => {
                let name = command_execution_label(item).unwrap_or_else(|| "command".to_string());
                input.emit_event(events, AgentAdapterEvent::ToolCompleted { name })?;
                emit_completed_trace("Command completed", item, input, events)?;
            }
            "reasoning" => {
                input.emit_event(
                    events,
                    AgentAdapterEvent::Progress {
                        label: "Reasoning completed".to_string(),
                        message: public_summary(item),
                        metadata: completed_trace_metadata(item),
                    },
                )?;
            }
            "fileChange" => emit_completed_trace("File change completed", item, input, events)?,
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

    pub(super) fn finalize_legacy_messages(
        &mut self,
        input: &AgentAdapterRunInput,
        events: &mut Vec<AgentAdapterEvent>,
    ) -> Result<(), String> {
        let last = self.legacy_completed.len().checked_sub(1);
        for (index, message) in self.legacy_completed.drain(..).enumerate() {
            let is_final = last == Some(index);
            input.emit_event(
                events,
                AgentAdapterEvent::AgentMessageCompleted {
                    external_thread_id: message.thread_id,
                    external_turn_id: message.turn_id,
                    external_item_id: message.item_id,
                    phase: if is_final {
                        CadConversationPhase::FinalAnswer
                    } else {
                        CadConversationPhase::Commentary
                    },
                    content: message.content,
                    sequence: message.sequence,
                    is_final: true,
                    metadata: message.metadata,
                },
            )?;
        }
        Ok(())
    }

    pub(super) fn has_completed_agent_message(&self) -> bool {
        self.completed_agent_message_count > 0
    }

    fn capture_agent_message_phase(&mut self, params: &Value) -> Result<(), String> {
        let Some(item) = params.get("item") else {
            return Ok(());
        };
        if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
            return Ok(());
        }
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Started agentMessage is missing item.id.".to_string())?;
        if let Some(phase) = parse_message_phase(item.get("phase"))? {
            self.assistant_phases.insert(item_id.to_string(), phase);
        }
        Ok(())
    }
}

fn emit_completed_trace(
    label: &str,
    item: &Value,
    input: &AgentAdapterRunInput,
    events: &mut Vec<AgentAdapterEvent>,
) -> Result<(), String> {
    input.emit_event(
        events,
        AgentAdapterEvent::Progress {
            label: label.to_string(),
            message: public_summary(item),
            metadata: completed_trace_metadata(item),
        },
    )
}

fn public_summary(item: &Value) -> Option<String> {
    for key in [
        "summary",
        "status",
        "aggregatedOutput",
        "output",
        "result",
        "error",
    ] {
        let Some(value) = item.get(key) else {
            continue;
        };
        let text = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        if !text.trim().is_empty() {
            return Some(truncate_text(&text, 2_000));
        }
    }
    None
}

fn completed_trace_metadata(item: &Value) -> Option<serde_json::Map<String, Value>> {
    let mut metadata = item_metadata("item/completed", item).unwrap_or_default();
    for key in ["status", "exitCode", "changes", "path", "paths"] {
        if let Some(value) = item.get(key) {
            metadata.insert(key.to_string(), bounded_public_value(value));
        }
    }
    Some(metadata)
}

fn bounded_public_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(truncate_text(text, 2_000)),
        Value::Array(values) => {
            Value::Array(values.iter().take(50).map(bounded_public_value).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .iter()
                .take(50)
                .map(|(key, value)| (key.clone(), bounded_public_value(value)))
                .collect(),
        ),
        value => value.clone(),
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}\n[truncated]")
    } else {
        truncated
    }
}

fn parse_message_phase(value: Option<&Value>) -> Result<Option<CadConversationPhase>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value == "commentary" => {
            Ok(Some(CadConversationPhase::Commentary))
        }
        Some(Value::String(value)) if value == "final_answer" => {
            Ok(Some(CadConversationPhase::FinalAnswer))
        }
        Some(value) => Err(format!("Unsupported Codex agentMessage phase: {value}.")),
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
