use super::codex_vlm_evaluator::{CodexVlmCheckInput, CodexVlmEvaluator};
use crate::cli::{structural, vlm};
use crate::protocol::*;
use crate::session_service::{timestamp, SessionService};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait VlmCheckExecutor: Send + Sync {
    async fn evaluate(&self, input: CodexVlmCheckInput) -> Result<Value, String>;
    async fn recover(&self, check: &CadValidationCheck) -> Result<Value, String>;
}

#[async_trait]
impl VlmCheckExecutor for CodexVlmEvaluator {
    async fn evaluate(&self, input: CodexVlmCheckInput) -> Result<Value, String> {
        self.evaluate_check(input).await
    }
    async fn recover(&self, check: &CadValidationCheck) -> Result<Value, String> {
        self.recover_check(check).await
    }
}

pub type RefinementEnqueue = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync + 'static>;

#[cfg(test)]
#[async_trait]
trait CheckExecutionProbe: Send + Sync {
    async fn entered(&self, kind: CadValidationCheckKind);
}

#[derive(Clone)]
pub struct ValidationCoordinator {
    service: Arc<SessionService>,
    evaluator: Arc<dyn VlmCheckExecutor>,
    refinement_enqueue: RefinementEnqueue,
    active_batches: Arc<Mutex<HashSet<String>>>,
    active_refinements: Arc<Mutex<HashSet<String>>>,
    cwd: PathBuf,
    #[cfg(test)]
    execution_probe: Option<Arc<dyn CheckExecutionProbe>>,
}

