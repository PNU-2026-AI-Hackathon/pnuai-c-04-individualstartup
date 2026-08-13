use super::*;

impl SessionService {
    pub fn list_validation_evaluations(
        &self,
        session_id: &str,
    ) -> Result<Vec<CadValidationEvaluation>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .validation_evaluations
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn get_validation_evaluation(
        &self,
        session_id: &str,
        evaluation_id: &str,
    ) -> Result<Option<CadValidationEvaluation>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .validation_evaluations
            .get(session_id)
            .into_iter()
            .flatten()
            .find(|evaluation| evaluation.id == evaluation_id)
            .cloned())
    }

    #[cfg(test)]
    pub(crate) fn create_validation_evaluation(
        &self,
        evaluation: CadValidationEvaluation,
    ) -> Result<CadValidationEvaluation, String> {
        validate_validation_evaluation_fields(&evaluation)?;
        if evaluation.status != CadValidationEvaluationStatus::Queued {
            return Err("A validation evaluation test fixture must start queued.".into());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        validate_validation_evaluation_graph(&state, &evaluation)?;
        validate_validation_evaluation_current_output(&state, &evaluation)?;
        let evaluations = state
            .validation_evaluations
            .entry(evaluation.session_id.clone())
            .or_default();
        if evaluations.iter().any(|candidate| {
            candidate.id == evaluation.id
                || (candidate.run_id == evaluation.run_id
                    && candidate.revision_id == evaluation.revision_id
                    && candidate.artifact_id == evaluation.artifact_id
                    && candidate.kind == evaluation.kind
                    && candidate.attempt == evaluation.attempt)
        }) {
            return Err(format!(
                "Validation evaluation test fixture conflicts: {}",
                evaluation.id
            ));
        }
        let saved = self.repository.create_validation_evaluation(&evaluation)?;
        evaluations.push(saved.clone());
        sort_validation_evaluations(evaluations);
        Ok(saved)
    }

    pub fn create_next_validation_evaluation(
        &self,
        input: CadValidationEvaluationCreate,
    ) -> Result<CadValidationEvaluation, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let next_attempt = state
            .validation_evaluations
            .get(&input.session_id)
            .into_iter()
            .flatten()
            .filter(|candidate| {
                candidate.run_id == input.run_id
                    && candidate.revision_id == input.revision_id
                    && candidate.artifact_id == input.artifact_id
                    && candidate.kind == input.kind
            })
            .map(|candidate| candidate.attempt)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "Validation evaluation attempt overflowed u32.".to_string())?;
        let evaluation = CadValidationEvaluation {
            id: uuid(),
            session_id: input.session_id,
            run_id: input.run_id,
            revision_id: input.revision_id,
            artifact_id: input.artifact_id,
            kind: input.kind,
            attempt: next_attempt,
            status: CadValidationEvaluationStatus::Queued,
            evaluator_thread_id: None,
            external_turn_id: None,
            input_contract: input.input_contract,
            report: None,
            passed: None,
            score: None,
            pass_threshold: input.pass_threshold,
            error: None,
            created_at: timestamp(),
            started_at: None,
            completed_at: None,
        };
        validate_validation_evaluation_fields(&evaluation)?;
        validate_validation_evaluation_graph(&state, &evaluation)?;
        validate_validation_evaluation_current_output(&state, &evaluation)?;
        let saved = self
            .repository
            .create_next_validation_evaluation(&evaluation)?;
        validate_saved_evaluation_contract_identity(&saved)?;
        let evaluations = state
            .validation_evaluations
            .entry(saved.session_id.clone())
            .or_default();
        if evaluations.iter().any(|candidate| {
            candidate.id == saved.id
                || (candidate.run_id == saved.run_id
                    && candidate.revision_id == saved.revision_id
                    && candidate.artifact_id == saved.artifact_id
                    && candidate.kind == saved.kind
                    && candidate.attempt == saved.attempt)
        }) {
            return Err(format!(
                "Repository returned a duplicate validation evaluation attempt: {} (attempt {})",
                saved.id, saved.attempt
            ));
        }
        evaluations.push(saved.clone());
        sort_validation_evaluations(evaluations);
        Ok(saved)
    }

    pub fn update_validation_evaluation(
        &self,
        evaluation: CadValidationEvaluation,
    ) -> Result<CadValidationEvaluation, String> {
        validate_validation_evaluation_fields(&evaluation)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        validate_validation_evaluation_graph(&state, &evaluation)?;
        let persisted =
            require_validation_evaluation(&state, &evaluation.session_id, &evaluation.id)?.clone();
        validate_evaluation_immutable_fields(&persisted, &evaluation)?;
        if persisted == evaluation {
            return Ok(persisted);
        }
        validate_evaluation_status_transition(&persisted.status, &evaluation.status)?;
        let saved = self.repository.update_validation_evaluation(&evaluation)?;
        let evaluations = state
            .validation_evaluations
            .get_mut(&evaluation.session_id)
            .expect("evaluation session state checked");
        let slot = evaluations
            .iter_mut()
            .find(|candidate| candidate.id == evaluation.id)
            .expect("evaluation checked");
        *slot = saved.clone();
        Ok(saved)
    }

    pub fn bind_validation_evaluation(
        &self,
        session_id: &str,
        evaluation_id: &str,
        evaluator_thread_id: &str,
        external_turn_id: &str,
    ) -> Result<CadValidationEvaluation, String> {
        if evaluator_thread_id.trim().is_empty() || external_turn_id.trim().is_empty() {
            return Err(
                "Validation evaluation thread and turn identifiers cannot be empty.".into(),
            );
        }
        let evaluation = {
            let state = self.inner.lock().map_err(lock_error)?;
            let mut evaluation =
                require_validation_evaluation(&state, session_id, evaluation_id)?.clone();
            if evaluation.status != CadValidationEvaluationStatus::Queued {
                return Err(format!(
                    "Only a queued validation evaluation can be bound: {evaluation_id}"
                ));
            }
            let thread = state
                .agent_threads
                .get(session_id)
                .into_iter()
                .flatten()
                .find(|thread| thread.id == evaluator_thread_id)
                .ok_or_else(|| format!("Agent thread not found: {evaluator_thread_id}"))?;
            if thread.plane != CadAgentPlane::Validation || thread.owner_id != evaluation_id {
                return Err(format!(
                    "Validation evaluator thread scope does not own evaluation {evaluation_id}."
                ));
            }
            if thread.archived_at.is_some() || thread.replaced_by_id.is_some() {
                return Err(format!(
                    "Cannot bind validation evaluation to inactive thread {evaluator_thread_id}."
                ));
            }
            evaluation.status = CadValidationEvaluationStatus::Running;
            evaluation.evaluator_thread_id = Some(evaluator_thread_id.to_string());
            evaluation.external_turn_id = Some(external_turn_id.to_string());
            evaluation.started_at = Some(timestamp());
            evaluation
        };
        self.update_validation_evaluation(evaluation)
    }

    pub fn complete_validation_evaluation(
        &self,
        session_id: &str,
        evaluation_id: &str,
        report: Value,
        score: f64,
        passed: bool,
    ) -> Result<CadValidationEvaluation, String> {
        if !report.is_object() {
            return Err("Validation evaluation report must be a JSON object.".into());
        }
        let evaluation = {
            let state = self.inner.lock().map_err(lock_error)?;
            let mut evaluation =
                require_validation_evaluation(&state, session_id, evaluation_id)?.clone();
            if evaluation.status != CadValidationEvaluationStatus::Running {
                return Err(format!(
                    "Only a running validation evaluation can succeed: {evaluation_id}"
                ));
            }
            evaluation.status = CadValidationEvaluationStatus::Succeeded;
            evaluation.report = Some(report);
            evaluation.score = Some(score);
            evaluation.passed = Some(passed);
            evaluation.completed_at = Some(timestamp());
            evaluation
        };
        self.update_validation_evaluation(evaluation)
    }

    pub fn fail_validation_evaluation(
        &self,
        session_id: &str,
        evaluation_id: &str,
        error: String,
    ) -> Result<CadValidationEvaluation, String> {
        if error.trim().is_empty() {
            return Err("Validation evaluation failure error cannot be empty.".into());
        }
        let evaluation = {
            let state = self.inner.lock().map_err(lock_error)?;
            let mut evaluation =
                require_validation_evaluation(&state, session_id, evaluation_id)?.clone();
            if !matches!(
                evaluation.status,
                CadValidationEvaluationStatus::Queued | CadValidationEvaluationStatus::Running
            ) {
                return Err(format!(
                    "Terminal validation evaluation cannot fail again: {evaluation_id}"
                ));
            }
            evaluation.status = CadValidationEvaluationStatus::Failed;
            evaluation.report = None;
            evaluation.score = None;
            evaluation.passed = None;
            evaluation.error = Some(error);
            evaluation.completed_at = Some(timestamp());
            evaluation
        };
        self.update_validation_evaluation(evaluation)
    }

    pub fn save_validation_evaluation_event(
        &self,
        event: CadValidationEvaluationEvent,
    ) -> Result<CadValidationEvaluationEvent, String> {
        validate_validation_evaluation_event_fields(&event)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, &event.session_id)?;
        let evaluation =
            require_validation_evaluation(&state, &event.session_id, &event.evaluation_id)?;
        if evaluation.evaluator_thread_id.as_deref() != Some(event.evaluator_thread_id.as_str()) {
            return Err(format!(
                "Validation evaluation event thread does not match evaluation {}.",
                event.evaluation_id
            ));
        }
        if let Some(turn_id) = &event.external_turn_id {
            if evaluation.external_turn_id.as_deref() != Some(turn_id) {
                return Err(format!(
                    "Validation evaluation event turn does not match evaluation {}.",
                    event.evaluation_id
                ));
            }
        }
        let events = state
            .validation_evaluation_events
            .entry(event.session_id.clone())
            .or_default();
        if let Some(existing) = events.iter().find(|candidate| candidate.id == event.id) {
            if existing != &event {
                return Err(format!(
                    "Validation evaluation event id was replayed with different content: {}",
                    event.id
                ));
            }
            return Ok(existing.clone());
        }
        if events.iter().any(|candidate| {
            candidate.evaluation_id == event.evaluation_id && candidate.sequence == event.sequence
        }) {
            return Err(format!(
                "Validation evaluation event sequence already exists: {}:{}",
                event.evaluation_id, event.sequence
            ));
        }
        let saved = self.repository.save_validation_evaluation_event(&event)?;
        events.push(saved.clone());
        events.sort_by(|left, right| {
            left.evaluation_id
                .cmp(&right.evaluation_id)
                .then_with(|| left.sequence.cmp(&right.sequence))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(saved)
    }

    pub fn list_validation_evaluation_events(
        &self,
        session_id: &str,
        evaluation_id: &str,
    ) -> Result<Vec<CadValidationEvaluationEvent>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_validation_evaluation(&state, session_id, evaluation_id)?;
        Ok(state
            .validation_evaluation_events
            .get(session_id)
            .into_iter()
            .flatten()
            .filter(|event| event.evaluation_id == evaluation_id)
            .cloned()
            .collect())
    }
}

