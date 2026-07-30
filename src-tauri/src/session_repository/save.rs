use super::support::*;
use super::*;

pub(super) fn save_artifact_manifest(
    connection: &Connection,
    session_id: &str,
    artifact: &CadArtifact,
) -> SessionRepositoryResult<()> {
    let metadata = artifact.metadata.clone().unwrap_or_default();
    let relative_path = metadata
        .get("relativePath")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Artifact manifest {} is missing relativePath.", artifact.id))?;
    let sha256 = metadata
        .get("sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Artifact manifest {} is missing sha256.", artifact.id))?;
    connection
        .execute(
            r#"
            INSERT INTO artifacts (
              id, session_id, revision_id, kind, format, relative_path, uri,
              sha256, bytes, created_at, deleted_at, missing_at, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
              session_id = excluded.session_id,
              revision_id = excluded.revision_id,
              kind = excluded.kind,
              format = excluded.format,
              relative_path = excluded.relative_path,
              uri = excluded.uri,
              sha256 = excluded.sha256,
              bytes = excluded.bytes,
              created_at = excluded.created_at,
              deleted_at = excluded.deleted_at,
              missing_at = excluded.missing_at,
              metadata_json = excluded.metadata_json
            "#,
            params![
                artifact.id,
                session_id,
                artifact.revision_id,
                to_db_text(&artifact.kind)?,
                artifact.format,
                relative_path,
                artifact.uri,
                sha256,
                artifact.bytes.unwrap_or_default() as i64,
                artifact.created_at,
                artifact.deleted_at,
                artifact.missing_at,
                serde_json::to_string(&metadata).map_err(|error| error.to_string())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_conversation_message(
    connection: &Connection,
    message: &CadConversationMessage,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO conversation_messages (
              id, session_id, revision_id, run_id, role, content, created_at, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                message.id,
                message.session_id,
                message.revision_id,
                message.run_id,
                to_db_text(&message.role)?,
                message.content,
                message.created_at,
                optional_metadata_json(message.metadata.as_ref())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_agent_run(
    connection: &Connection,
    run: &CadAgentRun,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO agent_runs (
              id, session_id, input_revision_id, output_revision_id, status, prompt,
              created_at, updated_at, started_at, completed_at, error, active_step,
              external_agent, external_thread_id, external_turn_id, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL)
            ON CONFLICT(id) DO UPDATE SET
              session_id = excluded.session_id,
              input_revision_id = excluded.input_revision_id,
              output_revision_id = excluded.output_revision_id,
              status = excluded.status,
              prompt = excluded.prompt,
              created_at = excluded.created_at,
              updated_at = excluded.updated_at,
              started_at = excluded.started_at,
              completed_at = excluded.completed_at,
              error = excluded.error,
              active_step = excluded.active_step,
              external_agent = excluded.external_agent,
              external_thread_id = excluded.external_thread_id,
              external_turn_id = excluded.external_turn_id
            "#,
            params![
                run.id,
                run.session_id,
                run.input_revision_id,
                run.output_revision_id,
                to_db_text(&run.status)?,
                run.prompt,
                run.created_at,
                run.updated_at,
                run.started_at,
                run.completed_at,
                run.error,
                run.active_step,
                run.external_agent,
                run.external_thread_id,
                run.external_turn_id,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_agent_run_event(
    connection: &mut Connection,
    event: &CadAgentRunEvent,
) -> SessionRepositoryResult<CadAgentRunEvent> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if let Some(sequence) = transaction
        .query_row(
            "SELECT sequence FROM agent_run_events WHERE id = ?1",
            params![event.id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    {
        let mut saved = event.clone();
        saved.sequence = sequence.max(0) as u64;
        transaction.commit().map_err(|error| error.to_string())?;
        return Ok(saved);
    }
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM agent_run_events WHERE run_id = ?1",
            params![event.run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    let mut saved = event.clone();
    saved.sequence = sequence.max(1) as u64;
    transaction
        .execute(
            r#"
            INSERT INTO agent_run_events (
              id, session_id, run_id, revision_id, event_type, sequence,
              created_at, payload_json, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                saved.id,
                saved.session_id,
                saved.run_id,
                saved.revision_id,
                to_db_text(&saved.event_type)?,
                saved.sequence as i64,
                saved.created_at,
                serde_json::to_string(&saved.payload).map_err(|error| error.to_string())?,
                optional_metadata_json(saved.metadata.as_ref())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

pub(super) fn save_workflow_plan(
    connection: &Connection,
    plan: &CadWorkflowPlan,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_plans (
              run_id, revision_id, plan_json, source_language, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(run_id) DO UPDATE SET
              revision_id = excluded.revision_id,
              plan_json = excluded.plan_json,
              source_language = excluded.source_language,
              created_at = excluded.created_at
            "#,
            params![
                plan.run_id,
                plan.revision_id,
                serde_json::to_string(&plan.plan).map_err(|error| error.to_string())?,
                to_db_text(&plan.source_language)?,
                plan.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_workflow_outer_iteration(
    connection: &Connection,
    iteration: &CadWorkflowOuterIteration,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_outer_iterations (
              id, run_id, iteration, revision_id, structural_report_json,
              vlm_report_json, failure_report_json, passed, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
              run_id = excluded.run_id,
              iteration = excluded.iteration,
              revision_id = excluded.revision_id,
              structural_report_json = excluded.structural_report_json,
              vlm_report_json = excluded.vlm_report_json,
              failure_report_json = excluded.failure_report_json,
              passed = excluded.passed,
              created_at = excluded.created_at
            "#,
            params![
                iteration.id,
                iteration.run_id,
                iteration.iteration as i64,
                iteration.revision_id,
                serde_json::to_string(&iteration.structural_report)
                    .map_err(|error| error.to_string())?,
                optional_json_value_text(iteration.vlm_report.as_ref())?,
                optional_json_value_text(iteration.failure_report.as_ref())?,
                i64::from(iteration.passed),
                iteration.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_workflow_pending_vlm(
    connection: &Connection,
    pending_vlm: &CadWorkflowPendingVlm,
) -> SessionRepositoryResult<()> {
    connection
        .execute(
            r#"
            INSERT INTO workflow_pending_vlm (
              run_id, artifact_id, contract_json, pass_threshold, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(run_id) DO UPDATE SET
              artifact_id = excluded.artifact_id,
              contract_json = excluded.contract_json,
              pass_threshold = excluded.pass_threshold,
              created_at = excluded.created_at
            "#,
            params![
                pending_vlm.run_id,
                pending_vlm.artifact_id,
                serde_json::to_string(&pending_vlm.contract).map_err(|error| error.to_string())?,
                pending_vlm.pass_threshold,
                pending_vlm.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