impl ValidationCoordinator {
    pub fn new(
        service: Arc<SessionService>,
        evaluator: Arc<dyn VlmCheckExecutor>,
        refinement_enqueue: RefinementEnqueue,
        cwd: PathBuf,
    ) -> Result<Self, String> {
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(format!(
                "Validation coordinator cwd is invalid: {}",
                cwd.display()
            ));
        }
        Ok(Self {
            service,
            evaluator,
            refinement_enqueue,
            active_batches: Arc::new(Mutex::new(HashSet::new())),
            active_refinements: Arc::new(Mutex::new(HashSet::new())),
            cwd,
            #[cfg(test)]
            execution_probe: None,
        })
    }

    #[cfg(test)]
    fn with_execution_probe(mut self, probe: Arc<dyn CheckExecutionProbe>) -> Self {
        self.execution_probe = Some(probe);
        self
    }

    pub fn recover_startup(&self) -> Result<(), String> {
        self.service
            .normalize_validation_batches_after_process_restart()?;
        for session in self
            .service
            .list_sessions_for_input(ListCadSessionsInput {
                include_archived: true,
                query: None,
            })?
            .sessions
        {
            for batch in self.service.list_validation_batches(&session.id)? {
                if batch.settled_at.is_some() {
                    if batch.effects_applied_at.is_none() {
                        self.apply_settled_effects(&batch)?;
                    }
                    continue;
                }
                if let Some(token) = batch.settlement_claimed_at.as_deref() {
                    self.service.release_validation_batch_settlement(
                        &session.id,
                        &batch.id,
                        token,
                    )?;
                }
                let checks = self
                    .service
                    .list_validation_checks(&session.id, &batch.id)?;
                for check in checks.iter().filter(|check| {
                    check.status == CadValidationCheckStatus::Running
                        && check.kind != CadValidationCheckKind::Vlm
                }) {
                    self.service
                        .reset_validation_check_for_recovery(&session.id, &check.id)?;
                }
                if checks.iter().all(check_terminal) {
                    self.try_settle_batch(&session.id, &batch.id)?;
                } else {
                    self.enqueue(batch)?;
                }
            }
        }
        Ok(())
    }

    pub fn enqueue_run(&self, session_id: &str, run_id: &str) -> Result<usize, String> {
        let run = self
            .service
            .get_agent_run(session_id, run_id)?
            .ok_or_else(|| format!("Agent run not found: {run_id}"))?;
        run.output_revision_id
            .ok_or_else(|| format!("Agent run {run_id} has no output revision."))?;
        let batches = self
            .service
            .list_validation_batches(session_id)?
            .into_iter()
            .filter(|batch| batch.run_id == run_id)
            .collect::<Vec<_>>();
        let mut enqueued = 0;
        for batch in batches {
            if batch.settled_at.is_some() {
                if batch.effects_applied_at.is_none() {
                    self.apply_settled_effects(&batch)?;
                }
            } else {
                self.enqueue(batch)?;
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }

    pub fn enqueue(&self, batch: CadValidationBatch) -> Result<(), String> {
        if !matches!(
            batch.status,
            CadValidationBatchStatus::Queued | CadValidationBatchStatus::Running
        ) || batch.settled_at.is_some()
        {
            return Err(format!(
                "Only an unsettled queued/running batch can be enqueued: {}",
                batch.id
            ));
        }
        let mut active = self
            .active_batches
            .lock()
            .map_err(|_| "Validation active-batch lock is poisoned.".to_string())?;
        if !active.insert(batch.id.clone()) {
            return Ok(());
        }
        drop(active);
        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            let id = batch.id.clone();
            let result = coordinator.execute_batch(batch).await;
            if let Ok(mut active) = coordinator.active_batches.lock() {
                active.remove(&id);
            }
            if let Err(error) = result {
                eprintln!("[cadastrophe:validation] batch_id={id} error={error}");
            }
        });
        Ok(())
    }

    async fn execute_batch(&self, batch: CadValidationBatch) -> Result<(), String> {
        let checks = self
            .service
            .list_validation_checks(&batch.session_id, &batch.id)?;
        if checks.len() != 3 {
            return Err(format!(
                "Validation batch {} does not have exactly three checks.",
                batch.id
            ));
        }
        let structural = required_check(&checks, CadValidationCheckKind::Structural)?.clone();
        let dfm = required_check(&checks, CadValidationCheckKind::Dfm)?.clone();
        let vlm = required_check(&checks, CadValidationCheckKind::Vlm)?.clone();
        let (a, b, c) = tokio::join!(
            self.execute_blocking_check(batch.clone(), structural),
            self.execute_blocking_check(batch.clone(), dfm),
            self.execute_vlm_check(batch.clone(), vlm),
        );
        for result in [a, b, c] {
            result?;
        }
        self.try_settle_batch(&batch.session_id, &batch.id)
    }

    async fn execute_blocking_check(
        &self,
        batch: CadValidationBatch,
        check: CadValidationCheck,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(probe) = &self.execution_probe {
            probe.entered(check.kind.clone()).await;
        }
        if check_terminal(&check) {
            return Ok(());
        }
        let check = if check.status == CadValidationCheckStatus::Running {
            self.service
                .reset_validation_check_for_recovery(&check.session_id, &check.id)?
        } else {
            check
        };
        let running = self
            .service
            .start_validation_check(&check.session_id, &check.id)?;
        let service = Arc::clone(&self.service);
        let app_data_dir = self.service.app_data_dir().to_path_buf();
        let result = match tokio::task::spawn_blocking(move || {
            run_blocking_check(service, &app_data_dir, &batch, &running)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(format!("Validation blocking task join failed: {error}")),
        };
        self.persist_check_result(&check, result)
    }

    async fn execute_vlm_check(
        &self,
        batch: CadValidationBatch,
        check: CadValidationCheck,
    ) -> Result<(), String> {
        #[cfg(test)]
        if let Some(probe) = &self.execution_probe {
            probe.entered(check.kind.clone()).await;
        }
        if check_terminal(&check) {
            return Ok(());
        }
        let result: Result<Value, String> = if check.status == CadValidationCheckStatus::Running {
            self.evaluator.recover(&check).await
        } else {
            let prepare = async {
                // Rendering is blocking but the check intentionally remains queued. A restart simply
                // renders again; running is reserved for a bound recoverable Codex turn.
                let service = Arc::clone(&self.service);
                let app_data_dir = self.service.app_data_dir().to_path_buf();
                let render_batch = batch.clone();
                let render_check = check.clone();
                let render_app_data_dir = app_data_dir.clone();
                let image = match tokio::task::spawn_blocking(move || {
                    render_for_check(service, &render_app_data_dir, &render_batch, &render_check)
                })
                .await
                {
                    Ok(Ok(image)) => image,
                    Ok(Err(error)) => {
                        return Err(format!("VLM render failed: {error}"));
                    }
                    Err(error) => {
                        return Err(format!("VLM render task join failed: {error}"));
                    }
                };
                let path = artifact_path(&app_data_dir, &image)?;
                let evaluation_check =
                    vlm_evaluation_snapshot(&batch, &check, &image, &app_data_dir)?;
                self.evaluator
                    .evaluate(CodexVlmCheckInput {
                        check: evaluation_check,
                        rendered_image_path: path,
                        cwd: self.cwd.clone(),
                        app_data_dir,
                    })
                    .await
            };
            prepare.await
        };
        match result {
            Ok(report) => {
                let current = self
                    .service
                    .get_validation_check(&check.session_id, &check.id)?
                    .ok_or_else(|| format!("Validation check not found: {}", check.id))?;
                if current.status != CadValidationCheckStatus::Running {
                    return self
                        .service
                        .fail_validation_check(
                            &check.session_id,
                            &check.id,
                            "VLM returned before its turn was bound.".into(),
                        )
                        .map(|_| ());
                }
                match validate_vlm_check_report(&batch, &current, report) {
                    Ok(validated) => {
                        if !validated.passed {
                            if let Err(error) = ensure_rejection_has_actionable_issue(
                                &current.kind,
                                &validated.report,
                            ) {
                                return self
                                    .service
                                    .fail_validation_check(
                                        &check.session_id,
                                        &check.id,
                                        format!("VLM rejection report is not actionable: {error}"),
                                    )
                                    .map(|_| ());
                            }
                        }
                        self.service
                            .complete_validation_check(
                                &check.session_id,
                                &check.id,
                                validated.report,
                                validated.passed,
                            )
                            .map(|_| ())
                    }
                    Err(error) => self
                        .service
                        .fail_validation_check(
                            &check.session_id,
                            &check.id,
                            format!("VLM report contract validation failed: {error}"),
                        )
                        .map(|_| ()),
                }
            }
            Err(error) => self
                .service
                .fail_validation_check(
                    &check.session_id,
                    &check.id,
                    format!("VLM evaluation execution failed: {error}"),
                )
                .map(|_| ()),
        }
    }

    fn persist_check_result(
        &self,
        check: &CadValidationCheck,
        result: Result<(Value, bool), String>,
    ) -> Result<(), String> {
        match result {
            Ok((report, passed)) => {
                if !passed {
                    if let Err(error) = ensure_rejection_has_actionable_issue(&check.kind, &report)
                    {
                        return self
                            .service
                            .fail_validation_check(
                                &check.session_id,
                                &check.id,
                                format!("Validation rejection report is not actionable: {error}"),
                            )
                            .map(|_| ());
                    }
                }
                self.service
                    .complete_validation_check(&check.session_id, &check.id, report, passed)
                    .map(|_| ())
            }
            Err(error) => self
                .service
                .fail_validation_check(&check.session_id, &check.id, error)
                .map(|_| ()),
        }
    }

    fn try_settle_batch(&self, session_id: &str, batch_id: &str) -> Result<(), String> {
        let Some(claimed) = self
            .service
            .try_claim_validation_batch_settlement(session_id, batch_id)?
        else {
            return Ok(());
        };
        let token = claimed
            .settlement_claimed_at
            .clone()
            .ok_or_else(|| "Claimed validation batch has no claim token.".to_string())?;
        let checks = self.service.list_validation_checks(session_id, batch_id)?;
        let failed = checks
            .iter()
            .any(|check| check.status == CadValidationCheckStatus::Failed);
        let aggregate = if failed {
            None
        } else {
            Some(build_aggregate(&claimed, &checks)?)
        };
        let status = if failed {
            CadValidationBatchStatus::Failed
        } else {
            CadValidationBatchStatus::Succeeded
        };
        let settled = self
            .service
            .settle_validation_batch(session_id, batch_id, &token, status, aggregate)?;
        self.apply_settled_effects(&settled)
    }

    fn apply_settled_effects(&self, batch: &CadValidationBatch) -> Result<(), String> {
        if batch.effects_applied_at.is_some() {
            return Ok(());
        }
        let Some(claimed) = self
            .service
            .try_claim_validation_batch_effects(&batch.session_id, &batch.id)?
        else {
            return Ok(());
        };
        let batch = &claimed;
        let effect_claim = required_effect_claim(batch)?;
        let checks = self
            .service
            .list_validation_checks(&batch.session_id, &batch.id)?;
        if !self.is_latest(batch)? {
            self.service.mark_validation_batch_effects_applied(
                &batch.session_id,
                &batch.id,
                effect_claim,
            )?;
            return Ok(());
        }
        let effect_run = self
            .service
            .get_agent_run(&batch.session_id, &batch.run_id)?
            .ok_or_else(|| format!("Agent run not found: {}", batch.run_id))?;
        if matches!(
            effect_run.status,
            CadAgentRunStatus::Completed | CadAgentRunStatus::Failed | CadAgentRunStatus::Cancelled
        ) {
            self.service.mark_validation_batch_effects_applied(
                &batch.session_id,
                &batch.id,
                effect_claim,
            )?;
            return Ok(());
        }
        if batch.status == CadValidationBatchStatus::Failed {
            let error = checks
                .iter()
                .filter_map(|check| {
                    check
                        .error
                        .as_ref()
                        .map(|error| format!("{:?}: {error}", check.kind))
                })
                .collect::<Vec<_>>()
                .join("; ");
            if error.is_empty() {
                return Err("Failed validation batch has no operational check error.".into());
            }
            self.service.update_agent_run(&batch.session_id, &batch.run_id, Some(CadAgentRunStatus::Failed), Some(None), Some(error.clone()), None, Some(json!({"validationBatchId": batch.id, "operationalFailure": true, "error": error})))?;
            self.service.mark_validation_batch_effects_applied(
                &batch.session_id,
                &batch.id,
                effect_claim,
            )?;
            return Ok(());
        }
        let aggregate = batch
            .aggregate_report
            .clone()
            .ok_or_else(|| "Succeeded validation batch has no aggregate report.".to_string())?;
        let passed = aggregate
            .get("passed")
            .and_then(Value::as_bool)
            .ok_or_else(|| "Aggregate report passed is missing.".to_string())?;
        let structural = required_check(&checks, CadValidationCheckKind::Structural)?
            .report
            .clone()
            .ok_or_else(|| "Structural report missing.".to_string())?;
        let dfm = required_check(&checks, CadValidationCheckKind::Dfm)?
            .report
            .clone()
            .ok_or_else(|| "DFM report missing.".to_string())?;
        let vlm = required_check(&checks, CadValidationCheckKind::Vlm)?
            .report
            .clone()
            .ok_or_else(|| "VLM report missing.".to_string())?;
        if let Some(profile_hash) = dfm.get("profileHash").and_then(Value::as_str) {
            self.service.update_artifact_profile_hash(
                &batch.session_id,
                &batch.artifact_id,
                profile_hash,
            )?;
        }
        let existing = self
            .service
            .get_session_state(&batch.session_id)?
            .workflow
            .outer_iterations
            .into_iter()
            .any(|iteration| iteration.id == format!("validation-batch-{}", batch.id));
        if !existing {
            let iteration = self
                .service
                .get_session_state(&batch.session_id)?
                .workflow
                .outer_iterations
                .iter()
                .filter(|item| item.run_id == batch.run_id)
                .map(|item| item.iteration)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| "Outer iteration overflow.".to_string())?;
            self.service.save_workflow_outer_iteration(
                &batch.session_id,
                CadWorkflowOuterIteration {
                    id: format!("validation-batch-{}", batch.id),
                    run_id: batch.run_id.clone(),
                    iteration,
                    revision_id: Some(batch.revision_id.clone()),
                    structural_report: structural,
                    dfm_report: Some(dfm),
                    vlm_report: Some(vlm),
                    failure_report: aggregate
                        .get("failureReport")
                        .filter(|v| !v.is_null())
                        .cloned(),
                    passed,
                    created_at: timestamp(),
                },
            )?;
        }
        if passed {
            self.service.update_agent_run(
                &batch.session_id,
                &batch.run_id,
                Some(CadAgentRunStatus::Completed),
                Some(None),
                None,
                None,
                Some(json!({"validationBatchId":batch.id,"passed":true})),
            )?;
        } else {
            let current = self
                .service
                .get_agent_run(&batch.session_id, &batch.run_id)?
                .ok_or_else(|| format!("Agent run not found: {}", batch.run_id))?;
            if batch.refinement_requested_at.is_some() && current.external_turn_id.is_some() {
                self.service.bind_validation_batch_refinement(
                    &batch.session_id,
                    &batch.id,
                    effect_claim,
                )?;
                return Ok(());
            }
            if batch.refinement_requested_at.is_none() {
                self.service.prepare_agent_run_validation_batch_refinement(
                    &batch.session_id,
                    &batch.run_id,
                    &batch.id,
                )?;
                self.service.request_validation_batch_refinement(
                    &batch.session_id,
                    &batch.id,
                    effect_claim,
                )?;
            }
            self.enqueue_pending_refinement(batch)?;
            return Ok(());
        }
        self.service.mark_validation_batch_effects_applied(
            &batch.session_id,
            &batch.id,
            effect_claim,
        )?;
        Ok(())
    }

    fn enqueue_pending_refinement(&self, batch: &CadValidationBatch) -> Result<(), String> {
        let mut active = self
            .active_refinements
            .lock()
            .map_err(|_| "Validation active-refinement lock is poisoned.".to_string())?;
        if !active.insert(batch.id.clone()) {
            return Ok(());
        }
        drop(active);
        if let Err(error) = (self.refinement_enqueue)(&batch.session_id, &batch.run_id) {
            self.active_refinements
                .lock()
                .map_err(|_| "Validation active-refinement lock is poisoned.".to_string())?
                .remove(&batch.id);
            return Err(error);
        }
        Ok(())
    }

    pub fn confirm_refinement_bound(
        &self,
        session_id: &str,
        run_id: &str,
    ) -> Result<usize, String> {
        let run = self
            .service
            .get_agent_run(session_id, run_id)?
            .ok_or_else(|| format!("Agent run not found: {run_id}"))?;
        if run.external_turn_id.is_none() {
            return Err(format!(
                "Agent run has no durable refinement turn binding: {run_id}"
            ));
        }
        let candidates = self
            .service
            .list_validation_batches(session_id)?
            .into_iter()
            .filter(|batch| {
                batch.run_id == run_id
                    && batch.refinement_requested_at.is_some()
                    && batch.refinement_bound_at.is_none()
                    && batch.effects_applied_at.is_none()
            })
            .collect::<Vec<_>>();
        let mut bound = 0;
        for batch in candidates {
            if batch.effects_claimed_at.is_some() {
                let effect_claim = required_effect_claim(&batch)?;
                if self.is_latest(&batch)? {
                    self.service.bind_validation_batch_refinement(
                        session_id,
                        &batch.id,
                        effect_claim,
                    )?;
                } else {
                    self.service.mark_validation_batch_effects_applied(
                        session_id,
                        &batch.id,
                        effect_claim,
                    )?;
                }
            } else {
                self.apply_settled_effects(&batch)?;
            }
            let current = self
                .service
                .get_validation_batch(session_id, &batch.id)?
                .ok_or_else(|| format!("Validation batch not found: {}", batch.id))?;
            if current.refinement_bound_at.is_some() && current.effects_applied_at.is_some() {
                self.active_refinements
                    .lock()
                    .map_err(|_| "Validation active-refinement lock is poisoned.".to_string())?
                    .remove(&batch.id);
                bound += 1;
            }
        }
        Ok(bound)
    }

    fn is_latest(&self, batch: &CadValidationBatch) -> Result<bool, String> {
        let run = self
            .service
            .get_agent_run(&batch.session_id, &batch.run_id)?
            .ok_or_else(|| format!("Agent run not found: {}", batch.run_id))?;
        if run.output_revision_id.as_deref() != Some(batch.revision_id.as_str()) {
            return Ok(false);
        }
        self.service
            .is_latest_validation_batch(&batch.session_id, &batch.id)
    }
}