fn require_validation_evaluation<'a>(
    state: &'a ServiceState,
    session_id: &str,
    evaluation_id: &str,
) -> Result<&'a CadValidationEvaluation, String> {
    require_session(state, session_id)?;
    state
        .validation_evaluations
        .get(session_id)
        .into_iter()
        .flatten()
        .find(|evaluation| evaluation.id == evaluation_id)
        .ok_or_else(|| format!("Validation evaluation not found: {evaluation_id}"))
}

fn validate_validation_evaluation_graph(
    state: &ServiceState,
    evaluation: &CadValidationEvaluation,
) -> Result<(), String> {
    require_session(state, &evaluation.session_id)?;
    let _run = state
        .agent_runs
        .get(&evaluation.session_id)
        .into_iter()
        .flatten()
        .find(|run| run.id == evaluation.run_id)
        .ok_or_else(|| format!("Agent run not found: {}", evaluation.run_id))?;
    let revision = state
        .revisions
        .get(&evaluation.revision_id)
        .filter(|revision| revision.session_id == evaluation.session_id)
        .ok_or_else(|| {
            format!(
                "Validation evaluation revision graph mismatch: {}",
                evaluation.revision_id
            )
        })?;
    let artifact = state
        .artifacts
        .get(&evaluation.artifact_id)
        .filter(|artifact| artifact.revision_id == revision.id && artifact.deleted_at.is_none())
        .ok_or_else(|| {
            format!(
                "Validation evaluation artifact graph mismatch: {}",
                evaluation.artifact_id
            )
        })?;
    if artifact.missing_at.is_some() {
        return Err(format!(
            "Validation evaluation artifact is missing: {}",
            artifact.id
        ));
    }
    if let Some(thread_id) = &evaluation.evaluator_thread_id {
        let thread = state
            .agent_threads
            .get(&evaluation.session_id)
            .into_iter()
            .flatten()
            .find(|thread| thread.id == *thread_id)
            .ok_or_else(|| format!("Agent thread not found: {thread_id}"))?;
        if thread.plane != CadAgentPlane::Validation || thread.owner_id != evaluation.id {
            return Err(format!(
                "Validation evaluation thread scope mismatch: {thread_id}"
            ));
        }
    }
    Ok(())
}

