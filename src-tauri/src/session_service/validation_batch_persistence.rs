use super::*;

impl SessionService {
    pub fn create_validation_batch(
        &self,
        input: CadValidationBatchCreate,
    ) -> Result<(CadValidationBatch, Vec<CadValidationCheck>), String> {
        validate_batch_create(&input)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        validate_batch_graph(&state, &input)?;
        let created_at = timestamp();
        let batch = CadValidationBatch {
            id: uuid(),
            session_id: input.session_id.clone(),
            run_id: input.run_id,
            revision_id: input.revision_id,
            artifact_id: input.artifact_id,
            attempt: 1,
            status: CadValidationBatchStatus::Queued,
            aggregate_report: None,
            created_at: created_at.clone(),
            started_at: None,
            settlement_claimed_at: None,
            settled_at: None,
            effects_claimed_at: None,
            refinement_requested_at: None,
            refinement_bound_at: None,
            effects_applied_at: None,
        };
        let checks = input
            .checks
            .into_iter()
            .map(|input| CadValidationCheck {
                id: uuid(),
                batch_id: batch.id.clone(),
                session_id: batch.session_id.clone(),
                kind: input.kind,
                status: CadValidationCheckStatus::Queued,
                input_contract: input.input_contract,
                report: None,
                passed: None,
                error: None,
                evaluator_thread_id: None,
                external_turn_id: None,
                created_at: created_at.clone(),
                started_at: None,
                completed_at: None,
            })
            .collect::<Vec<_>>();
        let (saved_batch, mut saved_checks) =
            self.repository.create_validation_batch(&batch, &checks)?;
        if saved_checks.len() != 3 {
            return Err(format!(
                "Repository returned {} checks for validation batch {}.",
                saved_checks.len(),
                saved_batch.id
            ));
        }
        for check in &saved_checks {
            let object = check.input_contract.as_object().ok_or_else(|| {
                format!(
                    "Persisted validation check contract is not an object: {}",
                    check.id
                )
            })?;
            if object.get("batchId").and_then(Value::as_str) != Some(saved_batch.id.as_str())
                || object.get("checkId").and_then(Value::as_str) != Some(check.id.as_str())
                || object.get("attempt").and_then(Value::as_u64)
                    != Some(u64::from(saved_batch.attempt))
            {
                return Err(format!(
                    "Persisted validation check contract identity mismatch: {}",
                    check.id
                ));
            }
        }
        saved_checks.sort_by_key(|check| check_kind_order(&check.kind));
        state
            .validation_batches
            .entry(saved_batch.session_id.clone())
            .or_default()
            .push(saved_batch.clone());
        state
            .validation_checks
            .entry(saved_batch.session_id.clone())
            .or_default()
            .extend(saved_checks.clone());
        Ok((saved_batch, saved_checks))
    }