fn check_terminal(check: &CadValidationCheck) -> bool {
    matches!(
        check.status,
        CadValidationCheckStatus::Succeeded | CadValidationCheckStatus::Failed
    )
}
fn required_effect_claim(batch: &CadValidationBatch) -> Result<&str, String> {
    batch
        .effects_claimed_at
        .as_deref()
        .ok_or_else(|| format!("Validation batch effects are not owned: {}", batch.id))
}
fn required_check(
    checks: &[CadValidationCheck],
    kind: CadValidationCheckKind,
) -> Result<&CadValidationCheck, String> {
    checks
        .iter()
        .find(|check| check.kind == kind)
        .ok_or_else(|| format!("Validation batch is missing {kind:?} check."))
}

fn run_blocking_check(
    service: Arc<SessionService>,
    app: &Path,
    batch: &CadValidationBatch,
    check: &CadValidationCheck,
) -> Result<(Value, bool), String> {
    verify_stl_contract(&check.input_contract)?;
    let artifact = batch_artifact(&service, batch)?;
    match check.kind {
        CadValidationCheckKind::Structural => {
            verify_executable_contract(&check.input_contract, "sidecarPath", "sidecarSha256")?;
            let plan: CadModelPlan =
                serde_json::from_value(required(&check.input_contract, "plan")?.clone())
                    .map_err(|e| format!("Structural plan contract is invalid: {e}"))?;
            let path = required_string(&check.input_contract, "sidecarPath")?;
            let result = structural::evaluate_structural_for_revision(
                &service,
                &app.to_path_buf(),
                &batch.session_id,
                Some(&batch.run_id),
                &batch.revision_id,
                &plan,
                Some(&artifact.id),
                Some(path),
            )
            .map_err(|e| e.message)?;
            structural::validate_structural_report(
                &result.report,
                &batch.run_id,
                &batch.revision_id,
            )
            .map_err(|e| e.message)?;
            Ok((result.report, result.passed))
        }
        CadValidationCheckKind::Dfm => {
            let prepared: crate::dfm::PreparedDfmInputs =
                serde_json::from_value(required(&check.input_contract, "prepared")?.clone())
                    .map_err(|e| format!("DFM prepared contract is invalid: {e}"))?;
            let result = crate::dfm::evaluate_prepared(
                &service,
                app,
                &batch.session_id,
                &batch.run_id,
                &batch.revision_id,
                &artifact,
                &prepared,
            )?;
            crate::dfm::validate_report(&result.report, &batch.run_id, &batch.revision_id)?;
            Ok((result.report, result.passed))
        }
        CadValidationCheckKind::Vlm => Err("VLM check cannot run in blocking validator.".into()),
    }
}

