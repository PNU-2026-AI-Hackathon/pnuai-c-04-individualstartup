use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent, AgentAdapterRunInput};
use crate::codex_process_client::{CodexProcessClient, CodexProcessConfig};
use crate::protocol::CadConversationRole;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::timeout;

mod events;
mod prompt;
#[cfg(test)]
mod tests;

use events::{extract_text, CodexEventCollector};
use prompt::{build_cad_prompt, build_thread_start_params, build_turn_start_params};

pub struct CodexAgentAdapter {
    client: CodexProcessClient,
    turn_timeout: Duration,
}

impl CodexAgentAdapter {
    pub fn new(client: CodexProcessClient) -> Self {
        Self {
            client,
            turn_timeout: duration_from_env("CADASTROPHE_CODEX_TURN_TIMEOUT_SECS", 900),
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
