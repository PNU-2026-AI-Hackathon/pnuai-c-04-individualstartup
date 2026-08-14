use super::prompt::{
    build_validation_thread_start_params, build_validation_turn_start_params,
    render_vlm_evaluator_prompt, ValidationPromptContext,
};
use crate::agent_thread_manager::{
    AgentThreadManager, AgentThreadManagerConfig, ManagedAgentTurn, RecoveredTurnStatus,
    StartScopedTurn,
};
use crate::codex_process_client::CodexProcessClient;
use crate::notification_router::RoutedNotification;
use crate::protocol::{
    CadAgentPlane, CadValidationCheck, CadValidationCheckEvent, CadValidationCheckKind,
    CadValidationCheckStatus, CadValidationEvaluation, CadValidationEvaluationEvent,
    CadValidationEvaluationStatus, ThreadScope,
};
use crate::session_service::{timestamp, SessionService};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct CodexVlmEvaluatorConfig {
    pub turn_timeout: Duration,
}

impl Default for CodexVlmEvaluatorConfig {
    fn default() -> Self {
        Self {
            turn_timeout: Duration::from_secs(300),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CodexVlmEvaluationInput {
    pub evaluation: CadValidationEvaluation,
    pub rendered_image_path: PathBuf,
    pub cwd: PathBuf,
    pub app_data_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct CodexVlmCheckInput {
    pub check: CadValidationCheck,
    pub rendered_image_path: PathBuf,
    pub cwd: PathBuf,
    pub app_data_dir: PathBuf,
}

#[derive(Clone)]
pub struct CodexVlmEvaluator {
    service: Arc<SessionService>,
    threads: AgentThreadManager,
    config: CodexVlmEvaluatorConfig,
}

impl CodexVlmEvaluator {
    pub async fn evaluate_check(&self, input: CodexVlmCheckInput) -> Result<Value, String> {
        if input.check.kind != CadValidationCheckKind::Vlm
            || input.check.status != CadValidationCheckStatus::Queued
        {
            return Err(format!(
                "Only a queued VLM check can be evaluated: {}",
                input.check.id
            ));
        }
        let contract = input
            .check
            .input_contract
            .as_object()
            .ok_or_else(|| "VLM check input contract must be a JSON object.".to_string())?;
        let batch = self
            .service
            .get_validation_batch(&input.check.session_id, &input.check.batch_id)?
            .ok_or_else(|| format!("Validation batch not found: {}", input.check.batch_id))?;
        let prompt = render_vlm_evaluator_prompt(&ValidationPromptContext {
            evaluation_id: &input.check.id,
            session_id: &input.check.session_id,
            run_id: &batch.run_id,
            revision_id: &batch.revision_id,
            artifact_id: &batch.artifact_id,
            evaluation_contract: &input.check.input_contract,
        })?;
        if contract.get("evaluationId").and_then(Value::as_str) != Some(input.check.id.as_str()) {
            return Err("VLM check input evaluationId does not match check id.".to_string());
        }
        let thread_start_params = build_validation_thread_start_params(&input.cwd)?;
        let turn_start_params = build_validation_turn_start_params(
            &prompt,
            &input.rendered_image_path,
            &input.cwd,
            &input.app_data_dir,
        )?;
        let scope = ThreadScope {
            session_id: input.check.session_id.clone(),
            plane: CadAgentPlane::Validation,
            owner_id: input.check.id.clone(),
        };
        let service = Arc::clone(&self.service);
        let bind_session_id = input.check.session_id.clone();
        let bind_check_id = input.check.id.clone();
        let mut turn = self
            .threads
            .start_scoped_turn(StartScopedTurn {
                scope,
                thread_start_params,
                turn_start_params,
                bind: Arc::new(move |binding| {
                    service
                        .bind_validation_check(
                            &bind_session_id,
                            &bind_check_id,
                            &binding.agent_thread_id,
                            &binding.external_turn_id,
                        )
                        .map(|_| ())
                }),
            })
            .await
            .map_err(|error| error.to_string())?;
        let result = self
            .collect_check_until_terminal(&input.check, &mut turn)
            .await;
        let cleanup = self
            .threads
            .finish_turn(&mut turn)
            .map_err(|error| error.to_string());
        match (result, cleanup) {
            (Ok(report), Ok(())) => Ok(report),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(cleanup)) => Err(format!(
                "{error} Additionally, validation Codex turn cleanup failed: {cleanup}"
            )),
        }
    }

    pub async fn recover_check(&self, check: &CadValidationCheck) -> Result<Value, String> {
        if check.kind != CadValidationCheckKind::Vlm
            || check.status != CadValidationCheckStatus::Running
        {
            return Err(format!(
                "Only a running VLM check can be recovered: {}",
                check.id
            ));
        }
        let thread_id = check
            .evaluator_thread_id
            .as_deref()
            .ok_or_else(|| format!("Running VLM check {} has no evaluator thread.", check.id))?;
        let turn_id = check
            .external_turn_id
            .as_deref()
            .ok_or_else(|| format!("Running VLM check {} has no external turn.", check.id))?;
        let mut thread = self
            .service
            .list_agent_threads(&check.session_id)?
            .into_iter()
            .find(|thread| thread.id == thread_id)
            .ok_or_else(|| format!("Validation evaluator thread not found: {thread_id}"))?;
        if thread.owner_id != check.id || thread.plane != CadAgentPlane::Validation {
            return Err(format!(
                "Validation evaluator thread scope mismatch for check {}.",
                check.id
            ));
        }
        let result = self
            .threads
            .read_thread_history(&thread.external_thread_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                format!(
                    "Validation Codex thread {} was not found during recovery.",
                    thread.external_thread_id
                )
            })
            .and_then(|value| parse_strict_validation_history(&value, turn_id));
        if thread.archived_at.is_none() && thread.replaced_by_id.is_none() {
            let now = timestamp();
            thread.status = crate::protocol::CadAgentThreadStatus::Archived;
            thread.updated_at = now.clone();
            thread.archived_at = Some(now);
            self.service.upsert_agent_thread(thread)?;
        }
        result
    }

    async fn collect_check_until_terminal(
        &self,
        check: &CadValidationCheck,
        turn: &mut ManagedAgentTurn,
    ) -> Result<Value, String> {
        let mut collector = StrictValidationCollector::default();
        let capture = timeout(self.config.turn_timeout, async {
            loop {
                let notification = turn.notifications.recv().await.ok_or_else(|| {
                    "Codex validation notification route closed before a terminal event."
                        .to_string()
                })?;
                assert_notification_identity(turn, &notification)?;
                self.persist_check_notification(check, turn, &notification)?;
                if let Some(report) = collector.ingest(
                    &turn.external_thread_id,
                    &turn.external_turn_id,
                    &notification,
                )? {
                    return Ok(report);
                }
            }
        })
        .await;
        match capture {
            Ok(result) => result,
            Err(_) => {
                let reconciliation = self.threads.interrupt_and_reconcile(turn).await.map_err(|error| format!(
                    "Validation Codex turn timed out after {} seconds; interrupt/reconciliation failed: {error}", self.config.turn_timeout.as_secs_f64()
                ))?;
                for notification in &reconciliation.observed_notifications {
                    self.persist_check_notification(check, turn, notification)?;
                    collector.ingest(
                        &turn.external_thread_id,
                        &turn.external_turn_id,
                        notification,
                    )?;
                }
                Err(format!("Validation Codex turn timed out after {} seconds and was interrupted; reconciled status: {}.", self.config.turn_timeout.as_secs_f64(), reconciliation_status_label(&reconciliation.reconciliation.status)))
            }
        }
    }

    fn persist_check_notification(
        &self,
        check: &CadValidationCheck,
        turn: &ManagedAgentTurn,
        notification: &RoutedNotification,
    ) -> Result<(), String> {
        assert_notification_identity(turn, notification)?;
        self.service
            .save_validation_check_event(CadValidationCheckEvent {
                id: Uuid::new_v4().to_string(),
                session_id: check.session_id.clone(),
                check_id: check.id.clone(),
                evaluator_thread_id: turn.agent_thread_id.clone(),
                external_turn_id: Some(turn.external_turn_id.clone()),
                external_item_id: notification.identifiers.item_id.clone(),
                method: notification.method.clone(),
                sequence: notification.transport_sequence,
                payload: crate::agent_gateway::normalize_transport_payload(&notification.raw),
                created_at: timestamp(),
            })
            .map(|_| ())
    }
    pub fn new(
        service: Arc<SessionService>,
        client: CodexProcessClient,
        config: CodexVlmEvaluatorConfig,
    ) -> Result<Self, String> {
        let threads = AgentThreadManager::new(
            Arc::clone(&service),
            client,
            AgentThreadManagerConfig::default(),
        )
        .map_err(|error| error.to_string())?;
        Self::with_thread_manager(service, threads, config)
    }

    pub(crate) fn with_thread_manager(
        service: Arc<SessionService>,
        threads: AgentThreadManager,
        config: CodexVlmEvaluatorConfig,
    ) -> Result<Self, String> {
        if config.turn_timeout.is_zero() {
            return Err("Validation Codex turn timeout must be greater than zero.".to_string());
        }
        Ok(Self {
            service,
            threads,
            config,
        })
    }

    pub async fn evaluate(&self, input: CodexVlmEvaluationInput) -> Result<Value, String> {
        self.validate_evaluation_snapshot(&input.evaluation)?;
        let prompt = render_vlm_evaluator_prompt(&ValidationPromptContext {
            evaluation_id: &input.evaluation.id,
            session_id: &input.evaluation.session_id,
            run_id: &input.evaluation.run_id,
            revision_id: &input.evaluation.revision_id,
            artifact_id: &input.evaluation.artifact_id,
            evaluation_contract: &input.evaluation.input_contract,
        })?;
        let thread_start_params = build_validation_thread_start_params(&input.cwd)?;
        let turn_start_params = build_validation_turn_start_params(
            &prompt,
            &input.rendered_image_path,
            &input.cwd,
            &input.app_data_dir,
        )?;
        let scope = ThreadScope {
            session_id: input.evaluation.session_id.clone(),
            plane: CadAgentPlane::Validation,
            owner_id: input.evaluation.id.clone(),
        };
        let service = Arc::clone(&self.service);
        let bind_session_id = input.evaluation.session_id.clone();
        let bind_evaluation_id = input.evaluation.id.clone();
        let mut turn = self
            .threads
            .start_scoped_turn(StartScopedTurn {
                scope,
                thread_start_params,
                turn_start_params,
                bind: Arc::new(move |binding| {
                    service
                        .bind_validation_evaluation(
                            &bind_session_id,
                            &bind_evaluation_id,
                            &binding.agent_thread_id,
                            &binding.external_turn_id,
                        )
                        .map(|_| ())
                }),
            })
            .await
            .map_err(|error| error.to_string())?;

        let result = self
            .collect_until_terminal(&input.evaluation, &mut turn)
            .await;
        let cleanup = self
            .threads
            .finish_turn(&mut turn)
            .map_err(|error| error.to_string());
        match (result, cleanup) {
            (Ok(report), Ok(())) => Ok(report),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(error), Err(cleanup_error)) => Err(format!(
                "{error} Additionally, validation Codex turn cleanup failed: {cleanup_error}"
            )),
        }
    }

    pub async fn recover(&self, evaluation: &CadValidationEvaluation) -> Result<Value, String> {
        if evaluation.status != CadValidationEvaluationStatus::Running {
            return Err(format!(
                "Only a running validation evaluation can be recovered: {}",
                evaluation.id
            ));
        }
        let persisted = self
            .service
            .get_validation_evaluation(&evaluation.session_id, &evaluation.id)?
            .ok_or_else(|| format!("Validation evaluation not found: {}", evaluation.id))?;
        if &persisted != evaluation {
            return Err(format!(
                "Validation evaluation recovery snapshot is stale: {}",
                evaluation.id
            ));
        }
        let agent_thread_id = evaluation.evaluator_thread_id.as_deref().ok_or_else(|| {
            format!(
                "Running validation evaluation {} has no evaluator thread id.",
                evaluation.id
            )
        })?;
        let external_turn_id = evaluation.external_turn_id.as_deref().ok_or_else(|| {
            format!(
                "Running validation evaluation {} has no external turn id.",
                evaluation.id
            )
        })?;
        let mut thread = self
            .service
            .list_agent_threads(&evaluation.session_id)?
            .into_iter()
            .find(|thread| thread.id == agent_thread_id)
            .ok_or_else(|| format!("Validation evaluator thread not found: {agent_thread_id}"))?;
        if thread.plane != CadAgentPlane::Validation || thread.owner_id != evaluation.id {
            return Err(format!(
                "Validation evaluator thread scope mismatch for evaluation {}.",
                evaluation.id
            ));
        }
        let recovery = self
            .threads
            .read_thread_history(&thread.external_thread_id)
            .await
            .map_err(|error| error.to_string())
            .and_then(|response| {
                response
                    .ok_or_else(|| {
                        format!(
                            "Validation Codex thread {} was not found during recovery.",
                            thread.external_thread_id
                        )
                    })
                    .and_then(|response| {
                        parse_strict_validation_history(&response, external_turn_id)
                    })
            });
        let archive = if thread.archived_at.is_none() && thread.replaced_by_id.is_none() {
            let now = timestamp();
            thread.status = crate::protocol::CadAgentThreadStatus::Archived;
            thread.updated_at = now.clone();
            thread.archived_at = Some(now);
            self.service.upsert_agent_thread(thread).map(|_| ())
        } else {
            Ok(())
        };
        match (recovery, archive) {
            (Ok(report), Ok(())) => Ok(report),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(archive_error)) => Err(format!(
                "{error} Additionally, recovered validation thread archival failed: {archive_error}"
            )),
        }
    }