fn render_for_check(
    service: Arc<SessionService>,
    app: &Path,
    batch: &CadValidationBatch,
    check: &CadValidationCheck,
) -> Result<CadArtifact, String> {
    verify_stl_contract(&check.input_contract)?;
    verify_executable_contract(
        &check.input_contract,
        "rendererSidecarPath",
        "rendererSidecarSha256",
    )?;
    let plan: CadModelPlan =
        serde_json::from_value(required(&check.input_contract, "plan")?.clone())
            .map_err(|e| format!("VLM plan contract is invalid: {e}"))?;
    let artifact = batch_artifact(&service, batch)?;
    let renderer = required_string(&check.input_contract, "rendererSidecarPath")?;
    vlm::render_vlm_images_for_artifact(
        &service,
        &app.to_path_buf(),
        &batch.session_id,
        &format!("{}-{}", batch.id, check.id),
        &batch.revision_id,
        &plan,
        &artifact,
        Some(renderer),
    )
    .map_err(|e| e.message)
}

fn vlm_evaluation_snapshot(
    batch: &CadValidationBatch,
    check: &CadValidationCheck,
    image: &CadArtifact,
    app: &Path,
) -> Result<CadValidationCheck, String> {
    let artifact = check
        .input_contract
        .pointer("/stl/artifact")
        .ok_or_else(|| "VLM input missing STL artifact.".to_string())?;
    let final_artifact: CadArtifact =
        serde_json::from_value(artifact.clone()).map_err(|e| e.to_string())?;
    let judge = vlm::build_vlm_contract(image).map_err(|e| e.message)?;
    let contract =
        super::contract::build_input_contract(super::contract::EvaluationContractInput {
            session_id: &batch.session_id,
            run_id: &batch.run_id,
            revision_id: &batch.revision_id,
            user_request: required_string(&check.input_contract, "userRequest")?,
            final_artifact: &final_artifact,
            rendered_image: image,
            pass_threshold: required(&check.input_contract, "passThreshold")?
                .as_f64()
                .ok_or_else(|| "VLM passThreshold invalid.".to_string())?,
            judge_contract: &judge,
            app_data_dir: app,
        })?;
    let mut object = contract
        .as_object()
        .cloned()
        .ok_or_else(|| "VLM contract is not object.".to_string())?;
    object.insert("evaluationId".into(), Value::String(check.id.clone()));
    object.insert("batchId".into(), Value::String(batch.id.clone()));
    object.insert("attempt".into(), json!(batch.attempt));
    let mut snapshot = check.clone();
    snapshot.input_contract = Value::Object(object);
    Ok(snapshot)
}

fn validate_vlm_check_report(
    batch: &CadValidationBatch,
    check: &CadValidationCheck,
    report: Value,
) -> Result<super::contract::ValidatedReport, String> {
    let evaluation = CadValidationEvaluation {
        id: check.id.clone(),
        session_id: batch.session_id.clone(),
        run_id: batch.run_id.clone(),
        revision_id: batch.revision_id.clone(),
        artifact_id: batch.artifact_id.clone(),
        kind: CadValidationEvaluationKind::Vlm,
        attempt: batch.attempt,
        status: CadValidationEvaluationStatus::Running,
        evaluator_thread_id: check.evaluator_thread_id.clone(),
        external_turn_id: check.external_turn_id.clone(),
        input_contract: check.input_contract.clone(),
        report: None,
        passed: None,
        score: None,
        pass_threshold: check
            .input_contract
            .get("passThreshold")
            .and_then(Value::as_f64)
            .ok_or_else(|| "VLM check input is missing passThreshold.".to_string())?,
        error: None,
        created_at: check.created_at.clone(),
        started_at: check.started_at.clone(),
        completed_at: None,
    };
    super::contract::validate_report(&evaluation, report)
}