fn validate_validation_evaluation_current_output(
    state: &ServiceState,
    evaluation: &CadValidationEvaluation,
) -> Result<(), String> {
    let run = state
        .agent_runs
        .get(&evaluation.session_id)
        .into_iter()
        .flatten()
        .find(|run| run.id == evaluation.run_id)
        .ok_or_else(|| format!("Agent run not found: {}", evaluation.run_id))?;
    if run.output_revision_id.as_deref() != Some(evaluation.revision_id.as_str()) {
        return Err(format!(
            "Validation evaluation revision does not match run {} output revision.",
            evaluation.run_id
        ));
    }
    Ok(())
}

fn validate_validation_evaluation_fields(
    evaluation: &CadValidationEvaluation,
) -> Result<(), String> {
    if [
        evaluation.id.as_str(),
        evaluation.session_id.as_str(),
        evaluation.run_id.as_str(),
        evaluation.revision_id.as_str(),
        evaluation.artifact_id.as_str(),
        evaluation.created_at.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err("Validation evaluation identifiers and created_at cannot be empty.".into());
    }
    if evaluation.attempt == 0 {
        return Err("Validation evaluation attempt must be at least 1.".into());
    }
    if !evaluation.input_contract.is_object() {
        return Err("Validation evaluation input_contract must be a JSON object.".into());
    }
    if !evaluation.pass_threshold.is_finite() || !(0.0..=1.0).contains(&evaluation.pass_threshold) {
        return Err(
            "Validation evaluation pass_threshold must be finite and between 0 and 1.".into(),
        );
    }
    match evaluation.status {
        CadValidationEvaluationStatus::Queued => {
            if evaluation.evaluator_thread_id.is_some()
                || evaluation.external_turn_id.is_some()
                || evaluation.started_at.is_some()
                || evaluation.completed_at.is_some()
                || evaluation.report.is_some()
                || evaluation.passed.is_some()
                || evaluation.score.is_some()
                || evaluation.error.is_some()
            {
                return Err("Queued validation evaluation contains non-queued fields.".into());
            }
        }
        CadValidationEvaluationStatus::Running => {
            if evaluation
                .evaluator_thread_id
                .as_deref()
                .is_none_or(str::is_empty)
                || evaluation
                    .external_turn_id
                    .as_deref()
                    .is_none_or(str::is_empty)
                || evaluation.started_at.is_none()
                || evaluation.completed_at.is_some()
                || evaluation.report.is_some()
                || evaluation.passed.is_some()
                || evaluation.score.is_some()
                || evaluation.error.is_some()
            {
                return Err("Running validation evaluation fields are inconsistent.".into());
            }
        }
        CadValidationEvaluationStatus::Succeeded => {
            let score = evaluation
                .score
                .filter(|score| score.is_finite() && (0.0..=1.0).contains(score))
                .ok_or_else(|| {
                    "Succeeded validation evaluation requires a score between 0 and 1.".to_string()
                })?;
            if evaluation
                .evaluator_thread_id
                .as_deref()
                .is_none_or(str::is_empty)
                || evaluation
                    .external_turn_id
                    .as_deref()
                    .is_none_or(str::is_empty)
                || evaluation.started_at.is_none()
                || evaluation.completed_at.is_none()
                || !evaluation.report.as_ref().is_some_and(Value::is_object)
                || evaluation.passed.is_none()
                || (evaluation.passed == Some(true) && score < evaluation.pass_threshold)
                || evaluation.error.is_some()
            {
                return Err("Succeeded validation evaluation fields are inconsistent.".into());
            }
        }
        CadValidationEvaluationStatus::Failed => {
            if evaluation.completed_at.is_none()
                || evaluation
                    .error
                    .as_deref()
                    .is_none_or(|error| error.trim().is_empty())
                || evaluation.report.is_some()
                || evaluation.passed.is_some()
                || evaluation.score.is_some()
                || (evaluation.external_turn_id.is_some()
                    && evaluation.evaluator_thread_id.is_none())
            {
                return Err("Failed validation evaluation fields are inconsistent.".into());
            }
        }
    }
    Ok(())
}

