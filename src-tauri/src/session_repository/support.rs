use super::*;

pub(super) fn artifact_metadata(
    metadata_json: Option<String>,
) -> SessionRepositoryResult<Metadata> {
    let Some(metadata_json) = metadata_json else {
        return Ok(Metadata::new());
    };
    match serde_json::from_str(&metadata_json) {
        Ok(metadata) => Ok(metadata),
        Err(error) => {
            let mut metadata = Metadata::new();
            metadata.insert(
                "metadataRecovery".to_string(),
                json!({
                    "status": "corrupt-metadata",
                    "error": error.to_string()
                }),
            );
            Ok(metadata)
        }
    }
}

pub(super) fn optional_metadata(
    metadata_json: Option<String>,
) -> SessionRepositoryResult<Option<Metadata>> {
    metadata_json
        .map(|json| artifact_metadata(Some(json)))
        .transpose()
}

pub(super) fn optional_metadata_json(
    metadata: Option<&Metadata>,
) -> SessionRepositoryResult<Option<String>> {
    metadata
        .map(|metadata| serde_json::to_string(metadata).map_err(|error| error.to_string()))
        .transpose()
}

pub(super) fn optional_json_value(value: Option<String>) -> SessionRepositoryResult<Option<Value>> {
    value
        .map(|json| serde_json::from_str(&json).map_err(|error| error.to_string()))
        .transpose()
}

pub(super) fn optional_json_value_text(
    value: Option<&Value>,
) -> SessionRepositoryResult<Option<String>> {
    value
        .map(|value| serde_json::to_string(value).map_err(|error| error.to_string()))
        .transpose()
}

pub(super) fn load_current_session_id(
    connection: &Connection,
) -> SessionRepositoryResult<Option<String>> {
    connection
        .query_row(
            r#"
            SELECT id
            FROM sessions
            WHERE deleted_at IS NULL AND archived_at IS NULL
            ORDER BY COALESCE(last_viewed_at, updated_at) DESC
            LIMIT 1
            "#,
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())
}

pub(super) fn load_app_kv_bool(
    connection: &Connection,
    key: &str,
) -> SessionRepositoryResult<bool> {
    let value_json: Option<String> = connection
        .query_row(
            "SELECT value_json FROM app_kv WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(value_json) = value_json else {
        return Ok(false);
    };
    Ok(serde_json::from_str::<bool>(&value_json).unwrap_or(false))
}

pub(super) fn rebuild_loaded_revision_summaries(
    sessions: &mut HashMap<String, CadSession>,
    revisions: &HashMap<String, CadRevision>,
    agent_runs: &HashMap<String, Vec<CadAgentRun>>,
    session_id: &str,
) {
    let mut summaries: Vec<CadRevisionSummary> = revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| CadRevisionSummary {
            id: revision.id.clone(),
            source_hash: storage::sha256_hex(revision.source.as_bytes()),
            parent_revision_id: revision.parent_revision_id.clone(),
            restored_from_revision_id: revision.restored_from_revision_id.clone(),
            source_language: revision.source_language.clone(),
            created_at: revision.created_at.clone(),
            diagnostics: revision.diagnostics.clone(),
            artifact_count: revision.artifact_count,
            run_links: loaded_revision_run_links(agent_runs, session_id, &revision.id),
        })
        .collect();
    summaries.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    if let Some(session) = sessions.get_mut(session_id) {
        session.revisions = summaries;
    }
}

pub(super) fn loaded_revision_run_links(
    agent_runs: &HashMap<String, Vec<CadAgentRun>>,
    session_id: &str,
    revision_id: &str,
) -> Vec<CadRevisionRunLink> {
    let mut links = Vec::new();
    for run in agent_runs.get(session_id).into_iter().flatten() {
        if run.input_revision_id.as_deref() == Some(revision_id) {
            links.push(CadRevisionRunLink {
                run_id: run.id.clone(),
                role: "input".to_string(),
                status: run.status.clone(),
                updated_at: run.updated_at.clone(),
            });
        }
        if run.output_revision_id.as_deref() == Some(revision_id) {
            links.push(CadRevisionRunLink {
                run_id: run.id.clone(),
                role: "output".to_string(),
                status: run.status.clone(),
                updated_at: run.updated_at.clone(),
            });
        }
    }
    links.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
    links
}

pub(super) fn revision_user_events(
    metadata_json: Option<String>,
) -> SessionRepositoryResult<Vec<CadUserEvent>> {
    let Some(metadata_json) = metadata_json else {
        return Ok(Vec::new());
    };
    let Ok(metadata) = serde_json::from_str::<Value>(&metadata_json) else {
        return Ok(Vec::new());
    };
    metadata
        .get("userEvents")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| error.to_string())
        .map(|events| events.unwrap_or_default())
}

pub(super) fn recover_runtime_kind(value: &str) -> (CadRuntimeKind, Option<CadDiagnostic>) {
    match from_db_text(value) {
        Ok(runtime) => (runtime, None),
        Err(_) => (
            CadRuntimeKind::OpenscadWasm,
            Some(CadDiagnostic {
                severity: "warning".to_string(),
                message: format!(
                    "Unknown persisted runtime {value:?}; recovered with openscad-wasm."
                ),
                line: None,
                column: None,
            }),
        ),
    }
}

pub(super) fn recover_source_language(value: &str) -> (CadSourceLanguage, Option<CadDiagnostic>) {
    match from_db_text(value) {
        Ok(source_language) => (source_language, None),
        Err(_) => (
            CadSourceLanguage::Openscad,
            Some(CadDiagnostic {
                severity: "warning".to_string(),
                message: format!(
                    "Unknown persisted source language {value:?}; recovered with openscad."
                ),
                line: None,
                column: None,
            }),
        ),
    }
}

pub(super) fn recover_title_source(value: &str) -> Option<CadSessionTitleSource> {
    from_db_text(value).ok()
}

pub(super) fn recover_diagnostics(value: &str) -> CadDiagnostics {
    serde_json::from_str(value).unwrap_or_else(|error| CadDiagnostics {
        ok: false,
        elapsed_ms: 0,
        items: vec![CadDiagnostic {
            severity: "warning".to_string(),
            message: format!("Corrupt persisted diagnostics were reset: {error}"),
            line: None,
            column: None,
        }],
    })
}

pub(super) fn to_db_text<T: Serialize>(value: &T) -> SessionRepositoryResult<String> {
    match serde_json::to_value(value).map_err(|error| error.to_string())? {
        Value::String(value) => Ok(value),
        other => Err(format!("Expected string-backed enum, got {other:?}")),
    }
}

pub(super) fn from_db_text<T: DeserializeOwned>(value: &str) -> SessionRepositoryResult<T> {
    serde_json::from_value(Value::String(value.to_string())).map_err(|error| error.to_string())
}

pub(super) fn to_rusqlite_error(error: String) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(error.into())
}
