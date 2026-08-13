use super::codex_vlm_evaluator::{CodexVlmEvaluationInput, CodexVlmEvaluator};
use super::contract::{
    rendered_image_path, validate_evaluation_kind, validate_report, ValidatedReport,
};
use crate::protocol::{
    CadAgentRunStatus, CadValidationEvaluation, CadValidationEvaluationStatus,
    CadWorkflowOuterIteration, ListCadSessionsInput,
};
use crate::session_service::{timestamp, SessionService};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait VlmEvaluationExecutor: Send + Sync {
    async fn evaluate(&self, input: CodexVlmEvaluationInput) -> Result<Value, String>;
    async fn recover(&self, evaluation: &CadValidationEvaluation) -> Result<Value, String>;
}

#[async_trait]
impl VlmEvaluationExecutor for CodexVlmEvaluator {
    async fn evaluate(&self, input: CodexVlmEvaluationInput) -> Result<Value, String> {
        CodexVlmEvaluator::evaluate(self, input).await
    }

    async fn recover(&self, evaluation: &CadValidationEvaluation) -> Result<Value, String> {
        CodexVlmEvaluator::recover(self, evaluation).await
    }
}

pub type RefinementEnqueue = Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync + 'static>;

fn latest_artifact_id<'a>(
    evaluations: impl Iterator<Item = &'a CadValidationEvaluation>,
) -> Option<&'a str> {
    evaluations
        .max_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.attempt.cmp(&right.attempt))
                .then_with(|| left.id.cmp(&right.id))
        })
        .map(|evaluation| evaluation.artifact_id.as_str())
}

#[derive(Clone)]
pub struct ValidationCoordinator {
    service: Arc<SessionService>,
    evaluator: Arc<dyn VlmEvaluationExecutor>,
    refinement_enqueue: RefinementEnqueue,
    active_sessions: Arc<Mutex<HashSet<String>>>,
    cwd: PathBuf,
}