fn build_aggregate(
    batch: &CadValidationBatch,
    checks: &[CadValidationCheck],
) -> Result<Value, String> {
    let mut map = Map::new();
    let mut passed = true;
    let mut issues = Vec::new();
    for kind in [
        CadValidationCheckKind::Structural,
        CadValidationCheckKind::Dfm,
        CadValidationCheckKind::Vlm,
    ] {
        let check = required_check(checks, kind.clone())?;
        let report = check
            .report
            .clone()
            .ok_or_else(|| format!("Succeeded {kind:?} check has no report."))?;
        let ok = check
            .passed
            .ok_or_else(|| format!("Succeeded {kind:?} check has no passed."))?;
        passed &= ok;
        if !ok {
            collect_issues(&format!("{:?}", kind).to_lowercase(), &report, &mut issues)?;
        }
        map.insert(
            format!("{:?}", kind).to_lowercase(),
            json!({"passed":ok,"report":report}),
        );
    }
    let failure = if passed {
        Value::Null
    } else {
        json!({"contractType":"cadastrophe.failure_report.v1","reason":"validation_batch_rejected","summary":"One or more validation checks rejected the final artifact.","nextAction":"outer_loop_refine_source","issues":issues})
    };
    Ok(
        json!({"contractType":"cadastrophe.finalization_report.v2","batchId":batch.id,"revisionId":batch.revision_id,"artifactId":batch.artifact_id,"attempt":batch.attempt,"passed":passed,"checks":map,"failureReport":failure}),
    )
}
fn ensure_rejection_has_actionable_issue(
    kind: &CadValidationCheckKind,
    report: &Value,
) -> Result<(), String> {
    let mut issues = Vec::new();
    collect_issues(&format!("{kind:?}").to_lowercase(), report, &mut issues).map(|_| ())
}

fn collect_issues(source: &str, report: &Value, out: &mut Vec<Value>) -> Result<usize, String> {
    let initial_len = out.len();
    for key in [
        "issues",
        "findings",
        "diagnostics",
        "checks",
        "inconsistencies",
    ] {
        if let Some(items) = report.get(key).and_then(Value::as_array) {
            for item in items {
                let rejected = item.get("passed").and_then(Value::as_bool) == Some(false)
                    || item
                        .get("severity")
                        .and_then(Value::as_str)
                        .is_some_and(|s| matches!(s, "error" | "major"))
                    || key == "findings"
                    || key == "inconsistencies";
                if rejected {
                    if let Some(obj) = item.as_object() {
                        if let Some(issue) = actionable_issue_from_object(source, obj) {
                            out.push(issue);
                        }
                    } else if let Some(message) =
                        item.as_str().filter(|value| !value.trim().is_empty())
                    {
                        out.push(json!({"source":source,"message":message}));
                    }
                }
            }
        }
    }
    if out.len() == initial_len {
        let failure = report.get("failureReport").and_then(Value::as_object);
        let code = failure
            .and_then(|value| value.get("reason"))
            .and_then(Value::as_str)
            .or_else(|| report.get("code").and_then(Value::as_str));
        let message = failure
            .and_then(|value| value.get("summary"))
            .and_then(Value::as_str)
            .or_else(|| report.get("diagnostic").and_then(Value::as_str));
        match (
            code.filter(|value| !value.trim().is_empty()),
            message.filter(|value| !value.trim().is_empty()),
        ) {
            (Some(code), Some(message)) => {
                out.push(json!({"source":source,"code":code,"message":message}));
            }
            _ => {
                return Err(format!(
                    "Rejected {source} report contains no actionable issue with an actual message and no complete failure reason/summary."
                ));
            }
        }
    }
    Ok(out.len() - initial_len)
}