    fn validate_evaluation_snapshot(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> Result<(), String> {
        if evaluation.status != CadValidationEvaluationStatus::Queued {
            return Err(format!(
                "Validation evaluation must be queued before Codex starts: {}",
                evaluation.id
            ));
        }
        let persisted = self
            .service
            .get_validation_evaluation(&evaluation.session_id, &evaluation.id)?
            .ok_or_else(|| format!("Validation evaluation not found: {}", evaluation.id))?;
        if &persisted != evaluation {
            return Err(format!(
                "Validation evaluation snapshot is stale or does not match persistence: {}",
                evaluation.id
            ));
        }
        Ok(())
    }

    async fn collect_until_terminal(
        &self,
        evaluation: &CadValidationEvaluation,
        turn: &mut ManagedAgentTurn,
    ) -> Result<Value, String> {
        let mut collector = StrictValidationCollector::default();
        let capture = timeout(self.config.turn_timeout, async {
            loop {
                let notification = turn.notifications.recv().await.ok_or_else(|| {
                    "Codex validation notification route closed before a terminal event."
                        .to_string()
                })?;
                self.persist_notification(evaluation, turn, &notification)?;
                if let Some(report) = collector.ingest(
                    &turn.external_thread_id,
                    &turn.external_turn_id,
                    &notification,
                )? {
                    return Ok(report);
                }
            }
        })
        .await;
        match capture {
            Ok(result) => result,
            Err(_) => {
                let reconciliation = self
                    .threads
                    .interrupt_and_reconcile(turn)
                    .await
                    .map_err(|error| {
                        format!(
                            "Validation Codex turn timed out after {} seconds; interrupt/reconciliation failed: {error}",
                            self.config.turn_timeout.as_secs_f64()
                        )
                    })?;
                for notification in &reconciliation.observed_notifications {
                    self.persist_notification(evaluation, turn, notification)?;
                    collector.ingest(
                        &turn.external_thread_id,
                        &turn.external_turn_id,
                        notification,
                    )?;
                }
                Err(format!(
                    "Validation Codex turn timed out after {} seconds and was interrupted; reconciled status: {}.",
                    self.config.turn_timeout.as_secs_f64(),
                    reconciliation_status_label(&reconciliation.reconciliation.status)
                ))
            }
        }
    }

    fn persist_notification(
        &self,
        evaluation: &CadValidationEvaluation,
        turn: &ManagedAgentTurn,
        notification: &RoutedNotification,
    ) -> Result<(), String> {
        assert_notification_identity(turn, notification)?;
        self.service
            .save_validation_evaluation_event(CadValidationEvaluationEvent {
                id: Uuid::new_v4().to_string(),
                session_id: evaluation.session_id.clone(),
                evaluation_id: evaluation.id.clone(),
                evaluator_thread_id: turn.agent_thread_id.clone(),
                external_turn_id: Some(turn.external_turn_id.clone()),
                external_item_id: notification.identifiers.item_id.clone(),
                method: notification.method.clone(),
                sequence: notification.transport_sequence,
                payload: crate::agent_gateway::normalize_transport_payload(&notification.raw),
                created_at: timestamp(),
            })
            .map(|_| ())
    }
}

#[derive(Default)]
struct StrictValidationCollector {
    final_message: Option<String>,
}

impl StrictValidationCollector {
    fn ingest(
        &mut self,
        external_thread_id: &str,
        external_turn_id: &str,
        notification: &RoutedNotification,
    ) -> Result<Option<Value>, String> {
        assert_external_identity(external_thread_id, external_turn_id, notification)?;
        let params = notification.raw.get("params").unwrap_or(&Value::Null);
        match notification.method.as_str() {
            "turn/started"
            | "item/agentMessage/delta"
            | "item/reasoning/delta"
            | "item/reasoning/textDelta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/summaryPartAdded" => Ok(None),
            "item/started" => {
                validate_non_modeling_item(params.get("item"), "item/started")?;
                Ok(None)
            }
            "item/completed" => {
                let item = params
                    .get("item")
                    .ok_or_else(|| "Validation item/completed omitted item.".to_string())?;
                let item_type = required_item_type(item, "item/completed")?;
                match item_type {
                    "reasoning" => Ok(None),
                    "agentMessage" => {
                        if item.get("phase").and_then(Value::as_str) != Some("final_answer") {
                            return Err(
                                "Validation assistant message must have final_answer phase."
                                    .to_string(),
                            );
                        }
                        if self.final_message.is_some() {
                            return Err(
                                "Validation turn produced more than one final assistant message."
                                    .to_string(),
                            );
                        }
                        let text = item.get("text").and_then(Value::as_str).ok_or_else(|| {
                            "Validation final assistant message omitted text.".to_string()
                        })?;
                        self.final_message = Some(text.to_string());
                        Ok(None)
                    }
                    other => Err(format!(
                        "Validation turn emitted forbidden item type {other:?}."
                    )),
                }
            }
            "turn/completed" => {
                let status = params
                    .pointer("/turn/status")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "Validation turn/completed omitted turn.status.".to_string())?;
                match status {
                    "completed" => {
                        let raw = self.final_message.take().ok_or_else(|| {
                            "Validation turn completed without exactly one final assistant message."
                                .to_string()
                        })?;
                        parse_strict_json_object(&raw).map(Some)
                    }
                    "failed" => Err(params
                        .pointer("/turn/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("Validation Codex turn failed.")
                        .to_string()),
                    "interrupted" => Err("Validation Codex turn was interrupted.".to_string()),
                    other => Err(format!(
                        "Validation turn/completed returned unsupported status {other:?}."
                    )),
                }
            }
            "turn/failed" => Err(params
                .pointer("/turn/error/message")
                .or_else(|| params.pointer("/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("Validation Codex turn failed.")
                .to_string()),
            "turn/interrupted" => Err("Validation Codex turn was interrupted.".to_string()),
            method if method.starts_with("item/") => Err(format!(
                "Validation turn emitted unsupported item notification {method:?}."
            )),
            method => Err(format!(
                "Validation turn emitted unsupported routed notification {method:?}."
            )),
        }
    }
}

fn validate_non_modeling_item(item: Option<&Value>, method: &str) -> Result<(), String> {
    let item = item.ok_or_else(|| format!("Validation {method} omitted item."))?;
    match required_item_type(item, method)? {
        "reasoning" => Ok(()),
        "agentMessage" => {
            if item.get("phase").and_then(Value::as_str) == Some("final_answer") {
                Ok(())
            } else {
                Err("Validation assistant message must have final_answer phase.".to_string())
            }
        }
        other => Err(format!(
            "Validation turn emitted forbidden item type {other:?}."
        )),
    }
}

fn required_item_type<'a>(item: &'a Value, method: &str) -> Result<&'a str, String> {
    item.get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Validation {method} omitted item.type."))
}

fn parse_strict_json_object(raw: &str) -> Result<Value, String> {
    if raw.trim() != raw || raw.is_empty() {
        return Err(
            "Validation final assistant message must be an unadorned raw JSON object.".to_string(),
        );
    }
    let parsed: Value = serde_json::from_str(raw).map_err(|error| {
        format!("Validation final assistant message is not strict raw JSON: {error}")
    })?;
    if !parsed.is_object() {
        return Err("Validation final assistant message must be a JSON object.".to_string());
    }
    Ok(parsed)
}

fn parse_strict_validation_history(
    response: &Value,
    expected_turn_id: &str,
) -> Result<Value, String> {
    let turns = response
        .pointer("/thread/turns")
        .and_then(Value::as_array)
        .ok_or_else(|| "Validation thread/read response omitted thread.turns.".to_string())?;
    let turn = turns
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(expected_turn_id))
        .ok_or_else(|| {
            format!("Validation turn {expected_turn_id} was not found during recovery.")
        })?;
    match turn.get("status").and_then(Value::as_str) {
        Some("completed") => {}
        Some("inProgress") => {
            return Err(format!(
                "Validation turn {expected_turn_id} remains in progress after application restart; live event reattachment is unavailable and the outcome is unknown."
            ))
        }
        Some("interrupted") => {
            return Err(format!(
                "Validation turn {expected_turn_id} was interrupted before recovery."
            ))
        }
        Some("failed") => {
            return Err(turn
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("Recovered validation Codex turn failed.")
                .to_string())
        }
        Some(other) => {
            return Err(format!(
                "Recovered validation turn returned unsupported status {other:?}."
            ))
        }
        None => return Err("Recovered validation turn omitted status.".to_string()),
    }
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "Recovered validation turn omitted items.".to_string())?;
    let mut final_message = None;
    for item in items {
        match required_item_type(item, "thread/read history")? {
            "reasoning" => {}
            "agentMessage" => {
                if item.get("phase").and_then(Value::as_str) != Some("final_answer") {
                    return Err(
                        "Recovered validation assistant message was not final_answer.".to_string(),
                    );
                }
                if final_message.is_some() {
                    return Err(
                        "Recovered validation turn contained multiple final assistant messages."
                            .to_string(),
                    );
                }
                final_message = Some(
                    item.get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            "Recovered validation final assistant message omitted text.".to_string()
                        })?
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "Recovered validation turn contained forbidden item type {other:?}."
                ))
            }
        }
    }
    parse_strict_json_object(&final_message.ok_or_else(|| {
        "Recovered validation turn contained no final assistant message.".to_string()
    })?)
}