impl ValidationCoordinator {
    pub fn new(
        service: Arc<SessionService>,
        evaluator: Arc<dyn VlmEvaluationExecutor>,
        refinement_enqueue: RefinementEnqueue,
        cwd: PathBuf,
    ) -> Result<Self, String> {
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(format!(
                "Validation coordinator working directory must be an existing absolute directory: {}",
                cwd.display()
            ));
        }
        Ok(Self {
            service,
            evaluator,
            refinement_enqueue,
            active_sessions: Arc::new(Mutex::new(HashSet::new())),
            cwd,
        })
    }

    pub fn recover_startup(&self) -> Result<(), String> {
        let sessions = self
            .service
            .list_sessions_for_input(ListCadSessionsInput {
                include_archived: true,
                query: None,
            })?
            .sessions;
        for session in sessions {
            let evaluations = self.service.list_validation_evaluations(&session.id)?;
            for evaluation in evaluations.iter().filter(|evaluation| {
                matches!(
                    evaluation.status,
                    CadValidationEvaluationStatus::Queued | CadValidationEvaluationStatus::Running
                )
            }) {
                self.enqueue(evaluation.clone())?;
            }
            let state = self.service.get_session_state(&session.id)?;
            for run in state.agent_runs.iter().filter(|run| {
                !matches!(
                    run.status,
                    CadAgentRunStatus::Completed
                        | CadAgentRunStatus::Failed
                        | CadAgentRunStatus::Cancelled
                )
            }) {
                let Some(output_revision_id) = run.output_revision_id.as_deref() else {
                    continue;
                };
                let current = evaluations
                    .iter()
                    .filter(|evaluation| {
                        evaluation.run_id == run.id && evaluation.revision_id == output_revision_id
                    })
                    .collect::<Vec<_>>();
                if current.is_empty() {
                    continue;
                }
                let latest_artifact = current
                    .iter()
                    .max_by(|left, right| {
                        left.created_at
                            .cmp(&right.created_at)
                            .then_with(|| left.attempt.cmp(&right.attempt))
                            .then_with(|| left.id.cmp(&right.id))
                    })
                    .map(|evaluation| evaluation.artifact_id.as_str())
                    .expect("current validation rows are non-empty");
                let current_batch = current
                    .into_iter()
                    .filter(|evaluation| evaluation.artifact_id == latest_artifact)
                    .collect::<Vec<_>>();
                if current_batch.iter().any(|evaluation| {
                    matches!(
                        evaluation.status,
                        CadValidationEvaluationStatus::Queued
                            | CadValidationEvaluationStatus::Running
                    )
                }) {
                    continue;
                }
                if let Some(failed) = current_batch.iter().find(|evaluation| {
                    evaluation.run_id == run.id
                        && evaluation.status == CadValidationEvaluationStatus::Failed
                }) {
                    self.fail_run_for_evaluation(
                        failed,
                        failed.error.clone().ok_or_else(|| {
                            format!("Failed validation evaluation {} has no error.", failed.id)
                        })?,
                    )?;
                    continue;
                }
                if let Some(succeeded) = current_batch.iter().find(|evaluation| {
                    evaluation.run_id == run.id
                        && evaluation.status == CadValidationEvaluationStatus::Succeeded
                }) {
                    self.settle_terminal_batch(succeeded)?;
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
        let revision_id = run
            .output_revision_id
            .ok_or_else(|| format!("Agent run {run_id} has no output revision for validation."))?;
        let current = self
            .service
            .list_validation_evaluations(session_id)?
            .into_iter()
            .filter(|evaluation| {
                evaluation.run_id == run_id && evaluation.revision_id == revision_id
            })
            .collect::<Vec<_>>();
        let Some(latest_artifact) = current
            .iter()
            .max_by(|left, right| {
                left.created_at
                    .cmp(&right.created_at)
                    .then_with(|| left.attempt.cmp(&right.attempt))
                    .then_with(|| left.id.cmp(&right.id))
            })
            .map(|evaluation| evaluation.artifact_id.clone())
        else {
            return Ok(0);
        };
        let evaluations = current
            .into_iter()
            .filter(|evaluation| {
                evaluation.artifact_id == latest_artifact
                    && matches!(
                        evaluation.status,
                        CadValidationEvaluationStatus::Queued
                            | CadValidationEvaluationStatus::Running
                    )
            })
            .collect::<Vec<_>>();
        for evaluation in &evaluations {
            self.enqueue(evaluation.clone())?;
        }
        Ok(evaluations.len())
    }

    pub fn enqueue(&self, evaluation: CadValidationEvaluation) -> Result<(), String> {
        validate_evaluation_kind(&evaluation)?;
        if !matches!(
            evaluation.status,
            CadValidationEvaluationStatus::Queued | CadValidationEvaluationStatus::Running
        ) {
            return Err(format!(
                "Only queued/running validation evaluations can be enqueued: {}",
                evaluation.id
            ));
        }
        let mut active = self
            .active_sessions
            .lock()
            .map_err(|_| "Validation coordinator active-session lock is poisoned.".to_string())?;
        if !active.insert(evaluation.session_id.clone()) {
            return Ok(());
        }
        drop(active);

        let coordinator = self.clone();
        tauri::async_runtime::spawn(async move {
            let session_id = evaluation.session_id.clone();
            if let Err(error) = coordinator.drain_session(&session_id).await {
                eprintln!("[cadastrophe:validation] session_id={session_id} error={error}");
            }
        });
        Ok(())
    }

    async fn drain_session(&self, session_id: &str) -> Result<(), String> {
        loop {
            let mut candidates = self.pending_candidates(session_id)?;
            candidates.sort_by(|left, right| {
                let rank = |status: &CadValidationEvaluationStatus| match status {
                    CadValidationEvaluationStatus::Running => 0,
                    CadValidationEvaluationStatus::Queued => 1,
                    _ => 2,
                };
                rank(&left.status)
                    .cmp(&rank(&right.status))
                    .then_with(|| left.created_at.cmp(&right.created_at))
                    .then_with(|| left.attempt.cmp(&right.attempt))
                    .then_with(|| left.id.cmp(&right.id))
            });
            let Some(next) = candidates.into_iter().next() else {
                let mut active = self.active_sessions.lock().map_err(|_| {
                    "Validation coordinator active-session lock is poisoned.".to_string()
                })?;
                let has_pending = !self.pending_candidates(session_id)?.is_empty();
                if has_pending {
                    drop(active);
                    continue;
                }
                active.remove(session_id);
                return Ok(());
            };
            if let Err(error) = self.execute(next).await {
                let mut active = self.active_sessions.lock().map_err(|_| {
                    "Validation coordinator active-session lock is poisoned.".to_string()
                })?;
                active.remove(session_id);
                return Err(error);
            }
        }
    }

    fn pending_candidates(&self, session_id: &str) -> Result<Vec<CadValidationEvaluation>, String> {
        Ok(self
            .service
            .list_validation_evaluations(session_id)?
            .into_iter()
            .filter(|evaluation| {
                matches!(
                    evaluation.status,
                    CadValidationEvaluationStatus::Queued | CadValidationEvaluationStatus::Running
                )
            })
            .collect())
    }

    async fn execute(&self, evaluation: CadValidationEvaluation) -> Result<(), String> {
        let result = match evaluation.status {
            CadValidationEvaluationStatus::Queued => match rendered_image_path(&evaluation) {
                Ok(image_path) => {
                    self.evaluator
                        .evaluate(CodexVlmEvaluationInput {
                            evaluation: evaluation.clone(),
                            rendered_image_path: image_path,
                            cwd: self.cwd.clone(),
                            app_data_dir: self.service.app_data_dir().to_path_buf(),
                        })
                        .await
                }
                Err(error) => Err(format!("VLM rendered image validation failed: {error}")),
            },
            CadValidationEvaluationStatus::Running => self.evaluator.recover(&evaluation).await,
            _ => unreachable!("enqueue validates non-terminal status"),
        };

        match result {
            Ok(report) => match validate_report(&evaluation, report) {
                Ok(validated) => self.apply_valid_report(&evaluation, validated),
                Err(error) => self.apply_operational_failure(
                    &evaluation,
                    format!("VLM evaluation report contract validation failed: {error}"),
                ),
            },
            Err(error) => self.apply_operational_failure(
                &evaluation,
                format!("VLM evaluation execution failed: {error}"),
            ),
        }
    }

    fn apply_valid_report(
        &self,
        snapshot: &CadValidationEvaluation,
        validated: ValidatedReport,
    ) -> Result<(), String> {
        let current = self
            .service
            .get_validation_evaluation(&snapshot.session_id, &snapshot.id)?
            .ok_or_else(|| format!("Validation evaluation not found: {}", snapshot.id))?;
        if current.status != CadValidationEvaluationStatus::Running {
            return Err(format!(
                "VLM evaluator returned a report before evaluation {} reached running status.",
                snapshot.id
            ));
        }
        self.service.complete_validation_evaluation(
            &current.session_id,
            &current.id,
            validated.report.clone(),
            validated.score,
            validated.passed,
        )?;
        self.settle_terminal_batch(&current)
    }

    fn settle_terminal_batch(&self, current: &CadValidationEvaluation) -> Result<(), String> {
        let run = self
            .service
            .get_agent_run(&current.session_id, &current.run_id)?
            .ok_or_else(|| format!("Agent run not found: {}", current.run_id))?;
        if run.output_revision_id.as_deref() != Some(current.revision_id.as_str()) {
            return Ok(());
        }
        let all = self
            .service
            .list_validation_evaluations(&current.session_id)?;
        if latest_artifact_id(all.iter().filter(|candidate| {
            candidate.run_id == current.run_id && candidate.revision_id == current.revision_id
        })) != Some(current.artifact_id.as_str())
        {
            return Ok(());
        }
        let batch = self.validation_batch(current)?;
        if batch.iter().any(|evaluation| {
            matches!(
                evaluation.status,
                CadValidationEvaluationStatus::Queued | CadValidationEvaluationStatus::Running
            )
        }) {
            return Ok(());
        }
        if let Some(failed) = batch
            .iter()
            .find(|evaluation| evaluation.status == CadValidationEvaluationStatus::Failed)
        {
            let error = failed.error.clone().ok_or_else(|| {
                format!("Failed validation evaluation {} has no error.", failed.id)
            })?;
            self.fail_run_for_evaluation(failed, error)?;
            return Ok(());
        }
        let representative_evaluation = batch
            .iter()
            .find(|evaluation| evaluation.passed == Some(false))
            .or_else(|| batch.iter().find(|evaluation| evaluation.id == current.id))
            .ok_or_else(|| "Completed validation batch is empty.".to_string())?;
        let representative_report = representative_evaluation.report.clone().ok_or_else(|| {
            format!(
                "Succeeded validation evaluation {} has no report.",
                representative_evaluation.id
            )
        })?;
        let representative = validate_report(representative_evaluation, representative_report)?;
        let state = self.service.get_session_state(&current.session_id)?;
        let already_persisted = state.workflow.outer_iterations.iter().any(|iteration| {
            iteration.run_id == current.run_id
                && iteration.revision_id.as_deref() == Some(current.revision_id.as_str())
                && iteration.vlm_report.as_ref() == Some(&representative.report)
        });
        if !already_persisted {
            self.persist_outer_iteration(current, &representative)?;
        }
        self.service
            .clear_workflow_pending_vlm(&current.session_id, &current.run_id)?;

        if representative.passed {
            self.service.update_agent_run(
                &current.session_id,
                &current.run_id,
                Some(CadAgentRunStatus::Completed),
                Some(None),
                None,
                None,
                Some(json!({
                    "evaluationId": representative_evaluation.id,
                    "score": representative.score,
                    "passed": true,
                    "nextAction": "complete"
                })),
            )?;
        } else {
            self.service.prepare_agent_run_refinement_turn(
                &current.session_id,
                &current.run_id,
                &representative_evaluation.id,
            )?;
            (self.refinement_enqueue)(&current.session_id, &current.run_id)?;
        }
        Ok(())
    }

    fn validation_batch(
        &self,
        evaluation: &CadValidationEvaluation,
    ) -> Result<Vec<CadValidationEvaluation>, String> {
        Ok(self
            .service
            .list_validation_evaluations(&evaluation.session_id)?
            .into_iter()
            .filter(|candidate| {
                candidate.run_id == evaluation.run_id
                    && candidate.revision_id == evaluation.revision_id
                    && candidate.artifact_id == evaluation.artifact_id
                    && candidate.kind == evaluation.kind
            })
            .collect())
    }

    fn persist_outer_iteration(
        &self,
        evaluation: &CadValidationEvaluation,
        validated: &ValidatedReport,
    ) -> Result<(), String> {
        let contract = evaluation
            .input_contract
            .as_object()
            .ok_or_else(|| "Persisted VLM input contract is not an object.".to_string())?;
        let structural_report = contract.get("structuralReport").cloned().ok_or_else(|| {
            "Persisted VLM input contract is missing structuralReport.".to_string()
        })?;
        let dfm_report = contract
            .get("dfmReport")
            .cloned()
            .ok_or_else(|| "Persisted VLM input contract is missing dfmReport.".to_string())?;
        let state = self.service.get_session_state(&evaluation.session_id)?;
        let iteration = state
            .workflow
            .outer_iterations
            .iter()
            .filter(|iteration| iteration.run_id == evaluation.run_id)
            .map(|iteration| iteration.iteration)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "Workflow outer iteration overflowed u64.".to_string())?;
        self.service.save_workflow_outer_iteration(
            &evaluation.session_id,
            CadWorkflowOuterIteration {
                id: format!("workflow-outer-{}-{iteration}", evaluation.run_id),
                run_id: evaluation.run_id.clone(),
                iteration,
                revision_id: Some(evaluation.revision_id.clone()),
                structural_report,
                dfm_report: Some(dfm_report),
                vlm_report: Some(validated.report.clone()),
                failure_report: validated.failure_report.clone(),
                passed: validated.passed,
                created_at: timestamp(),
            },
        )?;
        Ok(())
    }

    fn apply_operational_failure(
        &self,
        evaluation: &CadValidationEvaluation,
        error: String,
    ) -> Result<(), String> {
        self.service.fail_validation_evaluation(
            &evaluation.session_id,
            &evaluation.id,
            error.clone(),
        )?;
        let persisted = self
            .service
            .get_validation_evaluation(&evaluation.session_id, &evaluation.id)?
            .ok_or_else(|| format!("Validation evaluation not found: {}", evaluation.id))?;
        self.settle_terminal_batch(&persisted)
    }

    fn fail_run_for_evaluation(
        &self,
        evaluation: &CadValidationEvaluation,
        error: String,
    ) -> Result<(), String> {
        self.service.update_agent_run(
            &evaluation.session_id,
            &evaluation.run_id,
            Some(CadAgentRunStatus::Failed),
            Some(None),
            Some(error.clone()),
            None,
            Some(json!({
                "evaluationId": evaluation.id,
                "validationFailed": true,
                "error": error
            })),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::*;
    use base64::Engine;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeEvaluator {
        service: Arc<SessionService>,
        outcomes: Mutex<VecDeque<Outcome>>,
    }

    #[derive(Clone, Copy)]
    enum Outcome {
        Pass,
        Reject,
        Malformed,
    }

    #[async_trait]
    impl VlmEvaluationExecutor for FakeEvaluator {
        async fn evaluate(&self, input: CodexVlmEvaluationInput) -> Result<Value, String> {
            let evaluation = input.evaluation;
            let now = timestamp();
            let thread_id = format!("validation-thread-{}", evaluation.id);
            self.service.upsert_agent_thread(CadAgentThread {
                id: thread_id.clone(),
                session_id: evaluation.session_id.clone(),
                plane: CadAgentPlane::Validation,
                owner_id: evaluation.id.clone(),
                external_agent: "fake-vlm".to_string(),
                external_thread_id: format!("external-{}", evaluation.id),
                status: CadAgentThreadStatus::Active,
                connection_generation: Some(1),
                created_at: now.clone(),
                updated_at: now.clone(),
                last_resumed_at: None,
                archived_at: None,
                replaced_by_id: None,
                metadata: None,
            })?;
            self.service.bind_validation_evaluation(
                &evaluation.session_id,
                &evaluation.id,
                &thread_id,
                &format!("turn-{}", evaluation.id),
            )?;
            let mut thread = self
                .service
                .list_agent_threads(&evaluation.session_id)?
                .into_iter()
                .find(|thread| thread.id == thread_id)
                .ok_or_else(|| "Fake validation thread disappeared.".to_string())?;
            thread.status = CadAgentThreadStatus::Archived;
            thread.updated_at = timestamp();
            thread.archived_at = Some(timestamp());
            self.service.upsert_agent_thread(thread)?;
            let outcome = self
                .outcomes
                .lock()
                .map_err(|_| "Fake evaluator outcome lock is poisoned.".to_string())?
                .pop_front()
                .ok_or_else(|| "Fake evaluator has no outcome.".to_string())?;
            match outcome {
                Outcome::Pass => Ok(report(&evaluation, true)),
                Outcome::Reject => Ok(report(&evaluation, false)),
                Outcome::Malformed => Ok(json!({"contractType": "wrong"})),
            }
        }

        async fn recover(&self, evaluation: &CadValidationEvaluation) -> Result<Value, String> {
            let outcome = self
                .outcomes
                .lock()
                .map_err(|_| "Fake evaluator outcome lock is poisoned.".to_string())?
                .pop_front()
                .ok_or_else(|| "Fake evaluator has no recovery outcome.".to_string())?;
            match outcome {
                Outcome::Pass => Ok(report(evaluation, true)),
                Outcome::Reject => Ok(report(evaluation, false)),
                Outcome::Malformed => Ok(json!({"contractType": "wrong"})),
            }
        }
    }

    fn report(evaluation: &CadValidationEvaluation, passed: bool) -> Value {
        let (scores, composite, score) = if passed {
            (
                json!({"structure": 3, "components": 3, "proportions": 3}),
                9,
                1.0,
            )
        } else {
            (
                json!({"structure": 2, "components": 1, "proportions": 2}),
                5,
                5.0 / 9.0,
            )
        };
        json!({
            "contractType": "cadastrophe.vlm_judge_report.v1",
            "evaluationId": evaluation.id,
            "sessionId": evaluation.session_id,
            "runId": evaluation.run_id,
            "revisionId": evaluation.revision_id,
            "artifactId": evaluation.artifact_id,
            "kind": "vlm",
            "attempt": evaluation.attempt,
            "score": score,
            "passed": passed,
            "scores": scores,
            "composite": composite,
            "findings": if passed { json!([]) } else { json!([{"severity": "major", "message": "support tab is missing"}]) },
            "enumeration": [{"planName": "support tab", "observed": if passed { "present" } else { "missing" }}],
            "inconsistencies": if passed { json!([]) } else { json!(["support tab absent"]) },
            "diagnostic": if passed { "accepted" } else { "requested component is absent" },
            "failureReport": if passed { Value::Null } else { json!({
                "contractType": "cadastrophe.failure_report.v1",
                "reason": "major_component_missing",
                "summary": "support tab is missing",
                "nextAction": "outer_loop_refine_source"
            }) }
        })
    }

    struct Fixture {
        service: Arc<SessionService>,
        session_id: String,
        run_id: String,
        revision_id: String,
        artifact_id: String,
        input_contract: Value,
        cwd: PathBuf,
    }

    fn fixture() -> Fixture {
        let cwd =
            std::env::temp_dir().join(format!("validation-coordinator-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&cwd).unwrap();
        let service = Arc::new(SessionService::new(cwd.clone()));
        let session = service
            .create_session(CreateCadSessionInput::default())
            .unwrap();
        let (run, _) = service
            .create_agent_run(
                &session.session_id,
                "Create a bracket with a support tab.".to_string(),
                None,
                Some("fake-modeler".to_string()),
                None,
            )
            .unwrap();
        let revision_id = service
            .update_model_source(UpdateModelSourceInput {
                session_id: session.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "cube([2,2,2]);".to_string(),
                parent_revision_id: None,
                parameters: None,
            })
            .unwrap()
            .revision_id;
        service
            .link_agent_run_output_revision(&session.session_id, &run.id, revision_id.clone())
            .unwrap();
        let artifact = service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: session.session_id.clone(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::Stl,
                format: "stl".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"solid test\nendsolid test\n"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 0,
                    items: vec![],
                },
                metadata: Metadata::new(),
            })
            .unwrap()
            .artifact;
        let image = service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: session.session_id.clone(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::RenderImage,
                format: "png".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD.encode(b"png fixture"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 0,
                    items: vec![],
                },
                metadata: Metadata::new(),
            })
            .unwrap()
            .artifact;
        let metadata = image.metadata.as_ref().unwrap();
        let image_path = metadata
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                cwd.join(metadata["relativePath"].as_str().unwrap())
                    .to_str()
                    .unwrap()
                    .to_string()
            });
        let input_contract = json!({
            "contractType": "cadastrophe.vlm_evaluation_input.v1",
            "sessionId": session.session_id,
            "runId": run.id,
            "revisionId": revision_id,
            "artifactId": artifact.id,
            "kind": "vlm",
            "userRequest": run.prompt,
            "passThreshold": 0.8,
            "renderedImage": {
                "artifactId": image.id,
                "path": image_path,
                "sha256": metadata["sha256"]
            },
            "structuralReport": {"contractType": "cadastrophe.structural_report.v1", "passed": true},
            "dfmReport": {"contractType": "cadastrophe.dfm_report.v1", "passed": true}
        });
        Fixture {
            service,
            session_id: session.session_id,
            run_id: run.id,
            revision_id,
            artifact_id: artifact.id,
            input_contract,
            cwd,
        }
    }

    fn create_attempt(fixture: &Fixture) -> CadValidationEvaluation {
        fixture
            .service
            .create_next_validation_evaluation(CadValidationEvaluationCreate {
                session_id: fixture.session_id.clone(),
                run_id: fixture.run_id.clone(),
                revision_id: fixture.revision_id.clone(),
                artifact_id: fixture.artifact_id.clone(),
                kind: CadValidationEvaluationKind::Vlm,
                input_contract: fixture.input_contract.clone(),
                pass_threshold: 0.8,
            })
            .unwrap()
    }

    fn create_new_revision_attempt(fixture: &Fixture) -> CadValidationEvaluation {
        let revision_id = fixture
            .service
            .update_model_source(UpdateModelSourceInput {
                session_id: fixture.session_id.clone(),
                source_language: CadSourceLanguage::Openscad,
                source: "cube([6,6,6]);".to_string(),
                parent_revision_id: Some(fixture.revision_id.clone()),
                parameters: None,
            })
            .unwrap()
            .revision_id;
        fixture
            .service
            .link_agent_run_output_revision(
                &fixture.session_id,
                &fixture.run_id,
                revision_id.clone(),
            )
            .unwrap();
        let artifact = fixture
            .service
            .persist_runtime_artifact(PersistRuntimeArtifactInput {
                session_id: fixture.session_id.clone(),
                revision_id: revision_id.clone(),
                kind: CadArtifactKind::Stl,
                format: "stl".to_string(),
                contents_base64: base64::engine::general_purpose::STANDARD
                    .encode(b"solid latest\nendsolid latest\n"),
                diagnostics: CadDiagnostics {
                    ok: true,
                    elapsed_ms: 0,
                    items: vec![],
                },
                metadata: Metadata::new(),
            })
            .unwrap()
            .artifact;
        fixture
            .service
            .create_next_validation_evaluation(CadValidationEvaluationCreate {
                session_id: fixture.session_id.clone(),
                run_id: fixture.run_id.clone(),
                revision_id: revision_id.clone(),
                artifact_id: artifact.id.clone(),
                kind: CadValidationEvaluationKind::Vlm,
                input_contract: json!({
                    "contractType": "cadastrophe.vlm_evaluation_input.v1",
                    "sessionId": fixture.session_id,
                    "runId": fixture.run_id,
                    "revisionId": revision_id,
                    "artifactId": artifact.id,
                    "kind": "vlm",
                    "userRequest": "Create a bracket with a support tab.",
                    "passThreshold": 0.8,
                    "renderedImage": fixture.input_contract["renderedImage"],
                    "structuralReport": {"contractType": "cadastrophe.structural_report.v1", "passed": true},
                    "dfmReport": {"contractType": "cadastrophe.dfm_report.v1", "passed": true}
                }),
                pass_threshold: 0.8,
            })
            .unwrap()
    }

    fn bind_running(
        fixture: &Fixture,
        evaluation: &CadValidationEvaluation,
    ) -> CadValidationEvaluation {
        let now = timestamp();
        let thread_id = format!("recovery-thread-{}", evaluation.id);
        fixture
            .service
            .upsert_agent_thread(CadAgentThread {
                id: thread_id.clone(),
                session_id: fixture.session_id.clone(),
                plane: CadAgentPlane::Validation,
                owner_id: evaluation.id.clone(),
                external_agent: "fake-vlm".to_string(),
                external_thread_id: format!("recovery-external-{}", evaluation.id),
                status: CadAgentThreadStatus::NotLoaded,
                connection_generation: None,
                created_at: now.clone(),
                updated_at: now,
                last_resumed_at: None,
                archived_at: None,
                replaced_by_id: None,
                metadata: None,
            })
            .unwrap();
        fixture
            .service
            .bind_validation_evaluation(
                &fixture.session_id,
                &evaluation.id,
                &thread_id,
                &format!("recovery-turn-{}", evaluation.id),
            )
            .unwrap()
    }

    fn finish_terminal(
        fixture: &Fixture,
        evaluation: &CadValidationEvaluation,
        passed: bool,
    ) -> CadValidationEvaluation {
        let running = bind_running(fixture, evaluation);
        let thread_id = running.evaluator_thread_id.clone().unwrap();
        let mut thread = fixture
            .service
            .list_agent_threads(&fixture.session_id)
            .unwrap()
            .into_iter()
            .find(|thread| thread.id == thread_id)
            .unwrap();
        thread.status = CadAgentThreadStatus::Archived;
        thread.archived_at = Some(timestamp());
        thread.updated_at = timestamp();
        fixture.service.upsert_agent_thread(thread).unwrap();
        let report = report(&running, passed);
        let score = report["score"].as_f64().unwrap();
        fixture
            .service
            .complete_validation_evaluation(&fixture.session_id, &running.id, report, score, passed)
            .unwrap()
    }

    async fn wait_terminal(
        service: &SessionService,
        session_id: &str,
        count: usize,
    ) -> CadSessionState {
        for _ in 0..100 {
            let state = service.get_session_state(session_id).unwrap();
            if state.validation_evaluations.len() == count
                && state.validation_evaluations.iter().all(|evaluation| {
                    matches!(
                        evaluation.status,
                        CadValidationEvaluationStatus::Succeeded
                            | CadValidationEvaluationStatus::Failed
                    )
                })
            {
                return state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("validation evaluations did not become terminal")
    }

    #[tokio::test]
    async fn sequential_batch_passes_complete_run_without_mixing_conversation_or_transport() {
        let fixture = fixture();
        let first = create_attempt(&fixture);
        let second = create_attempt(&fixture);
        let refinements = Arc::new(AtomicUsize::new(0));
        let refinement_count = Arc::clone(&refinements);
        let coordinator = ValidationCoordinator::new(
            Arc::clone(&fixture.service),
            Arc::new(FakeEvaluator {
                service: Arc::clone(&fixture.service),
                outcomes: Mutex::new(VecDeque::from([Outcome::Pass, Outcome::Pass])),
            }),
            Arc::new(move |_, _| {
                refinement_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            fixture.cwd.clone(),
        )
        .unwrap();
        coordinator.enqueue(first).unwrap();
        coordinator.enqueue(second).unwrap();
        let state = wait_terminal(&fixture.service, &fixture.session_id, 2).await;
        assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Completed);
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert_eq!(refinements.load(Ordering::SeqCst), 0);
        assert!(state.conversation.is_empty());
        assert_ne!(
            state.validation_evaluations[0].evaluator_thread_id,
            state.validation_evaluations[1].evaluator_thread_id
        );
        assert_ne!(
            state.validation_evaluations[0].external_turn_id,
            state.validation_evaluations[1].external_turn_id
        );
        let validation_threads = state
            .agent_threads
            .iter()
            .filter(|thread| thread.plane == CadAgentPlane::Validation)
            .collect::<Vec<_>>();
        assert_eq!(validation_threads.len(), 2);
        assert!(validation_threads.iter().all(|thread| {
            thread.status == CadAgentThreadStatus::Archived && thread.archived_at.is_some()
        }));
        assert_eq!(
            fixture
                .service
                .agent_session_diagnostics(&fixture.session_id)
                .unwrap()
                .transport_event_count,
            0
        );
        assert!(fixture
            .service
            .list_validation_evaluation_events(
                &fixture.session_id,
                &state.validation_evaluations[0].id
            )
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn rejecting_batch_refines_once_after_all_attempts_are_terminal() {
        let fixture = fixture();
        let first = create_attempt(&fixture);
        let second = create_attempt(&fixture);
        let refinements = Arc::new(AtomicUsize::new(0));
        let refinement_count = Arc::clone(&refinements);
        let coordinator = ValidationCoordinator::new(
            Arc::clone(&fixture.service),
            Arc::new(FakeEvaluator {
                service: Arc::clone(&fixture.service),
                outcomes: Mutex::new(VecDeque::from([Outcome::Reject, Outcome::Pass])),
            }),
            Arc::new(move |_, _| {
                refinement_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            fixture.cwd.clone(),
        )
        .unwrap();
        coordinator.enqueue(first).unwrap();
        coordinator.enqueue(second).unwrap();
        let state = wait_terminal(&fixture.service, &fixture.session_id, 2).await;
        for _ in 0..200 {
            if refinements.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert_eq!(refinements.load(Ordering::SeqCst), 1);
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert!(!state.workflow.outer_iterations[0].passed);
        assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Running);
        assert!(state.agent_runs[0].external_turn_id.is_none());
    }

    #[tokio::test]
    async fn malformed_report_fails_evaluation_and_run_without_synthetic_result() {
        let fixture = fixture();
        let evaluation = create_attempt(&fixture);
        let coordinator = ValidationCoordinator::new(
            Arc::clone(&fixture.service),
            Arc::new(FakeEvaluator {
                service: Arc::clone(&fixture.service),
                outcomes: Mutex::new(VecDeque::from([Outcome::Malformed])),
            }),
            Arc::new(|_, _| Err("refinement must not run after malformed JSON".to_string())),
            fixture.cwd.clone(),
        )
        .unwrap();
        coordinator.enqueue(evaluation).unwrap();
        let state = wait_terminal(&fixture.service, &fixture.session_id, 1).await;
        assert_eq!(
            state.validation_evaluations[0].status,
            CadValidationEvaluationStatus::Failed
        );
        assert!(state.validation_evaluations[0].report.is_none());
        assert!(state.validation_evaluations[0].passed.is_none());
        assert!(state.validation_evaluations[0]
            .error
            .as_deref()
            .unwrap()
            .contains("contract validation failed"));
        assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Failed);
        assert!(state.workflow.outer_iterations.is_empty());
    }

    #[tokio::test]
    async fn startup_recovery_executes_queued_and_recovers_running_evaluations() {
        for running in [false, true] {
            let fixture = fixture();
            let evaluation = create_attempt(&fixture);
            if running {
                bind_running(&fixture, &evaluation);
            }
            let coordinator = ValidationCoordinator::new(
                Arc::clone(&fixture.service),
                Arc::new(FakeEvaluator {
                    service: Arc::clone(&fixture.service),
                    outcomes: Mutex::new(VecDeque::from([Outcome::Pass])),
                }),
                Arc::new(|_, _| Err("passing recovery must not refine".to_string())),
                fixture.cwd.clone(),
            )
            .unwrap();
            coordinator.recover_startup().unwrap();
            let state = wait_terminal(&fixture.service, &fixture.session_id, 1).await;
            assert_eq!(
                state.validation_evaluations[0].status,
                CadValidationEvaluationStatus::Succeeded
            );
            assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Completed);
        }
    }

    #[tokio::test]
    async fn terminal_recovery_ignores_stale_rejected_revision_and_settles_latest_pass() {
        let fixture = fixture();
        let old = create_attempt(&fixture);
        finish_terminal(&fixture, &old, false);

        let latest = create_new_revision_attempt(&fixture);
        finish_terminal(&fixture, &latest, true);
        assert!(fixture
            .service
            .list_startup_agent_run_recovery_candidates()
            .unwrap()
            .is_empty());

        let coordinator = ValidationCoordinator::new(
            Arc::clone(&fixture.service),
            Arc::new(FakeEvaluator {
                service: Arc::clone(&fixture.service),
                outcomes: Mutex::new(VecDeque::new()),
            }),
            Arc::new(|_, _| Err("stale rejection must not refine".to_string())),
            fixture.cwd.clone(),
        )
        .unwrap();
        coordinator.recover_startup().unwrap();
        let state = fixture
            .service
            .get_session_state(&fixture.session_id)
            .unwrap();
        assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Completed);
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert!(state.workflow.outer_iterations[0].passed);
        assert_eq!(
            state.workflow.outer_iterations[0].revision_id.as_deref(),
            Some(latest.revision_id.as_str())
        );
    }

    #[tokio::test]
    async fn startup_drains_stale_queued_revision_without_mutating_latest_batch_outcome() {
        let fixture = fixture();
        let stale = create_attempt(&fixture);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let latest = create_new_revision_attempt(&fixture);
        let refinements = Arc::new(AtomicUsize::new(0));
        let refinement_count = Arc::clone(&refinements);
        let coordinator = ValidationCoordinator::new(
            Arc::clone(&fixture.service),
            Arc::new(FakeEvaluator {
                service: Arc::clone(&fixture.service),
                outcomes: Mutex::new(VecDeque::from([Outcome::Reject, Outcome::Pass])),
            }),
            Arc::new(move |_, _| {
                refinement_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            fixture.cwd.clone(),
        )
        .unwrap();
        coordinator.recover_startup().unwrap();
        wait_terminal(&fixture.service, &fixture.session_id, 2).await;
        for _ in 0..200 {
            if fixture
                .service
                .get_agent_run(&fixture.session_id, &fixture.run_id)
                .unwrap()
                .unwrap()
                .status
                == CadAgentRunStatus::Completed
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let state = fixture
            .service
            .get_session_state(&fixture.session_id)
            .unwrap();
        assert_eq!(state.agent_runs[0].status, CadAgentRunStatus::Completed);
        assert_eq!(refinements.load(Ordering::SeqCst), 0);
        assert_eq!(state.workflow.outer_iterations.len(), 1);
        assert_eq!(
            state.workflow.outer_iterations[0].revision_id.as_deref(),
            Some(latest.revision_id.as_str())
        );
        assert_eq!(
            state
                .validation_evaluations
                .iter()
                .find(|evaluation| evaluation.id == stale.id)
                .unwrap()
                .passed,
            Some(false)
        );
    }
}