fn actionable_issue_from_object(source: &str, object: &Map<String, Value>) -> Option<Value> {
    let message = ["message", "summary", "diagnostic", "observed"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())?;
    let code = ["code", "name", "reason", "severity"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty());
    let mut issue = object.clone();
    issue.insert("source".into(), json!(source));
    issue.insert("message".into(), json!(message));
    if let Some(code) = code {
        issue.insert("code".into(), json!(code));
    }
    Some(Value::Object(issue))
}
fn batch_artifact(
    service: &SessionService,
    batch: &CadValidationBatch,
) -> Result<CadArtifact, String> {
    service
        .get_revision(&batch.session_id, &batch.revision_id)?
        .artifacts
        .into_iter()
        .find(|a| a.id == batch.artifact_id)
        .ok_or_else(|| format!("Validation STL artifact not found: {}", batch.artifact_id))
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    v.get(key)
        .ok_or_else(|| format!("Validation input missing {key}."))
}
fn required_string<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    required(v, key)?
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Validation input {key} must be non-empty string."))
}
fn artifact_path(app: &Path, artifact: &CadArtifact) -> Result<PathBuf, String> {
    crate::cli::artifacts::artifact_filesystem_path(app, artifact)
        .map(PathBuf::from)
        .ok_or_else(|| "Rendered artifact path missing.".into())
}
fn verify_stl_contract(c: &Value) -> Result<(), String> {
    let path = PathBuf::from(
        c.pointer("/stl/path")
            .and_then(Value::as_str)
            .ok_or_else(|| "STL contract path missing.".to_string())?,
    );
    let expected = c
        .pointer("/stl/sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| "STL contract sha256 missing.".to_string())?;
    let bytes = std::fs::read(&path)
        .map_err(|e| format!("Immutable STL unavailable {}: {e}", path.display()))?;
    let actual = crate::storage::sha256_hex(&bytes);
    if actual != expected {
        return Err(format!(
            "Immutable STL hash changed: expected {expected}, received {actual}."
        ));
    }
    Ok(())
}
fn verify_executable_contract(c: &Value, path_key: &str, hash_key: &str) -> Result<(), String> {
    let path = required_string(c, path_key)?;
    let expected = required_string(c, hash_key)?;
    let actual = crate::storage::sha256_hex(
        &std::fs::read(path)
            .map_err(|e| format!("Validator executable unavailable {path}: {e}"))?,
    );
    if actual != expected {
        return Err(format!(
            "Validator executable changed after queueing: {path}."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_repository::SqliteSessionRepository;
    use crate::storage::{self, StorageLayout};
    use base64::Engine;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NeverVlm;
    #[async_trait]
    impl VlmCheckExecutor for NeverVlm {
        async fn evaluate(&self, _: CodexVlmCheckInput) -> Result<Value, String> {
            Err("unexpected VLM execution".into())
        }
        async fn recover(&self, _: &CadValidationCheck) -> Result<Value, String> {
            Err("unexpected VLM recovery".into())
        }
    }

    struct RecoveringVlm {
        recover_count: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl VlmCheckExecutor for RecoveringVlm {
        async fn evaluate(&self, _: CodexVlmCheckInput) -> Result<Value, String> {
            Err("queued VLM evaluation was not expected".into())
        }
        async fn recover(&self, _: &CadValidationCheck) -> Result<Value, String> {
            self.recover_count.fetch_add(1, Ordering::SeqCst);
            Err("simulated recovered VLM operational failure".into())
        }
    }

    struct OverlapProbe {
        barrier: tokio::sync::Barrier,
        active: AtomicUsize,
        peak: AtomicUsize,
    }
    #[async_trait]
    impl CheckExecutionProbe for OverlapProbe {
        async fn entered(&self, _: CadValidationCheckKind) {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            self.barrier.wait().await;
            self.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    struct Fixture {
        service: Arc<SessionService>,
        session: String,
        run: String,
        revision: String,
        artifact: String,
        cwd: PathBuf,
    }
    fn fixture() -> Fixture {
        let cwd = std::env::temp_dir().join(format!("batch-coordinator-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cwd).unwrap();
        let service = Arc::new(SessionService::new(cwd.clone()));
        populate_fixture(service, cwd)
    }
    fn persistent_fixture() -> (Fixture, StorageLayout) {
        let app_data_dir =
            std::env::temp_dir().join(format!("batch-coordinator-db-{}", uuid::Uuid::new_v4()));
        let layout = StorageLayout::from_app_data_dir(app_data_dir.clone());
        storage::initialize_storage(&layout).unwrap();
        let service = Arc::new(
            SessionService::with_repository(
                layout.clone(),
                Arc::new(SqliteSessionRepository::new(layout.clone())),
            )
            .unwrap(),
        );
        (populate_fixture(service, app_data_dir), layout)
    }
    fn populate_fixture(service: Arc<SessionService>, cwd: PathBuf) -> Fixture {
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let (run, _) = service
            .create_agent_run(
                &session.session_id,
                "make bracket".into(),
                None,
                Some("test".into()),
                None,
            )
            .unwrap();
        let revision = service
            .update_model_source(UpdateModelSourceInput {
                session_id: session.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "cube([1,1,1]);".into(),
                parent_revision_id: None,
                parameters: None,
            })
            .unwrap()
            .revision_id;
        service
            .link_agent_run_output_revision(&session.session_id, &run.id, revision.clone())
            .unwrap();
        let artifact = service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: session.session_id.clone(),
                revision_id: revision.clone(),
                kind: CadArtifactKind::Stl,
                format: "stl".into(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"solid x\nendsolid x\n"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 0,
                    items: vec![],
                },
                metadata: Map::new(),
            })
            .unwrap()
            .artifact
            .id;
        Fixture {
            service,
            session: session.session_id,
            run: run.id,
            revision,
            artifact,
            cwd,
        }
    }
    fn create_batch(f: &Fixture) -> (CadValidationBatch, Vec<CadValidationCheck>) {
        f.service
            .create_validation_batch(CadValidationBatchCreate {
                session_id: f.session.clone(),
                run_id: f.run.clone(),
                revision_id: f.revision.clone(),
                artifact_id: f.artifact.clone(),
                checks: vec![
                    CadValidationCheckCreate {
                        kind: CadValidationCheckKind::Structural,
                        input_contract: json!({}),
                    },
                    CadValidationCheckCreate {
                        kind: CadValidationCheckKind::Dfm,
                        input_contract: json!({}),
                    },
                    CadValidationCheckCreate {
                        kind: CadValidationCheckKind::Vlm,
                        input_contract: json!({"passThreshold":0.8}),
                    },
                ],
            })
            .unwrap()
    }
    fn start_all(f: &Fixture, checks: &[CadValidationCheck]) {
        for check in checks {
            match check.kind {
                CadValidationCheckKind::Structural | CadValidationCheckKind::Dfm => {
                    f.service
                        .start_validation_check(&f.session, &check.id)
                        .unwrap();
                }
                CadValidationCheckKind::Vlm => {
                    let now = timestamp();
                    let thread = CadAgentThread {
                        id: format!("thread-{}", check.id),
                        session_id: f.session.clone(),
                        plane: CadAgentPlane::Validation,
                        owner_id: check.id.clone(),
                        external_agent: "fake".into(),
                        external_thread_id: format!("external-{}", check.id),
                        status: CadAgentThreadStatus::Active,
                        connection_generation: Some(1),
                        created_at: now.clone(),
                        updated_at: now,
                        last_resumed_at: None,
                        archived_at: None,
                        replaced_by_id: None,
                        metadata: None,
                    };
                    f.service.upsert_agent_thread(thread.clone()).unwrap();
                    f.service
                        .bind_validation_check(&f.session, &check.id, &thread.id, "turn-1")
                        .unwrap();
                }
            }
        }
    }
    fn complete_rejecting_checks(f: &Fixture, checks: &[CadValidationCheck]) {
        start_all(f, checks);
        for check in checks {
            let passed = check.kind == CadValidationCheckKind::Vlm;
            let report = match check.kind {
                CadValidationCheckKind::Structural => {
                    json!({"passed":false,"checks":[{"passed":false,"message":"thin"}]})
                }
                CadValidationCheckKind::Dfm => {
                    json!({"passed":false,"profileHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","diagnostics":[{"severity":"error","message":"bridge"}]})
                }
                CadValidationCheckKind::Vlm => json!({"passed":true,"findings":[]}),
            };
            f.service
                .complete_validation_check(&f.session, &check.id, report, passed)
                .unwrap();
        }
    }
    fn coordinator(f: &Fixture, count: Arc<AtomicUsize>) -> ValidationCoordinator {
        ValidationCoordinator::new(
            f.service.clone(),
            Arc::new(NeverVlm),
            Arc::new(move |_, _| {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            f.cwd.clone(),
        )
        .unwrap()
    }

    fn batch() -> CadValidationBatch {
        CadValidationBatch {
            id: "batch-1".into(),
            session_id: "session-1".into(),
            run_id: "run-1".into(),
            revision_id: "revision-1".into(),
            artifact_id: "artifact-1".into(),
            attempt: 1,
            status: CadValidationBatchStatus::Running,
            aggregate_report: None,
            created_at: "1".into(),
            started_at: Some("1".into()),
            settlement_claimed_at: None,
            settled_at: None,
            effects_claimed_at: None,
            refinement_requested_at: None,
            refinement_bound_at: None,
            effects_applied_at: None,
        }
    }
    fn check(kind: CadValidationCheckKind, passed: bool, report: Value) -> CadValidationCheck {
        CadValidationCheck {
            id: format!("{kind:?}"),
            batch_id: "batch-1".into(),
            session_id: "session-1".into(),
            kind,
            status: CadValidationCheckStatus::Succeeded,
            input_contract: json!({}),
            report: Some(report),
            passed: Some(passed),
            error: None,
            evaluator_thread_id: None,
            external_turn_id: None,
            created_at: "1".into(),
            started_at: Some("1".into()),
            completed_at: Some("2".into()),
        }
    }

    #[test]
    fn multiple_rejections_are_aggregated_with_source_tags() {
        let checks = vec![
            check(
                CadValidationCheckKind::Structural,
                false,
                json!({"checks":[{"passed":false,"code":"wall","message":"thin"}]}),
            ),
            check(
                CadValidationCheckKind::Dfm,
                false,
                json!({"diagnostics":[{"severity":"error","code":"bridge","message":"unsupported"}]}),
            ),
            check(CadValidationCheckKind::Vlm, true, json!({"findings":[]})),
        ];
        let aggregate = build_aggregate(&batch(), &checks).unwrap();
        assert_eq!(aggregate["passed"], false);
        let issues = aggregate["failureReport"]["issues"].as_array().unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0]["source"], "structural");
        assert_eq!(issues[1]["source"], "dfm");
        assert_eq!(
            aggregate["failureReport"]["reason"],
            "validation_batch_rejected"
        );
    }

    #[test]
    fn operational_failure_cannot_be_aggregated_as_rejection() {
        let checks = vec![
            check(CadValidationCheckKind::Structural, true, json!({})),
            CadValidationCheck {
                id: "dfm".into(),
                batch_id: "batch-1".into(),
                session_id: "session-1".into(),
                kind: CadValidationCheckKind::Dfm,
                status: CadValidationCheckStatus::Failed,
                input_contract: json!({}),
                report: None,
                passed: None,
                error: Some("slicer unavailable".into()),
                evaluator_thread_id: None,
                external_turn_id: None,
                created_at: "1".into(),
                started_at: Some("1".into()),
                completed_at: Some("2".into()),
            },
            check(CadValidationCheckKind::Vlm, true, json!({})),
        ];
        assert!(build_aggregate(&batch(), &checks).is_err());
        assert!(checks[1].report.is_none());
        assert!(checks[1].passed.is_none());
    }

    #[test]
    fn rejection_without_actual_actionable_issue_is_an_operational_contract_failure() {
        let checks = vec![
            check(
                CadValidationCheckKind::Structural,
                false,
                json!({"passed":false,"checks":[]}),
            ),
            check(CadValidationCheckKind::Dfm, true, json!({})),
            check(CadValidationCheckKind::Vlm, true, json!({})),
        ];
        let error = build_aggregate(&batch(), &checks).unwrap_err();
        assert!(error.contains("no actionable issue"));

        let f = fixture();
        let (_, persisted) = create_batch(&f);
        let structural = required_check(&persisted, CadValidationCheckKind::Structural).unwrap();
        f.service
            .start_validation_check(&f.session, &structural.id)
            .unwrap();
        coordinator(&f, Arc::new(AtomicUsize::new(0)))
            .persist_check_result(structural, Ok((json!({"passed":false,"checks":[]}), false)))
            .unwrap();
        let failed = f
            .service
            .get_validation_check(&f.session, &structural.id)
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, CadValidationCheckStatus::Failed);
        assert!(failed.report.is_none());
        assert!(failed.error.unwrap().contains("not actionable"));
    }

    #[tokio::test]
    async fn execute_batch_enters_all_three_check_paths_concurrently() {
        let f = fixture();
        let (batch, _) = create_batch(&f);
        let probe = Arc::new(OverlapProbe {
            barrier: tokio::sync::Barrier::new(3),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        });
        let coordinator =
            coordinator(&f, Arc::new(AtomicUsize::new(0))).with_execution_probe(probe.clone());
        coordinator.execute_batch(batch.clone()).await.unwrap();
        assert_eq!(probe.peak.load(Ordering::SeqCst), 3);
        let settled = f
            .service
            .get_validation_batch(&f.session, &batch.id)
            .unwrap()
            .unwrap();
        assert_eq!(settled.status, CadValidationBatchStatus::Failed);
        assert!(settled.settled_at.is_some());
    }

    #[tokio::test]
    async fn startup_reexecutes_running_blocking_checks_and_recovers_running_vlm() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        start_all(&f, &checks);
        let recover_count = Arc::new(AtomicUsize::new(0));
        let coordinator = ValidationCoordinator::new(
            f.service.clone(),
            Arc::new(RecoveringVlm {
                recover_count: recover_count.clone(),
            }),
            Arc::new(|_, _| Ok(())),
            f.cwd.clone(),
        )
        .unwrap();
        coordinator.recover_startup().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if f.service
                    .get_validation_batch(&f.session, &batch.id)
                    .unwrap()
                    .unwrap()
                    .settled_at
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(recover_count.load(Ordering::SeqCst), 1);
        let recovered = f
            .service
            .list_validation_checks(&f.session, &batch.id)
            .unwrap();
        for kind in [
            CadValidationCheckKind::Structural,
            CadValidationCheckKind::Dfm,
        ] {
            let check = required_check(&recovered, kind).unwrap();
            assert_eq!(check.status, CadValidationCheckStatus::Failed);
            assert!(check.error.as_deref().unwrap().contains("contract"));
        }
        assert_eq!(
            required_check(&recovered, CadValidationCheckKind::Vlm)
                .unwrap()
                .status,
            CadValidationCheckStatus::Failed
        );
    }

    #[test]
    fn rejecting_batch_persists_one_outer_iteration_and_enqueues_once() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        complete_rejecting_checks(&f, &checks);
        let count = Arc::new(AtomicUsize::new(0));
        let coordinator = coordinator(&f, count.clone());
        coordinator.try_settle_batch(&f.session, &batch.id).unwrap();
        coordinator.try_settle_batch(&f.session, &batch.id).unwrap();
        let state = f.service.get_session_state(&f.session).unwrap();
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            state.workflow.outer_iterations[0]
                .failure_report
                .as_ref()
                .unwrap()["reason"],
            "validation_batch_rejected"
        );
    }

    #[test]
    fn requested_refinement_is_reenqueued_after_restart_before_turn_binding() {
        let (f, layout) = persistent_fixture();
        let (batch, checks) = create_batch(&f);
        complete_rejecting_checks(&f, &checks);
        let initial_count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, initial_count.clone())
            .try_settle_batch(&f.session, &batch.id)
            .unwrap();
        assert_eq!(initial_count.load(Ordering::SeqCst), 1);
        let requested = f
            .service
            .get_validation_batch(&f.session, &batch.id)
            .unwrap()
            .unwrap();
        assert!(requested.refinement_requested_at.is_some());
        assert!(requested.refinement_bound_at.is_none());
        assert!(requested.effects_applied_at.is_none());

        let session_id = f.session.clone();
        let run_id = f.run.clone();
        let batch_id = batch.id.clone();
        let cwd = f.cwd.clone();
        drop(f);
        let restarted_service = Arc::new(
            SessionService::with_repository(
                layout.clone(),
                Arc::new(SqliteSessionRepository::new(layout)),
            )
            .unwrap(),
        );
        let restart_count = Arc::new(AtomicUsize::new(0));
        let restarted = ValidationCoordinator::new(
            restarted_service.clone(),
            Arc::new(NeverVlm),
            {
                let restart_count = restart_count.clone();
                Arc::new(move |_, _| {
                    restart_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
            cwd,
        )
        .unwrap();
        restarted.recover_startup().unwrap();
        assert_eq!(restart_count.load(Ordering::SeqCst), 1);
        let recovered = restarted_service
            .get_validation_batch(&session_id, &batch_id)
            .unwrap()
            .unwrap();
        assert!(recovered.refinement_requested_at.is_some());
        assert!(recovered.refinement_bound_at.is_none());
        assert!(recovered.effects_applied_at.is_none());
        assert_eq!(
            restarted_service
                .get_agent_run(&session_id, &run_id)
                .unwrap()
                .unwrap()
                .active_step
                .as_deref(),
            Some("Refining after validation batch")
        );
    }

    #[test]
    fn persisted_turn_binding_is_confirmed_after_restart_without_duplicate_enqueue() {
        let (f, layout) = persistent_fixture();
        let (batch, checks) = create_batch(&f);
        complete_rejecting_checks(&f, &checks);
        let initial_count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, initial_count.clone())
            .try_settle_batch(&f.session, &batch.id)
            .unwrap();
        f.service
            .update_agent_run_external_metadata(
                &f.session,
                &f.run,
                Some("codex".into()),
                Some("thread-refine".into()),
                Some("turn-refine".into()),
            )
            .unwrap();

        let session_id = f.session.clone();
        let run_id = f.run.clone();
        let batch_id = batch.id.clone();
        let cwd = f.cwd.clone();
        drop(f);
        let restarted_service = Arc::new(
            SessionService::with_repository(
                layout.clone(),
                Arc::new(SqliteSessionRepository::new(layout)),
            )
            .unwrap(),
        );
        let candidates = restarted_service
            .list_startup_agent_run_recovery_candidates()
            .unwrap();
        assert!(candidates.iter().any(|candidate| {
            candidate.run_id == run_id
                && candidate.action == CadAgentRunRecoveryAction::QueryHistory
        }));
        let restart_count = Arc::new(AtomicUsize::new(0));
        let restarted = ValidationCoordinator::new(
            restarted_service.clone(),
            Arc::new(NeverVlm),
            {
                let restart_count = restart_count.clone();
                Arc::new(move |_, _| {
                    restart_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
            cwd,
        )
        .unwrap();
        restarted.recover_startup().unwrap();
        assert_eq!(restart_count.load(Ordering::SeqCst), 0);
        let recovered = restarted_service
            .get_validation_batch(&session_id, &batch_id)
            .unwrap()
            .unwrap();
        assert!(recovered.refinement_requested_at.is_some());
        assert!(recovered.refinement_bound_at.is_some());
        assert!(recovered.effects_applied_at.is_some());
    }

    #[test]
    fn operational_failure_fails_run_without_iteration_or_refinement() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        start_all(&f, &checks);
        for check in &checks {
            if check.kind == CadValidationCheckKind::Dfm {
                f.service
                    .fail_validation_check(&f.session, &check.id, "slicer crashed".into())
                    .unwrap();
            } else {
                f.service
                    .complete_validation_check(&f.session, &check.id, json!({"passed":true}), true)
                    .unwrap();
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, count.clone())
            .try_settle_batch(&f.session, &batch.id)
            .unwrap();
        let state = f.service.get_session_state(&f.session).unwrap();
        assert!(state.workflow.outer_iterations.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(
            state
                .agent_runs
                .iter()
                .find(|run| run.id == f.run)
                .unwrap()
                .status,
            CadAgentRunStatus::Failed
        );
        assert!(f
            .service
            .get_validation_batch(&f.session, &batch.id)
            .unwrap()
            .unwrap()
            .aggregate_report
            .is_none());
    }

    #[test]
    fn older_same_revision_batch_settles_without_run_side_effects() {
        let f = fixture();
        let (old, checks) = create_batch(&f);
        let (_new, _) = create_batch(&f);
        start_all(&f, &checks);
        for check in &checks {
            f.service
                .complete_validation_check(
                    &f.session,
                    &check.id,
                    json!({"passed":true,"profileHash":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}),
                    true,
                )
                .unwrap();
        }
        let count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, count.clone())
            .try_settle_batch(&f.session, &old.id)
            .unwrap();
        let state = f.service.get_session_state(&f.session).unwrap();
        assert!(state.workflow.outer_iterations.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(f
            .service
            .get_validation_batch(&f.session, &old.id)
            .unwrap()
            .unwrap()
            .settled_at
            .is_some());
    }

    #[test]
    fn terminal_unsettled_batch_is_settled_during_startup_recovery() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        start_all(&f, &checks);
        for check in &checks {
            if check.kind == CadValidationCheckKind::Dfm {
                f.service
                    .fail_validation_check(&f.session, &check.id, "slicer exited".into())
                    .unwrap();
            } else {
                f.service
                    .complete_validation_check(&f.session, &check.id, json!({"passed":true}), true)
                    .unwrap();
            }
        }
        let count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, count.clone()).recover_startup().unwrap();
        let settled = f
            .service
            .get_validation_batch(&f.session, &batch.id)
            .unwrap()
            .unwrap();
        assert_eq!(settled.status, CadValidationBatchStatus::Failed);
        assert!(settled.settled_at.is_some());
        assert!(settled.effects_applied_at.is_some());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stale_revision_batch_settles_without_mutating_run_or_workflow() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        start_all(&f, &checks);
        for check in &checks {
            f.service
                .complete_validation_check(&f.session, &check.id, json!({"passed":true}), true)
                .unwrap();
        }
        let next = f
            .service
            .update_model_source(UpdateModelSourceInput {
                session_id: f.session.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "sphere(2);".into(),
                parent_revision_id: Some(f.revision.clone()),
                parameters: None,
            })
            .unwrap()
            .revision_id;
        f.service
            .link_agent_run_output_revision(&f.session, &f.run, next)
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, count.clone())
            .try_settle_batch(&f.session, &batch.id)
            .unwrap();
        let state = f.service.get_session_state(&f.session).unwrap();
        assert!(state.workflow.outer_iterations.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(f
            .service
            .get_validation_batch(&f.session, &batch.id)
            .unwrap()
            .unwrap()
            .settled_at
            .is_some());
    }

    #[test]
    fn cancelled_run_is_not_resurrected_by_terminal_batch() {
        let f = fixture();
        let (batch, checks) = create_batch(&f);
        start_all(&f, &checks);
        for check in &checks {
            f.service
                .complete_validation_check(&f.session, &check.id, json!({"passed":true}), true)
                .unwrap();
        }
        f.service
            .update_agent_run(
                &f.session,
                &f.run,
                Some(CadAgentRunStatus::Cancelled),
                Some(None),
                None,
                None,
                None,
            )
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        coordinator(&f, count.clone())
            .try_settle_batch(&f.session, &batch.id)
            .unwrap();
        let state = f.service.get_session_state(&f.session).unwrap();
        assert_eq!(
            state
                .agent_runs
                .iter()
                .find(|run| run.id == f.run)
                .unwrap()
                .status,
            CadAgentRunStatus::Cancelled
        );
        assert!(state.workflow.outer_iterations.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