fn validate_evaluation_immutable_fields(
    persisted: &CadValidationEvaluation,
    candidate: &CadValidationEvaluation,
) -> Result<(), String> {
    if persisted.session_id != candidate.session_id
        || persisted.run_id != candidate.run_id
        || persisted.revision_id != candidate.revision_id
        || persisted.artifact_id != candidate.artifact_id
        || persisted.kind != candidate.kind
        || persisted.attempt != candidate.attempt
        || persisted.input_contract != candidate.input_contract
        || persisted.pass_threshold != candidate.pass_threshold
        || persisted.created_at != candidate.created_at
    {
        return Err(format!(
            "Validation evaluation attempt fields are immutable: {}",
            candidate.id
        ));
    }
    Ok(())
}

fn validate_evaluation_status_transition(
    from: &CadValidationEvaluationStatus,
    to: &CadValidationEvaluationStatus,
) -> Result<(), String> {
    let valid = matches!(
        (from, to),
        (
            CadValidationEvaluationStatus::Queued,
            CadValidationEvaluationStatus::Running
        ) | (
            CadValidationEvaluationStatus::Queued,
            CadValidationEvaluationStatus::Failed
        ) | (
            CadValidationEvaluationStatus::Running,
            CadValidationEvaluationStatus::Succeeded
        ) | (
            CadValidationEvaluationStatus::Running,
            CadValidationEvaluationStatus::Failed
        )
    );
    if !valid {
        return Err(format!(
            "Invalid validation evaluation status transition: {from:?} -> {to:?}"
        ));
    }
    Ok(())
}

