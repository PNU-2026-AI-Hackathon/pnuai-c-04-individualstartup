use crate::agent_adapter::{AgentAdapter, AgentAdapterEvent, AgentAdapterRunInput};
use crate::agent_thread_manager::{
    AgentThreadManager, AgentThreadManagerConfig, RecoveredTurnStatus, StartManagedTurn,
};
use crate::codex_process_client::{CodexProcessClient, CodexProcessConfig};
use crate::notification_router::RoutedNotification;
use crate::session_service::SessionService;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

mod events;
mod prompt;
#[cfg(test)]
mod tests;

use events::{extract_text, CodexEventCollector};
use prompt::{build_modeling_turn_input, build_thread_start_params, build_turn_start_params};

pub struct CodexAgentAdapter {
    threads: AgentThreadManager,
    process_client: CodexProcessClient,
    turn_timeout: Duration,
}

impl CodexAgentAdapter {
    pub(crate) fn process_client(&self) -> CodexProcessClient {
        self.process_client.clone()
    }

    pub fn new(service: Arc<SessionService>, client: CodexProcessClient) -> Result<Self, String> {
        Ok(Self {
            threads: AgentThreadManager::new(
                service,
                client.clone(),
                AgentThreadManagerConfig::default(),
            )
            .map_err(|error| error.to_string())?,
            process_client: client,
            turn_timeout: duration_from_env("CADASTROPHE_CODEX_TURN_TIMEOUT_SECS", 900),
        })
    }

    pub fn from_env(service: Arc<SessionService>) -> Result<Self, String> {
        Self::new(
            service,
            CodexProcessClient::new(CodexProcessConfig::default()),
        )
    }