    pub fn list_validation_batches(
        &self,
        session_id: &str,
    ) -> Result<Vec<CadValidationBatch>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(state
            .validation_batches
            .get(session_id)
            .cloned()
            .unwrap_or_default())
    }

    pub fn get_validation_batch(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<Option<CadValidationBatch>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(find_batch(&state, session_id, batch_id).cloned())
    }

    pub fn list_validation_checks(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<Vec<CadValidationCheck>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let mut checks = state
            .validation_checks
            .get(session_id)
            .into_iter()
            .flatten()
            .filter(|check| check.batch_id == batch_id)
            .cloned()
            .collect::<Vec<_>>();
        checks.sort_by_key(|check| check_kind_order(&check.kind));
        Ok(checks)
    }

    pub fn get_validation_check(
        &self,
        session_id: &str,
        check_id: &str,
    ) -> Result<Option<CadValidationCheck>, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        require_session(&state, session_id)?;
        Ok(find_check(&state, session_id, check_id).cloned())
    }

    pub fn start_validation_check(
        &self,
        session_id: &str,
        check_id: &str,
    ) -> Result<CadValidationCheck, String> {
        self.start_validation_check_inner(session_id, check_id, None, None)
    }

    pub fn bind_validation_check(
        &self,
        session_id: &str,
        check_id: &str,
        evaluator_thread_id: &str,
        external_turn_id: &str,
    ) -> Result<CadValidationCheck, String> {
        if evaluator_thread_id.trim().is_empty() || external_turn_id.trim().is_empty() {
            return Err("Validation check thread and turn identifiers cannot be empty.".into());
        }
        self.start_validation_check_inner(
            session_id,
            check_id,
            Some(evaluator_thread_id.to_string()),
            Some(external_turn_id.to_string()),
        )
    }

    fn start_validation_check_inner(
        &self,
        session_id: &str,
        check_id: &str,
        evaluator_thread_id: Option<String>,
        external_turn_id: Option<String>,
    ) -> Result<CadValidationCheck, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let persisted = require_check(&state, session_id, check_id)?.clone();
        if persisted.status != CadValidationCheckStatus::Queued {
            return Err(format!(
                "Only a queued validation check can start: {check_id}"
            ));
        }
        match persisted.kind {
            CadValidationCheckKind::Vlm => {
                let thread_id = evaluator_thread_id.as_deref().ok_or_else(|| {
                    "VLM validation check must be bound to an evaluator thread.".to_string()
                })?;
                let thread = state
                    .agent_threads
                    .get(session_id)
                    .into_iter()
                    .flatten()
                    .find(|thread| thread.id == thread_id)
                    .ok_or_else(|| format!("Agent thread not found: {thread_id}"))?;
                if thread.plane != CadAgentPlane::Validation
                    || thread.owner_id != check_id
                    || thread.archived_at.is_some()
                    || thread.replaced_by_id.is_some()
                {
                    return Err(format!(
                        "Validation evaluator thread does not actively own check {check_id}."
                    ));
                }
            }
            CadValidationCheckKind::Structural | CadValidationCheckKind::Dfm => {
                if evaluator_thread_id.is_some() || external_turn_id.is_some() {
                    return Err("Only VLM checks may bind an evaluator thread.".to_string());
                }
            }
        }
        let mut check = persisted.clone();
        check.status = CadValidationCheckStatus::Running;
        check.evaluator_thread_id = evaluator_thread_id;
        check.external_turn_id = external_turn_id;
        check.started_at = Some(timestamp());
        let saved = self
            .repository
            .update_validation_check(&check, &persisted.status)?;
        replace_check(&mut state, saved.clone())?;
        if let Some(batch) = find_batch_mut(&mut state, session_id, &saved.batch_id) {
            if batch.status == CadValidationBatchStatus::Queued {
                batch.status = CadValidationBatchStatus::Running;
                batch.started_at = saved.started_at.clone();
            }
        }
        Ok(saved)
    }

    pub fn complete_validation_check(
        &self,
        session_id: &str,
        check_id: &str,
        report: Value,
        passed: bool,
    ) -> Result<CadValidationCheck, String> {
        if !report.is_object() {
            return Err("Validation check report must be a JSON object.".to_string());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        let persisted = require_check(&state, session_id, check_id)?.clone();
        if persisted.status != CadValidationCheckStatus::Running {
            return Err(format!(
                "Only a running validation check can complete: {check_id}"
            ));
        }
        let mut check = persisted.clone();
        check.status = CadValidationCheckStatus::Succeeded;
        check.report = Some(report);
        check.passed = Some(passed);
        check.completed_at = Some(timestamp());
        let saved = self
            .repository
            .update_validation_check(&check, &persisted.status)?;
        replace_check(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn fail_validation_check(
        &self,
        session_id: &str,
        check_id: &str,
        error: String,
    ) -> Result<CadValidationCheck, String> {
        if error.trim().is_empty() {
            return Err("Validation check failure error cannot be empty.".to_string());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        let persisted = require_check(&state, session_id, check_id)?.clone();
        if !matches!(
            persisted.status,
            CadValidationCheckStatus::Queued | CadValidationCheckStatus::Running
        ) {
            return Err(format!(
                "Terminal validation check cannot fail again: {check_id}"
            ));
        }
        let mut check = persisted.clone();
        check.status = CadValidationCheckStatus::Failed;
        check.report = None;
        check.passed = None;
        check.error = Some(error);
        check.completed_at = Some(timestamp());
        let saved = self
            .repository
            .update_validation_check(&check, &persisted.status)?;
        replace_check(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn reset_validation_check_for_recovery(
        &self,
        session_id: &str,
        check_id: &str,
    ) -> Result<CadValidationCheck, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let persisted = require_check(&state, session_id, check_id)?.clone();
        if persisted.status != CadValidationCheckStatus::Running
            || persisted.kind == CadValidationCheckKind::Vlm
        {
            return Err(format!(
                "Only a running structural or DFM check can be reset: {check_id}"
            ));
        }
        let mut check = persisted.clone();
        check.status = CadValidationCheckStatus::Queued;
        check.started_at = None;
        let saved = self
            .repository
            .update_validation_check(&check, &persisted.status)?;
        replace_check(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn try_claim_validation_batch_settlement(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<Option<CadValidationBatch>, String> {
        let claimed_at = timestamp();
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let claimed = self.repository.try_claim_validation_batch_settlement(
            session_id,
            batch_id,
            &claimed_at,
        )?;
        if let Some(batch) = &claimed {
            replace_batch(&mut state, batch.clone())?;
        }
        Ok(claimed)
    }

    pub fn settle_validation_batch(
        &self,
        session_id: &str,
        batch_id: &str,
        claim_token: &str,
        status: CadValidationBatchStatus,
        aggregate_report: Option<Value>,
    ) -> Result<CadValidationBatch, String> {
        match status {
            CadValidationBatchStatus::Succeeded => {
                if !aggregate_report.as_ref().is_some_and(Value::is_object) {
                    return Err(
                        "Succeeded validation batch requires an aggregate report object."
                            .to_string(),
                    );
                }
            }
            CadValidationBatchStatus::Failed => {
                if aggregate_report.is_some() {
                    return Err(
                        "Failed validation batch cannot contain an aggregate report.".to_string(),
                    );
                }
            }
            _ => return Err("Validation batch can settle only to succeeded or failed.".to_string()),
        }
        if claim_token.trim().is_empty() {
            return Err("Validation batch settlement claim token cannot be empty.".to_string());
        }
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let saved = self.repository.settle_validation_batch(
            session_id,
            batch_id,
            claim_token,
            &status,
            aggregate_report.as_ref(),
            &timestamp(),
        )?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn release_validation_batch_settlement(
        &self,
        session_id: &str,
        batch_id: &str,
        claim_token: &str,
    ) -> Result<CadValidationBatch, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let saved = self.repository.release_validation_batch_settlement(
            session_id,
            batch_id,
            claim_token,
        )?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn mark_validation_batch_effects_applied(
        &self,
        session_id: &str,
        batch_id: &str,
        claim_token: &str,
    ) -> Result<CadValidationBatch, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let saved = self.repository.mark_validation_batch_effects_applied(
            session_id,
            batch_id,
            claim_token,
            &timestamp(),
        )?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn try_claim_validation_batch_effects(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<Option<CadValidationBatch>, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        require_batch(&state, session_id, batch_id)?;
        let claimed =
            self.repository
                .try_claim_validation_batch_effects(session_id, batch_id, &uuid())?;
        if let Some(batch) = &claimed {
            replace_batch(&mut state, batch.clone())?;
        }
        Ok(claimed)
    }

    pub fn release_validation_batch_effects(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<CadValidationBatch, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let current = require_batch(&state, session_id, batch_id)?.clone();
        let token = current
            .effects_claimed_at
            .as_deref()
            .ok_or_else(|| format!("Validation batch effects are not owned: {batch_id}"))?;
        let saved = self
            .repository
            .release_validation_batch_effects(session_id, batch_id, token)?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn request_validation_batch_refinement(
        &self,
        session_id: &str,
        batch_id: &str,
        claim_token: &str,
    ) -> Result<CadValidationBatch, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let current = require_batch(&state, session_id, batch_id)?.clone();
        if current.refinement_requested_at.is_some() {
            return Ok(current);
        }
        let saved = self.repository.request_validation_batch_refinement(
            session_id,
            batch_id,
            claim_token,
            &timestamp(),
        )?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn bind_validation_batch_refinement(
        &self,
        session_id: &str,
        batch_id: &str,
        claim_token: &str,
    ) -> Result<CadValidationBatch, String> {
        let mut state = self.inner.lock().map_err(lock_error)?;
        let current = require_batch(&state, session_id, batch_id)?.clone();
        if current.refinement_bound_at.is_some() && current.effects_applied_at.is_some() {
            return Ok(current);
        }
        let run = state
            .agent_runs
            .get(session_id)
            .into_iter()
            .flatten()
            .find(|run| run.id == current.run_id)
            .ok_or_else(|| format!("Agent run not found: {}", current.run_id))?;
        if run.external_turn_id.is_none() {
            return Err(format!(
                "Validation batch refinement has no durable external turn binding: {batch_id}"
            ));
        }
        let saved = self.repository.bind_validation_batch_refinement(
            session_id,
            batch_id,
            claim_token,
            &timestamp(),
        )?;
        replace_batch(&mut state, saved.clone())?;
        Ok(saved)
    }

    pub fn save_validation_check_event(
        &self,
        event: CadValidationCheckEvent,
    ) -> Result<CadValidationCheckEvent, String> {
        if event.method.trim().is_empty() {
            return Err("Validation check event method cannot be empty.".into());
        }
        let state = self.inner.lock().map_err(lock_error)?;
        let check = require_check(&state, &event.session_id, &event.check_id)?;
        if check.evaluator_thread_id.as_deref() != Some(event.evaluator_thread_id.as_str())
            || event.external_turn_id.as_deref() != check.external_turn_id.as_deref()
        {
            return Err("Validation check event binding mismatch.".into());
        }
        drop(state);
        self.repository.save_validation_check_event(&event)
    }

    pub fn is_latest_validation_batch(
        &self,
        session_id: &str,
        batch_id: &str,
    ) -> Result<bool, String> {
        self.repository
            .is_latest_validation_batch(session_id, batch_id)
    }
}

fn validate_batch_create(input: &CadValidationBatchCreate) -> Result<(), String> {
    if [
        input.session_id.as_str(),
        input.run_id.as_str(),
        input.revision_id.as_str(),
        input.artifact_id.as_str(),
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err("Validation batch graph identifiers cannot be empty.".to_string());
    }
    if input.checks.len() != 3 {
        return Err(format!(
            "Validation batch requires exactly three checks, received {}.",
            input.checks.len()
        ));
    }
    let kinds = input
        .checks
        .iter()
        .map(|check| check.kind.clone())
        .collect::<HashSet<_>>();
    let required = HashSet::from([
        CadValidationCheckKind::Structural,
        CadValidationCheckKind::Dfm,
        CadValidationCheckKind::Vlm,
    ]);
    if kinds != required {
        return Err("Validation batch requires one structural, DFM, and VLM check.".to_string());
    }
    if input
        .checks
        .iter()
        .any(|check| !check.input_contract.is_object())
    {
        return Err("Every validation check input contract must be a JSON object.".to_string());
    }
    Ok(())
}

fn validate_batch_graph(
    state: &ServiceState,
    input: &CadValidationBatchCreate,
) -> Result<(), String> {
    require_session(state, &input.session_id)?;
    let run = state
        .agent_runs
        .get(&input.session_id)
        .into_iter()
        .flatten()
        .find(|run| run.id == input.run_id)
        .ok_or_else(|| format!("Agent run not found: {}", input.run_id))?;
    if run.output_revision_id.as_deref() != Some(input.revision_id.as_str()) {
        return Err(format!(
            "Validation batch revision does not match run {} output revision.",
            input.run_id
        ));
    }
    let revision = state
        .revisions
        .get(&input.revision_id)
        .filter(|revision| revision.session_id == input.session_id)
        .ok_or_else(|| {
            format!(
                "Validation batch revision graph mismatch: {}",
                input.revision_id
            )
        })?;
    let _artifact = state
        .artifacts
        .get(&input.artifact_id)
        .filter(|artifact| {
            artifact.revision_id == revision.id
                && artifact.deleted_at.is_none()
                && artifact.missing_at.is_none()
        })
        .ok_or_else(|| {
            format!(
                "Validation batch artifact graph mismatch: {}",
                input.artifact_id
            )
        })?;
    Ok(())
}

fn check_kind_order(kind: &CadValidationCheckKind) -> u8 {
    match kind {
        CadValidationCheckKind::Structural => 0,
        CadValidationCheckKind::Dfm => 1,
        CadValidationCheckKind::Vlm => 2,
    }
}

fn find_batch<'a>(
    state: &'a ServiceState,
    session_id: &str,
    batch_id: &str,
) -> Option<&'a CadValidationBatch> {
    state
        .validation_batches
        .get(session_id)?
        .iter()
        .find(|batch| batch.id == batch_id)
}

fn find_batch_mut<'a>(
    state: &'a mut ServiceState,
    session_id: &str,
    batch_id: &str,
) -> Option<&'a mut CadValidationBatch> {
    state
        .validation_batches
        .get_mut(session_id)?
        .iter_mut()
        .find(|batch| batch.id == batch_id)
}

fn require_batch<'a>(
    state: &'a ServiceState,
    session_id: &str,
    batch_id: &str,
) -> Result<&'a CadValidationBatch, String> {
    require_session(state, session_id)?;
    find_batch(state, session_id, batch_id)
        .ok_or_else(|| format!("Validation batch not found: {batch_id}"))
}

fn find_check<'a>(
    state: &'a ServiceState,
    session_id: &str,
    check_id: &str,
) -> Option<&'a CadValidationCheck> {
    state
        .validation_checks
        .get(session_id)?
        .iter()
        .find(|check| check.id == check_id)
}

fn require_check<'a>(
    state: &'a ServiceState,
    session_id: &str,
    check_id: &str,
) -> Result<&'a CadValidationCheck, String> {
    require_session(state, session_id)?;
    find_check(state, session_id, check_id)
        .ok_or_else(|| format!("Validation check not found: {check_id}"))
}

fn replace_batch(state: &mut ServiceState, batch: CadValidationBatch) -> Result<(), String> {
    let slot = state
        .validation_batches
        .get_mut(&batch.session_id)
        .into_iter()
        .flatten()
        .find(|candidate| candidate.id == batch.id)
        .ok_or_else(|| format!("Validation batch state not found: {}", batch.id))?;
    *slot = batch;
    Ok(())
}

fn replace_check(state: &mut ServiceState, check: CadValidationCheck) -> Result<(), String> {
    let slot = state
        .validation_checks
        .get_mut(&check.session_id)
        .into_iter()
        .flatten()
        .find(|candidate| candidate.id == check.id)
        .ok_or_else(|| format!("Validation check state not found: {}", check.id))?;
    *slot = check;
    Ok(())
}