fn validate_validation_evaluation_event_fields(
    event: &CadValidationEvaluationEvent,
) -> Result<(), String> {
    if [
        event.id.as_str(),
        event.session_id.as_str(),
        event.evaluation_id.as_str(),
        event.evaluator_thread_id.as_str(),
        event.method.as_str(),
        event.created_at.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(
            "Validation evaluation event identifiers, method, and created_at cannot be empty."
                .into(),
        );
    }
    Ok(())
}

fn sort_validation_evaluations(evaluations: &mut [CadValidationEvaluation]) {
    evaluations.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn validate_saved_evaluation_contract_identity(
    evaluation: &CadValidationEvaluation,
) -> Result<(), String> {
    let object = evaluation.input_contract.as_object().ok_or_else(|| {
        "Persisted validation evaluation input_contract must be a JSON object.".to_string()
    })?;
    if object.get("evaluationId").and_then(Value::as_str) != Some(evaluation.id.as_str()) {
        return Err(format!(
            "Persisted validation evaluation contract id mismatch: {}",
            evaluation.id
        ));
    }
    if object.get("attempt").and_then(Value::as_u64) != Some(u64::from(evaluation.attempt)) {
        return Err(format!(
            "Persisted validation evaluation contract attempt mismatch: {}",
            evaluation.id
        ));
    }
    Ok(())
}