fn assert_notification_identity(
    turn: &ManagedAgentTurn,
    notification: &RoutedNotification,
) -> Result<(), String> {
    assert_external_identity(
        &turn.external_thread_id,
        &turn.external_turn_id,
        notification,
    )
}

fn assert_external_identity(
    external_thread_id: &str,
    external_turn_id: &str,
    notification: &RoutedNotification,
) -> Result<(), String> {
    if notification.identifiers.thread_id.as_deref() != Some(external_thread_id)
        || notification.identifiers.turn_id.as_deref() != Some(external_turn_id)
    {
        return Err(format!(
            "Validation notification identity mismatch: expected {}/{}, received {:?}/{:?}.",
            external_thread_id,
            external_turn_id,
            notification.identifiers.thread_id,
            notification.identifiers.turn_id
        ));
    }
    Ok(())
}

fn reconciliation_status_label(status: &RecoveredTurnStatus) -> &'static str {
    match status {
        RecoveredTurnStatus::Completed => "completed",
        RecoveredTurnStatus::Failed { .. } => "failed",
        RecoveredTurnStatus::Interrupted => "interrupted",
        RecoveredTurnStatus::InProgress => "in_progress",
        RecoveredTurnStatus::NotFound => "not_found",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        assert_external_identity, parse_strict_json_object, parse_strict_validation_history,
        StrictValidationCollector,
    };
    use crate::agent_thread_manager::{
        AgentThreadManager, AgentThreadManagerConfig, AgentThreadTransport, TransportRequestError,
    };
    use crate::notification_router::{NotificationIdentifiers, RoutedNotification};
    use crate::notification_router::{NotificationRouter, NotificationRouterConfig};
    use crate::protocol::{
        CadArtifactKind, CadDiagnostics, CadSourceLanguage, CadValidationEvaluation,
        CadValidationEvaluationKind, CadValidationEvaluationStatus, CreateCadSessionInput,
        PersistRuntimeArtifactInput, UpdateModelSourceInput,
    };
    use crate::session_service::{timestamp, SessionService};
    use async_trait::async_trait;
    use base64::Engine;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use uuid::Uuid;

    #[test]
    fn strict_json_accepts_one_raw_object() {
        assert_eq!(
            parse_strict_json_object(r#"{"passed":true}"#).unwrap(),
            json!({"passed": true})
        );
    }

    #[test]
    fn strict_json_rejects_fences_prose_arrays_and_whitespace() {
        for raw in [
            "```json\n{}\n```",
            "report: {}",
            "{} trailing",
            "[]",
            " {}",
            "{}\n",
        ] {
            assert!(parse_strict_json_object(raw).is_err(), "accepted {raw:?}");
        }
    }

    #[test]
    fn malformed_json_is_rejected() {
        assert!(parse_strict_json_object("{bad}").is_err());
    }

    #[test]
    fn recovered_history_rejects_tool_calls_and_non_terminal_turns() {
        let tool_history = json!({"thread":{"turns":[{
            "id":"turn-1","status":"completed","items":[
                {"type":"commandExecution","id":"tool-1"},
                {"type":"agentMessage","id":"message-1","phase":"final_answer","text":"{}"}
            ]
        }]}});
        assert!(parse_strict_validation_history(&tool_history, "turn-1").is_err());

        let running = json!({"thread":{"turns":[{
            "id":"turn-1","status":"inProgress","items":[]
        }]}});
        assert!(parse_strict_validation_history(&running, "turn-1").is_err());
    }

    fn notification(method: &str, params: serde_json::Value) -> RoutedNotification {
        RoutedNotification {
            transport_sequence: 1,
            method: method.to_string(),
            identifiers: NotificationIdentifiers {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("item-1".to_string()),
            },
            raw: json!({"method": method, "params": params}),
        }
    }

    #[test]
    fn mismatched_notification_identity_fails_fast() {
        let mut wrong = notification("turn/started", json!({}));
        wrong.identifiers.turn_id = Some("other-turn".to_string());
        assert!(assert_external_identity("thread-1", "turn-1", &wrong)
            .unwrap_err()
            .contains("identity mismatch"));
    }

    #[test]
    fn completed_turn_requires_exactly_one_strict_final_json_message() {
        let mut collector = StrictValidationCollector::default();
        collector
            .ingest(
                "thread-1",
                "turn-1",
                &notification(
                    "item/completed",
                    json!({"item": {
                        "id":"item-1", "type":"agentMessage",
                        "phase":"final_answer", "text":"{\"passed\":true}"
                    }}),
                ),
            )
            .unwrap();
        let report = collector
            .ingest(
                "thread-1",
                "turn-1",
                &notification(
                    "turn/completed",
                    json!({"turn":{"id":"turn-1","status":"completed"}}),
                ),
            )
            .unwrap()
            .unwrap();
        assert_eq!(report, json!({"passed": true}));
    }

    #[test]
    fn failed_turn_and_tool_items_are_rejected() {
        let mut collector = StrictValidationCollector::default();
        assert!(collector
            .ingest(
                "thread-1",
                "turn-1",
                &notification(
                    "item/completed",
                    json!({"item":{"id":"item-1","type":"commandExecution"}}),
                ),
            )
            .is_err());
        assert!(StrictValidationCollector::default()
            .ingest(
                "thread-1",
                "turn-1",
                &notification(
                    "turn/completed",
                    json!({"turn":{"status":"failed","error":{"message":"boom"}}}),
                ),
            )
            .unwrap_err()
            .contains("boom"));
    }

    #[derive(Clone)]
    struct FakeTransport {
        router: NotificationRouter,
        responses: Arc<Mutex<VecDeque<Result<serde_json::Value, TransportRequestError>>>>,
        notifications: Arc<Mutex<VecDeque<serde_json::Value>>>,
    }

    impl FakeTransport {
        fn new(
            responses: Vec<Result<serde_json::Value, TransportRequestError>>,
            notifications: Vec<serde_json::Value>,
        ) -> Self {
            Self {
                router: NotificationRouter::new(NotificationRouterConfig::default()).unwrap(),
                responses: Arc::new(Mutex::new(responses.into())),
                notifications: Arc::new(Mutex::new(notifications.into())),
            }
        }
    }

    #[async_trait]
    impl AgentThreadTransport for FakeTransport {
        async fn ensure_initialized(&self) -> Result<(), String> {
            Ok(())
        }

        async fn current_connection_generation(&self) -> Option<u64> {
            Some(1)
        }

        fn notification_router(&self) -> NotificationRouter {
            self.router.clone()
        }

        async fn request(
            &self,
            _method: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, TransportRequestError> {
            let response = self.responses.lock().unwrap().pop_front().unwrap();
            if response
                .as_ref()
                .ok()
                .and_then(|value| value.get("turn"))
                .is_some()
            {
                for raw in self.notifications.lock().unwrap().drain(..) {
                    self.router.route(raw).unwrap();
                }
            }
            response
        }
    }

    struct EvaluationFixture {
        service: Arc<SessionService>,
        evaluation: CadValidationEvaluation,
        cwd: std::path::PathBuf,
        image_path: std::path::PathBuf,
    }

    fn evaluation_fixture() -> EvaluationFixture {
        let cwd = std::env::temp_dir().join(format!("vlm-evaluator-{}", Uuid::new_v4()));
        fs::create_dir(&cwd).unwrap();
        let image_path = cwd.join("render.png");
        fs::write(&image_path, b"png").unwrap();
        let service = Arc::new(SessionService::new(cwd.clone()));
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let revision_id = service
            .update_model_source(UpdateModelSourceInput {
                session_id: session.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "cube([1,1,1]);".to_string(),
                parent_revision_id: None,
                parameters: None,
            })
            .unwrap()
            .revision_id;
        let run = service
            .create_agent_run(
                &session.session_id,
                "cube".to_string(),
                None,
                Some("codex".to_string()),
                None,
            )
            .unwrap()
            .0;
        service
            .link_agent_run_output_revision(&session.session_id, &run.id, revision_id.clone())
            .unwrap();
        let artifact = service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: session.session_id.clone(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::RenderImage,
                format: "png".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(b"png"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 1,
                    items: Vec::new(),
                },
                metadata: Default::default(),
            })
            .unwrap()
            .artifact;
        let evaluation_id = Uuid::new_v4().to_string();
        let evaluation = service
            .create_validation_evaluation(CadValidationEvaluation {
                id: evaluation_id.clone(),
                session_id: session.session_id.clone(),
                run_id: run.id.clone(),
                revision_id: revision_id.clone(),
                artifact_id: artifact.id.clone(),
                kind: CadValidationEvaluationKind::Vlm,
                attempt: 1,
                status: CadValidationEvaluationStatus::Queued,
                evaluator_thread_id: None,
                external_turn_id: None,
                input_contract: json!({
                    "contractType":"cadastrophe.vlm_evaluation_input.v1",
                    "evaluationId":evaluation_id,
                    "sessionId":session.session_id,
                    "runId":run.id,
                    "revisionId":revision_id,
                    "artifactId":artifact.id,
                    "userRequest":"cube",
                    "passThreshold":0.8,
                    "renderedImage":{"artifactId":artifact.id,"mediaType":"image/png"},
                    "outputContract":"cadastrophe.vlm_judge_report.v1"
                }),
                report: None,
                passed: None,
                score: None,
                pass_threshold: 0.8,
                error: None,
                created_at: timestamp(),
                started_at: None,
                completed_at: None,
            })
            .unwrap();
        EvaluationFixture {
            service,
            evaluation,
            cwd,
            image_path,
        }
    }

    async fn evaluate_with_fake(
        fixture: &EvaluationFixture,
        transport: FakeTransport,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let threads = AgentThreadManager::with_transport(
            Arc::clone(&fixture.service),
            Arc::new(transport),
            AgentThreadManagerConfig {
                interrupt_terminal_timeout: Duration::from_millis(1),
            },
        )
        .unwrap();
        let evaluator = super::CodexVlmEvaluator::with_thread_manager(
            Arc::clone(&fixture.service),
            threads,
            super::CodexVlmEvaluatorConfig {
                turn_timeout: timeout,
            },
        )
        .unwrap();
        evaluator
            .evaluate(super::CodexVlmEvaluationInput {
                evaluation: fixture.evaluation.clone(),
                rendered_image_path: fixture.image_path.clone(),
                cwd: fixture.cwd.clone(),
                app_data_dir: fixture.cwd.clone(),
            })
            .await
    }

    #[tokio::test]
    async fn fake_transport_returns_strict_json_and_persists_validation_events_only() {
        let fixture = evaluation_fixture();
        let transport = FakeTransport::new(
            vec![
                Ok(json!({"thread":{"id":"thread-1"}})),
                Ok(json!({"turn":{"id":"turn-1"}})),
            ],
            vec![
                json!({"method":"item/completed","params":{
                    "threadId":"thread-1","turnId":"turn-1",
                    "item":{"id":"item-1","type":"agentMessage","phase":"final_answer","text":"{\"score\":0.9}"}
                }}),
                json!({"method":"turn/completed","params":{
                    "threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}
                }}),
            ],
        );
        let report = evaluate_with_fake(&fixture, transport, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(report, json!({"score":0.9}));
        assert_eq!(
            fixture
                .service
                .list_validation_evaluation_events(
                    &fixture.evaluation.session_id,
                    &fixture.evaluation.id
                )
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn fake_transport_timeout_interrupts_reconciles_and_returns_error() {
        let fixture = evaluation_fixture();
        let transport = FakeTransport::new(
            vec![
                Ok(json!({"thread":{"id":"thread-1"}})),
                Ok(json!({"turn":{"id":"turn-1"}})),
                Ok(json!({})),
                Ok(json!({"thread":{"turns":[{
                    "id":"turn-1","status":"interrupted","items":[]
                }]}})),
            ],
            Vec::new(),
        );
        let error = evaluate_with_fake(&fixture, transport, Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(error.contains("interrupted"));
    }
}
