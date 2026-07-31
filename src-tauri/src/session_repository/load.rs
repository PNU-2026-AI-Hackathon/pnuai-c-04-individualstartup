use super::support::*;
use super::*;

pub(super) fn load_sessions(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadSession>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT id, title, title_source, selected_runtime, status, active_revision_id,
                   created_at, updated_at, last_viewed_at, connected_ui_clients,
                   archived_at, deleted_at
            FROM sessions
            WHERE deleted_at IS NULL
            ORDER BY updated_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let title_source: String = row.get(2)?;
            let selected_runtime: String = row.get(3)?;
            let status: String = row.get(4)?;
            let (selected_runtime, runtime_diagnostic) = recover_runtime_kind(&selected_runtime);
            let title_source =
                recover_title_source(&title_source).unwrap_or(CadSessionTitleSource::System);
            Ok(CadSession {
                id: row.get(0)?,
                title: row.get(1)?,
                title_source,
                selected_runtime,
                status: from_db_text(&status).map_err(to_rusqlite_error)?,
                recovery_diagnostics: runtime_diagnostic.into_iter().collect(),
                active_revision_id: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                last_viewed_at: row.get(8)?,
                connected_ui_clients: row.get::<_, i64>(9)?.max(0) as u32,
                archived_at: row.get(10)?,
                deleted_at: row.get(11)?,
                revisions: Vec::new(),
            })
        })
        .map_err(|error| error.to_string())?;
    let mut sessions = HashMap::new();
    for row in rows {
        let session = row.map_err(|error| error.to_string())?;
        sessions.insert(session.id.clone(), session);
    }
    Ok(sessions)
}