    pub async fn start_new_conversation(
        &self,
        session_id: &str,
    ) -> Result<crate::protocol::StartNewAgentConversationResult, String> {
        let cwd = codex_cwd()?;
        let thread_start_params = build_thread_start_params(&cwd)?;
        self.threads
            .start_new_conversation(session_id, thread_start_params)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn shutdown(&self) -> Result<(), String> {
        self.process_client.shutdown().await
    }
}

#[async_trait::async_trait]
impl AgentAdapter for CodexAgentAdapter {
    fn external_agent(&self) -> &'static str {
        "codex"
    }

    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String> {
        let cwd = codex_cwd()?;
        let turn_input = build_modeling_turn_input(&input)?;
        let thread_start_params = build_thread_start_params(&cwd)?;
        let replacement_context = format!(
            "The previous Codex thread could not be loaded. Continue this Cadastrophe session using the immutable scope --session '{}' --run '{}'. Re-read the persisted session/revision/workflow state before making changes.",
            input.session_id, input.run_id
        );
        let mut turn = self
            .threads
            .start_turn(StartManagedTurn {
                session_id: input.session_id.clone(),
                run_id: input.run_id.clone(),
                thread_start_params,
                turn_start_params: build_turn_start_params(&turn_input, &cwd, &input.app_data_dir),
                replacement_context: Some(replacement_context),
            })
            .await
            .map_err(|error| error.to_string())?;

        let mut events = Vec::new();
        let execution: Result<(), String> = async {
        input.emit_event(
            &mut events,
            AgentAdapterEvent::RunMetadata {
                external_agent: Some("codex".to_string()),
                external_thread_id: Some(turn.external_thread_id.clone()),
                external_turn_id: Some(turn.external_turn_id.clone()),
            },
        )?;
        input.emit_event(
            &mut events,
            AgentAdapterEvent::ToolStarted {
                name: "codex_turn".to_string(),
            },
        )?;

        let mut collector = CodexEventCollector::default();
        let capture = timeout(self.turn_timeout, async {
            loop {
                let Some(notification) = turn.notifications.recv().await else {
                    return Err("__codex_connection_closed__".to_string());
                };
                assert_turn_identity(&turn.external_thread_id, &turn.external_turn_id, &notification)?;
                collector.ingest(&notification, &turn.agent_thread_id, &input, &mut events)?;
                if let Some(terminal) = terminal_result(&notification)? {
                    return terminal;
                }
            }
        })
        .await;

        match capture {
            Ok(Err(error)) if error == "__codex_connection_closed__" => {
                let reconciliation = self
                    .threads
                    .reconcile_after_connection_loss(&turn.session_id, &turn.run_id)
                    .await
                    .map_err(|error| error.to_string())?;
                match reconciliation.status {
                    RecoveredTurnStatus::Completed => {}
                    RecoveredTurnStatus::Failed { message } => return Err(message),
                    RecoveredTurnStatus::Interrupted => {
                        return Err("Codex turn was interrupted during connection recovery.".to_string())
                    }
                    RecoveredTurnStatus::InProgress | RecoveredTurnStatus::NotFound => {
                        return Err("Codex turn outcome remains unknown after connection recovery.".to_string())
                    }
                }
            }
            Ok(result) => result?,
            Err(_) => {
                let interrupted = self
                    .threads
                    .interrupt_and_reconcile(&mut turn)
                    .await
                    .map_err(|error| error.to_string())?;
                for notification in interrupted.observed_notifications {
                    assert_turn_identity(
                        &turn.external_thread_id,
                        &turn.external_turn_id,
                        &notification,
                    )?;
                    collector.ingest(
                        &notification,
                        &turn.agent_thread_id,
                        &input,
                        &mut events,
                    )?;
                }
                match interrupted.reconciliation.status {
                    RecoveredTurnStatus::Completed => {}
                    RecoveredTurnStatus::Failed { message } => return Err(message),
                    RecoveredTurnStatus::Interrupted => {
                        return Err(format!(
                            "Codex turn timed out after {} seconds and was interrupted.",
                            self.turn_timeout.as_secs()
                        ))
                    }
                    RecoveredTurnStatus::InProgress => {
                        return Err("Codex turn remains in progress after timeout interrupt and history reconciliation.".to_string())
                    }
                    RecoveredTurnStatus::NotFound => {
                        return Err("Codex turn outcome is unknown after timeout interrupt and history reconciliation.".to_string())
                    }
                }
            }
        }

        if !collector.has_completed_agent_message() {
            let reconciliation = self
                .threads
                .reconcile_run(&turn.session_id, &turn.run_id)
                .await
                .map_err(|error| error.to_string())?;
            match reconciliation.status {
                RecoveredTurnStatus::Completed if !reconciliation.messages.is_empty() => {}
                RecoveredTurnStatus::Completed => {
                    return Err("Codex turn completed without an agent message, and thread history contained no completed agent message.".to_string())
                }
                RecoveredTurnStatus::Failed { message } => return Err(message),
                RecoveredTurnStatus::Interrupted => {
                    return Err("Codex turn history reported interruption after terminal notification.".to_string())
                }
                RecoveredTurnStatus::InProgress | RecoveredTurnStatus::NotFound => {
                    return Err("Codex terminal notification could not be reconciled with thread history.".to_string())
                }
            }
        }
        collector.finalize_legacy_messages(&input, &mut events)?;
        input.emit_event(
            &mut events,
            AgentAdapterEvent::ToolCompleted {
                name: "codex_turn".to_string(),
            },
        )?;
        Ok(())
        }.await;
        let cleanup = self
            .threads
            .finish_turn(&mut turn)
            .map_err(|error| error.to_string());
        match (execution, cleanup) {
            (Ok(()), Ok(())) => Ok(events),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(error), Err(cleanup_error)) => Err(format!(
                "{error} Additionally, Codex turn cleanup failed: {cleanup_error}"
            )),
        }
    }

    async fn interrupt_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        self.threads
            .interrupt_run_and_reconcile(session_id, run_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn reconcile_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        self.threads
            .reconcile_run(session_id, run_id)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn assert_turn_identity(
    thread_id: &str,
    turn_id: &str,
    notification: &RoutedNotification,
) -> Result<(), String> {
    if notification.identifiers.thread_id.as_deref() != Some(thread_id)
        || notification.identifiers.turn_id.as_deref() != Some(turn_id)
    {
        return Err(format!(
            "Routed Codex notification identity did not match collector route {thread_id}/{turn_id}: method={}, actualThreadId={:?}, actualTurnId={:?}, transportSequence={}.",
            notification.method,
            notification.identifiers.thread_id,
            notification.identifiers.turn_id,
            notification.transport_sequence,
        ));
    }
    Ok(())
}

fn terminal_result(
    notification: &RoutedNotification,
) -> Result<Option<Result<(), String>>, String> {
    match notification.method.as_str() {
        "turn/completed" => match notification
            .raw
            .pointer("/params/turn/status")
            .and_then(Value::as_str)
        {
            Some("completed") => Ok(Some(Ok(()))),
            Some("failed") => Ok(Some(Err(notification
                .raw
                .pointer("/params/turn/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Codex turn failed.")
                .to_string()))),
            Some("interrupted") => Ok(Some(Err("Codex turn was interrupted.".to_string()))),
            Some(status) => Err(format!("Unsupported terminal Codex turn status: {status}.")),
            None => Err("Codex turn/completed notification omitted turn.status.".to_string()),
        },
        "turn/failed" => Ok(Some(Err(extract_text(
            notification.raw.get("params").unwrap_or(&Value::Null),
        )
        .unwrap_or_else(|| "Codex turn failed.".to_string())))),
        "turn/interrupted" => Ok(Some(Err("Codex turn was interrupted.".to_string()))),
        _ => Ok(None),
    }
}

pub(crate) fn codex_cwd() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("CADASTROPHE_CODEX_CWD") {
        if value.trim().is_empty() {
            return Err("CADASTROPHE_CODEX_CWD cannot be empty when set.".to_string());
        }
        return Ok(PathBuf::from(value));
    }
    std::env::current_dir().map_err(|error| format!("Failed to resolve current directory: {error}"))
}

fn duration_from_env(name: &str, default_secs: u64) -> Duration {
    match std::env::var(name) {
        Ok(value) => match value.parse::<u64>() {
            Ok(seconds) if seconds > 0 => Duration::from_secs(seconds),
            _ => panic!("{name} must be a positive integer number of seconds."),
        },
        Err(std::env::VarError::NotPresent) => Duration::from_secs(default_secs),
        Err(error) => panic!("Failed to read {name}: {error}"),
    }
}
