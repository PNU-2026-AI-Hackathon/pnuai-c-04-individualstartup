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
    let revision_source: String = connection
        .query_row(
            "SELECT source FROM revisions WHERE id = ?1 AND session_id = ?2",
            params![artifact.revision_id, session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "Artifact manifest {} references a revision outside session {session_id}: {}",
                artifact.id, artifact.revision_id
            )
        })?;
    let expected_revision_hash = crate::storage::sha256_hex(revision_source.as_bytes());
    if artifact.revision_hash != expected_revision_hash {
        return Err(format!(
            "Artifact manifest {} revision_hash does not match revision {} source.",
            artifact.id, artifact.revision_id
        ));
    }
    if metadata.get("profileHash").and_then(Value::as_str) != artifact.profile_hash.as_deref() {
        return Err(format!(
            "Artifact manifest {} profileHash metadata does not match its profile_hash field.",
            artifact.id
        ));
    }
    connection
        .execute(
            r#"
            INSERT INTO artifacts (
              id, session_id, revision_id, kind, format, relative_path, uri,
              sha256, bytes, created_at, deleted_at, missing_at, metadata_json,
              revision_hash, profile_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
              metadata_json = excluded.metadata_json,
              revision_hash = excluded.revision_hash,
              profile_hash = excluded.profile_hash
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
                artifact.revision_hash,
                artifact.profile_hash,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn update_artifact_profile_hash(
    connection: &Connection,
    session_id: &str,
    artifact_id: &str,
    profile_hash: &str,
) -> SessionRepositoryResult<()> {
    let changed = connection
        .execute(
            r#"
            UPDATE artifacts
            SET profile_hash = ?1,
                metadata_json = json_set(COALESCE(metadata_json, '{}'), '$.profileHash', ?1)
            WHERE id = ?2 AND session_id = ?3 AND deleted_at IS NULL
            "#,
            params![profile_hash, artifact_id, session_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(format!(
            "Expected one active artifact for profile hash update, changed {changed}: {artifact_id}"
        ));
    }
    Ok(())
}

pub(super) fn save_conversation_message(
    connection: &Connection,
    message: &CadConversationMessage,
) -> SessionRepositoryResult<CadConversationMessage> {
    let changed_rows = connection
        .execute(
            r#"
            INSERT INTO conversation_messages (
              id, session_id, revision_id, run_id, role, content, created_at, metadata_json,
              external_thread_id, external_turn_id, external_item_id, phase, sequence, is_final
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(external_thread_id, external_turn_id, external_item_id)
              WHERE external_item_id IS NOT NULL
            DO UPDATE SET
              revision_id = excluded.revision_id,
              content = CASE
                WHEN excluded.is_final = 1 OR conversation_messages.is_final = 0
                  THEN excluded.content
                ELSE conversation_messages.content
              END,
              phase = COALESCE(excluded.phase, conversation_messages.phase),
              sequence = COALESCE(excluded.sequence, conversation_messages.sequence),
              is_final = MAX(conversation_messages.is_final, excluded.is_final),
              metadata_json = COALESCE(excluded.metadata_json, conversation_messages.metadata_json)
            WHERE conversation_messages.session_id = excluded.session_id
              AND conversation_messages.run_id IS excluded.run_id
              AND conversation_messages.role = excluded.role
            ON CONFLICT(id) DO UPDATE SET
              revision_id = excluded.revision_id,
              content = excluded.content,
              phase = excluded.phase,
              sequence = excluded.sequence,
              is_final = excluded.is_final,
              metadata_json = excluded.metadata_json,
              external_thread_id = excluded.external_thread_id,
              external_turn_id = excluded.external_turn_id,
              external_item_id = excluded.external_item_id
            WHERE conversation_messages.session_id = excluded.session_id
              AND conversation_messages.run_id IS excluded.run_id
              AND conversation_messages.role = excluded.role
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
                message.external_thread_id,
                message.external_turn_id,
                message.external_item_id,
                message.phase.as_ref().map(to_db_text).transpose()?,
                message.sequence.map(|sequence| sequence as i64),
                i64::from(message.is_final),
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed_rows != 1 {
        return Err(format!(
            "Conversation idempotency key belongs to a different message graph: {}",
            message.id
        ));
    }
    let (lookup_sql, lookup_params): (&str, Vec<&dyn rusqlite::ToSql>) =
        if message.external_item_id.is_some() {
            (
                r#"
                SELECT id, session_id, revision_id, run_id, role, content, created_at,
                       metadata_json, external_thread_id, external_turn_id,
                       external_item_id, phase, sequence, is_final
                FROM conversation_messages
                WHERE external_thread_id = ?1 AND external_turn_id = ?2
                  AND external_item_id = ?3
                "#,
                vec![
                    &message.external_thread_id,
                    &message.external_turn_id,
                    &message.external_item_id,
                ],
            )
        } else {
            (
                r#"
                SELECT id, session_id, revision_id, run_id, role, content, created_at,
                       metadata_json, external_thread_id, external_turn_id,
                       external_item_id, phase, sequence, is_final
                FROM conversation_messages WHERE id = ?1
                "#,
                vec![&message.id],
            )
        };
    connection
        .query_row(
            lookup_sql,
            lookup_params.as_slice(),
            conversation_message_from_row,
        )
        .map_err(|error| error.to_string())
}

fn conversation_message_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CadConversationMessage> {
    let role: String = row.get(4)?;
    let metadata_json: Option<String> = row.get(7)?;
    let phase: Option<String> = row.get(11)?;
    Ok(CadConversationMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        revision_id: row.get(2)?,
        run_id: row.get(3)?,
        role: from_db_text(&role).map_err(to_rusqlite_error)?,
        content: row.get(5)?,
        created_at: row.get(6)?,
        metadata: optional_metadata(metadata_json).map_err(to_rusqlite_error)?,
        external_thread_id: row.get(8)?,
        external_turn_id: row.get(9)?,
        external_item_id: row.get(10)?,
        phase: phase
            .map(|phase| from_db_text(&phase).map_err(to_rusqlite_error))
            .transpose()?,
        sequence: row
            .get::<_, Option<i64>>(12)?
            .map(|value| value.max(0) as u64),
        is_final: row.get::<_, i64>(13)? != 0,
    })
}

pub(super) fn save_agent_thread(
    connection: &Connection,
    thread: &CadAgentThread,
) -> SessionRepositoryResult<()> {
    let changed_rows = connection
        .execute(
            r#"
            INSERT INTO agent_threads (
              id, session_id, external_agent, external_thread_id, plane, owner_id, status,
              connection_generation, created_at, updated_at, last_resumed_at,
              archived_at, replaced_by_id, metadata_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(id) DO UPDATE SET
              status = excluded.status,
              connection_generation = excluded.connection_generation,
              updated_at = excluded.updated_at,
              last_resumed_at = excluded.last_resumed_at,
              archived_at = excluded.archived_at,
              replaced_by_id = excluded.replaced_by_id,
              metadata_json = excluded.metadata_json
            WHERE agent_threads.session_id = excluded.session_id
              AND agent_threads.external_agent = excluded.external_agent
              AND agent_threads.external_thread_id = excluded.external_thread_id
              AND agent_threads.plane = excluded.plane
              AND agent_threads.owner_id = excluded.owner_id
            "#,
            params![
                thread.id,
                thread.session_id,
                thread.external_agent,
                thread.external_thread_id,
                to_db_text(&thread.plane)?,
                thread.owner_id,
                to_db_text(&thread.status)?,
                thread.connection_generation.map(|value| value as i64),
                thread.created_at,
                thread.updated_at,
                thread.last_resumed_at,
                thread.archived_at,
                thread.replaced_by_id,
                optional_metadata_json(thread.metadata.as_ref())?,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed_rows != 1 {
        return Err(format!(
            "Agent thread id belongs to a different external thread graph: {}",
            thread.id
        ));
    }
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
              external_agent, external_thread_id, external_turn_id, metadata_json,
              agent_thread_id, connection_generation, recovery_status
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, ?16, ?17, ?18)
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
              external_turn_id = excluded.external_turn_id,
              agent_thread_id = excluded.agent_thread_id,
              connection_generation = excluded.connection_generation,
              recovery_status = excluded.recovery_status
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
                run.agent_thread_id,
                run.connection_generation.map(|value| value as i64),
                to_db_text(&run.recovery_status)?,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn save_agent_transport_event(
    connection: &Connection,
    event: &CadAgentTransportEvent,
) -> SessionRepositoryResult<CadAgentTransportEvent> {
    let changed_rows = connection
        .execute(
            r#"
            INSERT INTO agent_transport_events (
              id, session_id, run_id, agent_thread_id, external_turn_id,
              external_item_id, method, sequence, payload_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                event.id,
                event.session_id,
                event.run_id,
                event.agent_thread_id,
                event.external_turn_id,
                event.external_item_id,
                event.method,
                event.sequence as i64,
                serde_json::to_string(&event.payload).map_err(|error| error.to_string())?,
                event.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed_rows == 1 {
        return Ok(event.clone());
    }
    let saved = connection
        .query_row(
            r#"
            SELECT id, session_id, run_id, agent_thread_id, external_turn_id,
                   external_item_id, method, sequence, payload_json, created_at
            FROM agent_transport_events WHERE id = ?1
            "#,
            params![event.id],
            |row| {
                let payload_json: String = row.get(8)?;
                Ok(CadAgentTransportEvent {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    run_id: row.get(2)?,
                    agent_thread_id: row.get(3)?,
                    external_turn_id: row.get(4)?,
                    external_item_id: row.get(5)?,
                    method: row.get(6)?,
                    sequence: row.get::<_, i64>(7)?.max(0) as u64,
                    payload: serde_json::from_str(&payload_json)
                        .map_err(|error| to_rusqlite_error(error.to_string()))?,
                    created_at: row.get(9)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    if saved != *event {
        return Err(format!(
            "Transport event id was replayed with different content: {}",
            event.id
        ));
    }
    Ok(saved)
}

pub(super) fn create_validation_evaluation(
    connection: &Connection,
    evaluation: &CadValidationEvaluation,
) -> SessionRepositoryResult<CadValidationEvaluation> {
    let changed = connection
        .execute(
            r#"
            INSERT INTO validation_evaluations (
              id, session_id, run_id, revision_id, artifact_id, kind, attempt, status,
              evaluator_thread_id, external_turn_id, input_contract_json, report_json,
              passed, score, pass_threshold, error, created_at, started_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                      ?14, ?15, ?16, ?17, ?18, ?19)
            ON CONFLICT DO NOTHING
            "#,
            params![
                evaluation.id,
                evaluation.session_id,
                evaluation.run_id,
                evaluation.revision_id,
                evaluation.artifact_id,
                to_db_text(&evaluation.kind)?,
                i64::from(evaluation.attempt),
                to_db_text(&evaluation.status)?,
                evaluation.evaluator_thread_id,
                evaluation.external_turn_id,
                serde_json::to_string(&evaluation.input_contract)
                    .map_err(|error| error.to_string())?,
                optional_json_value_text(evaluation.report.as_ref())?,
                evaluation.passed.map(i64::from),
                evaluation.score,
                evaluation.pass_threshold,
                evaluation.error,
                evaluation.created_at,
                evaluation.started_at,
                evaluation.completed_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(format!(
            "Validation evaluation attempt already exists or id is already used: {} (attempt {})",
            evaluation.id, evaluation.attempt
        ));
    }
    Ok(evaluation.clone())
}

pub(super) fn create_next_validation_evaluation(
    connection: &mut Connection,
    evaluation: &CadValidationEvaluation,
) -> SessionRepositoryResult<CadValidationEvaluation> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let maximum_attempt: i64 = transaction
        .query_row(
            r#"
            SELECT COALESCE(MAX(attempt), 0)
            FROM validation_evaluations
            WHERE run_id = ?1 AND revision_id = ?2 AND artifact_id = ?3 AND kind = ?4
            "#,
            params![
                evaluation.run_id,
                evaluation.revision_id,
                evaluation.artifact_id,
                to_db_text(&evaluation.kind)?,
            ],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let next_attempt = maximum_attempt
        .checked_add(1)
        .ok_or_else(|| "Validation evaluation attempt overflowed i64.".to_string())?;
    let next_attempt = u32::try_from(next_attempt)
        .map_err(|_| "Validation evaluation attempt exceeds u32.".to_string())?;
    let mut evaluation = evaluation.clone();
    evaluation.attempt = next_attempt;
    set_validation_contract_identity(&mut evaluation.input_contract, &evaluation.id, next_attempt)?;
    let saved = create_validation_evaluation(&transaction, &evaluation)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

fn set_validation_contract_identity(
    input_contract: &mut Value,
    evaluation_id: &str,
    attempt: u32,
) -> SessionRepositoryResult<()> {
    let object = input_contract
        .as_object_mut()
        .ok_or_else(|| "Validation evaluation input_contract must be a JSON object.".to_string())?;
    if let Some(existing_id) = object.get("evaluationId") {
        if existing_id.as_str() != Some(evaluation_id) {
            return Err(format!(
                "Validation evaluation contract evaluationId does not match generated id: {evaluation_id}"
            ));
        }
    }
    if let Some(existing_attempt) = object.get("attempt") {
        if existing_attempt.as_u64() != Some(u64::from(attempt)) {
            return Err(format!(
                "Validation evaluation contract attempt does not match allocated attempt: {attempt}"
            ));
        }
    }
    object.insert(
        "evaluationId".to_string(),
        Value::String(evaluation_id.to_string()),
    );
    object.insert(
        "attempt".to_string(),
        Value::Number(serde_json::Number::from(attempt)),
    );
    Ok(())
}

pub(super) fn update_validation_evaluation(
    connection: &Connection,
    evaluation: &CadValidationEvaluation,
) -> SessionRepositoryResult<CadValidationEvaluation> {
    let persisted = load_validation_evaluation_by_id(connection, &evaluation.id)?
        .ok_or_else(|| format!("Validation evaluation not found: {}", evaluation.id))?;
    if persisted == *evaluation {
        return Ok(persisted);
    }
    if persisted.session_id != evaluation.session_id
        || persisted.run_id != evaluation.run_id
        || persisted.revision_id != evaluation.revision_id
        || persisted.artifact_id != evaluation.artifact_id
        || persisted.kind != evaluation.kind
        || persisted.attempt != evaluation.attempt
        || persisted.input_contract != evaluation.input_contract
        || persisted.pass_threshold != evaluation.pass_threshold
        || persisted.created_at != evaluation.created_at
    {
        return Err(format!(
            "Validation evaluation attempt fields are immutable: {}",
            evaluation.id
        ));
    }
    let changed = connection
        .execute(
            r#"
            UPDATE validation_evaluations
            SET status = ?1,
                evaluator_thread_id = ?2,
                external_turn_id = ?3,
                report_json = ?4,
                passed = ?5,
                score = ?6,
                error = ?7,
                started_at = ?8,
                completed_at = ?9
            WHERE id = ?10
            "#,
            params![
                to_db_text(&evaluation.status)?,
                evaluation.evaluator_thread_id,
                evaluation.external_turn_id,
                optional_json_value_text(evaluation.report.as_ref())?,
                evaluation.passed.map(i64::from),
                evaluation.score,
                evaluation.error,
                evaluation.started_at,
                evaluation.completed_at,
                evaluation.id,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err(format!(
            "Expected one validation evaluation update, changed {changed}: {}",
            evaluation.id
        ));
    }
    Ok(evaluation.clone())
}

fn load_validation_evaluation_by_id(
    connection: &Connection,
    evaluation_id: &str,
) -> SessionRepositoryResult<Option<CadValidationEvaluation>> {
    connection
        .query_row(
            r#"
            SELECT id, session_id, run_id, revision_id, artifact_id, kind, attempt, status,
                   evaluator_thread_id, external_turn_id, input_contract_json, report_json,
                   passed, score, pass_threshold, error, created_at, started_at, completed_at
            FROM validation_evaluations WHERE id = ?1
            "#,
            params![evaluation_id],
            validation_evaluation_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub(super) fn save_validation_evaluation_event(
    connection: &Connection,
    event: &CadValidationEvaluationEvent,
) -> SessionRepositoryResult<CadValidationEvaluationEvent> {
    let changed = connection
        .execute(
            r#"
            INSERT INTO validation_evaluation_events (
              id, session_id, evaluation_id, evaluator_thread_id, external_turn_id,
              external_item_id, method, sequence, payload_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![
                event.id,
                event.session_id,
                event.evaluation_id,
                event.evaluator_thread_id,
                event.external_turn_id,
                event.external_item_id,
                event.method,
                i64::try_from(event.sequence)
                    .map_err(|_| "Validation evaluation event sequence exceeds i64".to_string())?,
                serde_json::to_string(&event.payload).map_err(|error| error.to_string())?,
                event.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    if changed == 1 {
        return Ok(event.clone());
    }
    let persisted = connection
        .query_row(
            r#"
            SELECT id, session_id, evaluation_id, evaluator_thread_id, external_turn_id,
                   external_item_id, method, sequence, payload_json, created_at
            FROM validation_evaluation_events WHERE id = ?1
            "#,
            params![event.id],
            validation_evaluation_event_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "Validation evaluation event sequence already exists: {}:{}",
                event.evaluation_id, event.sequence
            )
        })?;
    if persisted != *event {
        return Err(format!(
            "Validation evaluation event id was replayed with different content: {}",
            event.id
        ));
    }
    Ok(persisted)
}

pub(super) fn save_agent_run_event(
    connection: &mut Connection,
    event: &CadAgentRunEvent,
) -> SessionRepositoryResult<CadAgentRunEvent> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let saved = save_agent_run_event_in_transaction(&transaction, event)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(saved)
}

pub(super) fn save_agent_run_event_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    event: &CadAgentRunEvent,
) -> SessionRepositoryResult<CadAgentRunEvent> {
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
              dfm_report_json, vlm_report_json, failure_report_json, passed, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ON CONFLICT(id) DO UPDATE SET
              run_id = excluded.run_id,
              iteration = excluded.iteration,
              revision_id = excluded.revision_id,
              structural_report_json = excluded.structural_report_json,
              dfm_report_json = excluded.dfm_report_json,
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
                optional_json_value_text(iteration.dfm_report.as_ref())?,
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
              run_id, artifact_id, revision_id, contract_json, pass_threshold,
              structural_report_json, dfm_report_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(run_id) DO UPDATE SET
              artifact_id = excluded.artifact_id,
              revision_id = excluded.revision_id,
              contract_json = excluded.contract_json,
              pass_threshold = excluded.pass_threshold,
              structural_report_json = excluded.structural_report_json,
              dfm_report_json = excluded.dfm_report_json,
              created_at = excluded.created_at
            "#,
            params![
                pending_vlm.run_id,
                pending_vlm.artifact_id,
                pending_vlm.revision_id,
                serde_json::to_string(&pending_vlm.contract).map_err(|error| error.to_string())?,
                pending_vlm.pass_threshold,
                pending_vlm
                    .structural_report
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| error.to_string())?,
                optional_json_value_text(pending_vlm.dfm_report.as_ref())?,
                pending_vlm.created_at,
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}