pub(super) fn load_revisions(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadRevision>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT revisions.id, revisions.session_id, revisions.parent_revision_id,
                   revisions.restored_from_revision_id, revisions.source_language,
                   revisions.source, revisions.parameters_json, revisions.diagnostics_json,
                   revisions.created_at, revisions.metadata_json
            FROM revisions
            INNER JOIN sessions ON sessions.id = revisions.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY revisions.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let source: String = row.get(5)?;
            let source_language: String = row.get(4)?;
            let parameters_json: String = row.get(6)?;
            let diagnostics_json: String = row.get(7)?;
            let metadata_json: Option<String> = row.get(9)?;
            let (source_language, source_language_diagnostic) =
                recover_source_language(&source_language);
            let mut diagnostics = recover_diagnostics(&diagnostics_json);
            if let Some(diagnostic) = source_language_diagnostic {
                diagnostics.ok = false;
                diagnostics.items.push(diagnostic);
            }
            Ok(CadRevision {
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: revision_user_events(metadata_json).map_err(to_rusqlite_error)?,
                run_links: Vec::new(),
                id,
                session_id: row.get(1)?,
                parent_revision_id: row.get(2)?,
                restored_from_revision_id: row.get(3)?,
                source_hash: storage::sha256_hex(source.as_bytes()),
                source_language,
                source,
                parameters: serde_json::from_str(&parameters_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                diagnostics,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut revisions = HashMap::new();
    for row in rows {
        let revision = row.map_err(|error| error.to_string())?;
        revisions.insert(revision.id.clone(), revision);
    }
    Ok(revisions)
}

pub(super) fn load_artifacts(
    connection: &Connection,
    layout: &StorageLayout,
    artifact_id: Option<&str>,
) -> SessionRepositoryResult<HashMap<String, CadArtifact>> {
    let sql = if artifact_id.is_some() {
        r#"
        SELECT artifacts.id, artifacts.revision_id, artifacts.kind, artifacts.format,
               artifacts.relative_path, artifacts.uri, artifacts.sha256, artifacts.bytes,
               artifacts.created_at, artifacts.deleted_at, artifacts.missing_at,
               artifacts.metadata_json
        FROM artifacts
        INNER JOIN sessions ON sessions.id = artifacts.session_id
        WHERE sessions.deleted_at IS NULL AND artifacts.id = ?1
        "#
    } else {
        r#"
        SELECT artifacts.id, artifacts.revision_id, artifacts.kind, artifacts.format,
               artifacts.relative_path, artifacts.uri, artifacts.sha256, artifacts.bytes,
               artifacts.created_at, artifacts.deleted_at, artifacts.missing_at,
               artifacts.metadata_json
        FROM artifacts
        INNER JOIN sessions ON sessions.id = artifacts.session_id
        WHERE sessions.deleted_at IS NULL AND artifacts.deleted_at IS NULL
        ORDER BY artifacts.created_at ASC
        "#
    };
    let mut statement = connection.prepare(sql).map_err(|error| error.to_string())?;
    let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<CadArtifact> {
        let kind: String = row.get(2)?;
        let relative_path: String = row.get(4)?;
        let sha256: String = row.get(6)?;
        let metadata_json: Option<String> = row.get(11)?;
        let mut metadata = artifact_metadata(metadata_json).map_err(to_rusqlite_error)?;
        metadata.insert(
            "relativePath".to_string(),
            Value::String(relative_path.clone()),
        );
        metadata.insert("sha256".to_string(), Value::String(sha256));
        metadata.insert(
            "path".to_string(),
            Value::String(
                layout
                    .app_data_dir()
                    .join(&relative_path)
                    .to_string_lossy()
                    .to_string(),
            ),
        );
        Ok(CadArtifact {
            id: row.get(0)?,
            revision_id: row.get(1)?,
            kind: from_db_text(&kind).map_err(to_rusqlite_error)?,
            format: row.get(3)?,
            uri: row.get(5)?,
            bytes: Some(row.get::<_, i64>(7)?.max(0) as u64),
            created_at: row.get(8)?,
            deleted_at: row.get(9)?,
            missing_at: row.get(10)?,
            metadata: Some(metadata),
        })
    };
    let rows = if let Some(artifact_id) = artifact_id {
        statement
            .query_map(params![artifact_id], map_row)
            .map_err(|error| error.to_string())?
    } else {
        statement
            .query_map([], map_row)
            .map_err(|error| error.to_string())?
    };
    let mut artifacts = HashMap::new();
    for row in rows {
        let artifact = row.map_err(|error| error.to_string())?;
        artifacts.insert(artifact.id.clone(), artifact);
    }
    Ok(artifacts)
}

pub(super) fn load_conversation_messages(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadConversationMessage>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT conversation_messages.id, conversation_messages.session_id,
                   conversation_messages.revision_id, conversation_messages.run_id,
                   conversation_messages.role, conversation_messages.content,
                   conversation_messages.created_at, conversation_messages.metadata_json
            FROM conversation_messages
            INNER JOIN sessions ON sessions.id = conversation_messages.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY conversation_messages.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let role: String = row.get(4)?;
            let metadata_json: Option<String> = row.get(7)?;
            Ok(CadConversationMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                revision_id: row.get(2)?,
                run_id: row.get(3)?,
                role: from_db_text(&role).map_err(to_rusqlite_error)?,
                content: row.get(5)?,
                created_at: row.get(6)?,
                metadata: optional_metadata(metadata_json).map_err(to_rusqlite_error)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut messages: HashMap<String, Vec<CadConversationMessage>> = HashMap::new();
    for row in rows {
        let message = row.map_err(|error| error.to_string())?;
        messages
            .entry(message.session_id.clone())
            .or_default()
            .push(message);
    }
    Ok(messages)
}

pub(super) fn load_agent_runs(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadAgentRun>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT agent_runs.id, agent_runs.session_id, agent_runs.input_revision_id,
                   agent_runs.output_revision_id, agent_runs.status, agent_runs.prompt,
                   agent_runs.created_at, agent_runs.updated_at, agent_runs.started_at,
                   agent_runs.completed_at, agent_runs.error, agent_runs.active_step,
                   agent_runs.external_agent, agent_runs.external_thread_id,
                   agent_runs.external_turn_id
            FROM agent_runs
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY agent_runs.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let status: String = row.get(4)?;
            Ok(CadAgentRun {
                id: row.get(0)?,
                session_id: row.get(1)?,
                input_revision_id: row.get(2)?,
                output_revision_id: row.get(3)?,
                status: from_db_text(&status).map_err(to_rusqlite_error)?,
                prompt: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                started_at: row.get(8)?,
                completed_at: row.get(9)?,
                error: row.get(10)?,
                active_step: row.get(11)?,
                external_agent: row.get(12)?,
                external_thread_id: row.get(13)?,
                external_turn_id: row.get(14)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut runs: HashMap<String, Vec<CadAgentRun>> = HashMap::new();
    for row in rows {
        let run = row.map_err(|error| error.to_string())?;
        runs.entry(run.session_id.clone()).or_default().push(run);
    }
    Ok(runs)
}

pub(super) fn load_agent_run_events(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadAgentRunEvent>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT agent_run_events.id, agent_run_events.session_id,
                   agent_run_events.run_id, agent_run_events.revision_id,
                   agent_run_events.event_type, agent_run_events.sequence,
                   agent_run_events.created_at, agent_run_events.payload_json,
                   agent_run_events.metadata_json
            FROM agent_run_events
            INNER JOIN sessions ON sessions.id = agent_run_events.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY agent_run_events.run_id ASC, agent_run_events.sequence ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let event_type: String = row.get(4)?;
            let payload_json: String = row.get(7)?;
            let metadata_json: Option<String> = row.get(8)?;
            Ok(CadAgentRunEvent {
                id: row.get(0)?,
                session_id: row.get(1)?,
                run_id: row.get(2)?,
                revision_id: row.get(3)?,
                event_type: from_db_text(&event_type).map_err(to_rusqlite_error)?,
                sequence: row.get::<_, i64>(5)?.max(0) as u64,
                created_at: row.get(6)?,
                payload: serde_json::from_str(&payload_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                metadata: optional_metadata(metadata_json).map_err(to_rusqlite_error)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut events: HashMap<String, Vec<CadAgentRunEvent>> = HashMap::new();
    for row in rows {
        let event = row.map_err(|error| error.to_string())?;
        events
            .entry(event.session_id.clone())
            .or_default()
            .push(event);
    }
    Ok(events)
}

pub(super) fn load_workflow_plans(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadWorkflowPlan>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_plans.run_id, workflow_plans.revision_id,
                   workflow_plans.plan_json, workflow_plans.source_language,
                   workflow_plans.created_at
            FROM workflow_plans
            INNER JOIN agent_runs ON agent_runs.id = workflow_plans.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_plans.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let plan_json: String = row.get(2)?;
            let source_language: String = row.get(3)?;
            Ok(CadWorkflowPlan {
                run_id: row.get(0)?,
                revision_id: row.get(1)?,
                plan: serde_json::from_str(&plan_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                source_language: from_db_text(&source_language).map_err(to_rusqlite_error)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut plans = HashMap::new();
    for row in rows {
        let plan = row.map_err(|error| error.to_string())?;
        plans.insert(plan.run_id.clone(), plan);
    }
    Ok(plans)
}

pub(super) fn load_workflow_outer_iterations(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, Vec<CadWorkflowOuterIteration>>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_outer_iterations.id, workflow_outer_iterations.run_id,
                   workflow_outer_iterations.iteration,
                   workflow_outer_iterations.revision_id,
                   workflow_outer_iterations.structural_report_json,
                   workflow_outer_iterations.vlm_report_json,
                   workflow_outer_iterations.failure_report_json,
                   workflow_outer_iterations.passed,
                   workflow_outer_iterations.created_at
            FROM workflow_outer_iterations
            INNER JOIN agent_runs ON agent_runs.id = workflow_outer_iterations.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_outer_iterations.run_id ASC,
                     workflow_outer_iterations.iteration ASC,
                     workflow_outer_iterations.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let structural_report_json: String = row.get(4)?;
            let vlm_report_json: Option<String> = row.get(5)?;
            let failure_report_json: Option<String> = row.get(6)?;
            Ok(CadWorkflowOuterIteration {
                id: row.get(0)?,
                run_id: row.get(1)?,
                iteration: row.get::<_, i64>(2)?.max(0) as u64,
                revision_id: row.get(3)?,
                structural_report: serde_json::from_str(&structural_report_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                vlm_report: optional_json_value(vlm_report_json).map_err(to_rusqlite_error)?,
                failure_report: optional_json_value(failure_report_json)
                    .map_err(to_rusqlite_error)?,
                passed: row.get::<_, i64>(7)? != 0,
                created_at: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut iterations: HashMap<String, Vec<CadWorkflowOuterIteration>> = HashMap::new();
    for row in rows {
        let iteration = row.map_err(|error| error.to_string())?;
        iterations
            .entry(iteration.run_id.clone())
            .or_default()
            .push(iteration);
    }
    Ok(iterations)
}

pub(super) fn load_workflow_pending_vlm(
    connection: &Connection,
) -> SessionRepositoryResult<HashMap<String, CadWorkflowPendingVlm>> {
    let mut statement = connection
        .prepare(
            r#"
            SELECT workflow_pending_vlm.run_id, workflow_pending_vlm.artifact_id,
                   workflow_pending_vlm.contract_json,
                   workflow_pending_vlm.pass_threshold,
                   workflow_pending_vlm.created_at,
                   workflow_pending_vlm.revision_id,
                   workflow_pending_vlm.structural_report_json
            FROM workflow_pending_vlm
            INNER JOIN agent_runs ON agent_runs.id = workflow_pending_vlm.run_id
            INNER JOIN sessions ON sessions.id = agent_runs.session_id
            WHERE sessions.deleted_at IS NULL
            ORDER BY workflow_pending_vlm.created_at ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let contract_json: String = row.get(2)?;
            let structural_report_json: Option<String> = row.get(6)?;
            Ok(CadWorkflowPendingVlm {
                run_id: row.get(0)?,
                artifact_id: row.get(1)?,
                revision_id: row.get(5)?,
                contract: serde_json::from_str(&contract_json)
                    .map_err(|error| to_rusqlite_error(error.to_string()))?,
                pass_threshold: row.get(3)?,
                structural_report: structural_report_json
                    .map(|value| {
                        serde_json::from_str(&value)
                            .map_err(|error| to_rusqlite_error(error.to_string()))
                    })
                    .transpose()?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let mut pending = HashMap::new();
    for row in rows {
        let pending_vlm = row.map_err(|error| error.to_string())?;
        pending.insert(pending_vlm.run_id.clone(), pending_vlm);
    }
    Ok(pending)
}

pub(super) fn attach_artifacts_to_revisions(
    revisions: &mut HashMap<String, CadRevision>,
    artifacts: &HashMap<String, CadArtifact>,
) {
    for artifact in artifacts
        .values()
        .filter(|artifact| artifact.deleted_at.is_none())
    {
        if let Some(revision) = revisions.get_mut(&artifact.revision_id) {
            revision.artifacts.push(artifact.clone());
        }
    }
    for revision in revisions.values_mut() {
        revision
            .artifacts
            .sort_by(|left, right| left.created_at.cmp(&right.created_at));
        revision.artifact_count = revision.artifacts.len();
    }
}
