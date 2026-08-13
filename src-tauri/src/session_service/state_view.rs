use super::*;

pub(super) fn rebuild_revision_summaries(state: &mut ServiceState, session_id: &str) {
    let mut summaries: Vec<CadRevisionSummary> = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| revision_summary(state, revision))
        .collect();
    summaries.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    if let Some(session) = state.sessions.get_mut(session_id) {
        session.revisions = summaries;
    }
}

pub(super) fn build_state(
    state: &ServiceState,
    session_id: &str,
) -> Result<CadSessionState, String> {
    let session = require_session(state, session_id)?;
    let mut session = session.clone();
    session.revisions = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session_id)
        .map(|revision| revision_summary(state, revision))
        .collect();
    session
        .revisions
        .sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let active_revision = session
        .active_revision_id
        .as_ref()
        .and_then(|revision_id| state.revisions.get(revision_id))
        .map(|revision| revision_with_derived_fields(state, revision));
    Ok(CadSessionState {
        session,
        active_revision,
        messages: state.messages.get(session_id).cloned().unwrap_or_default(),
        conversation: state
            .conversation
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        agent_threads: state
            .agent_threads
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        agent_runs: state
            .agent_runs
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        agent_run_events: state
            .agent_run_events
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        validation_evaluations: state
            .validation_evaluations
            .get(session_id)
            .cloned()
            .unwrap_or_default(),
        workflow: build_workflow_state(state, session_id)?,
    })
}

pub(super) fn build_workflow_state(
    state: &ServiceState,
    session_id: &str,
) -> Result<CadWorkflowState, String> {
    require_session(state, session_id)?;
    let run_ids = state
        .agent_runs
        .get(session_id)
        .into_iter()
        .flatten()
        .map(|run| run.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut plans = run_ids
        .iter()
        .filter_map(|run_id| state.workflow_plans.get(*run_id).cloned())
        .collect::<Vec<_>>();
    plans.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    let mut outer_iterations = run_ids
        .iter()
        .flat_map(|run_id| {
            state
                .workflow_outer_iterations
                .get(*run_id)
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect::<Vec<_>>();
    outer_iterations.sort_by(|left, right| {
        left.run_id
            .cmp(&right.run_id)
            .then_with(|| left.iteration.cmp(&right.iteration))
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    let mut pending_vlm = run_ids
        .iter()
        .filter_map(|run_id| state.workflow_pending_vlm.get(*run_id).cloned())
        .collect::<Vec<_>>();
    pending_vlm.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(CadWorkflowState {
        plans,
        outer_iterations,
        pending_vlm,
    })
}

pub(super) fn revision_summary(state: &ServiceState, revision: &CadRevision) -> CadRevisionSummary {
    CadRevisionSummary {
        id: revision.id.clone(),
        source_hash: source_hash(&revision.source),
        parent_revision_id: revision.parent_revision_id.clone(),
        restored_from_revision_id: revision.restored_from_revision_id.clone(),
        source_language: revision.source_language.clone(),
        created_at: revision.created_at.clone(),
        diagnostics: revision.diagnostics.clone(),
        artifact_count: revision.artifact_count,
        run_links: revision_run_links(state, &revision.session_id, &revision.id),
    }
}

pub(super) fn revision_with_derived_fields(
    state: &ServiceState,
    revision: &CadRevision,
) -> CadRevision {
    let mut revision = revision.clone();
    revision.source_hash = source_hash(&revision.source);
    revision.run_links = revision_run_links(state, &revision.session_id, &revision.id);
    revision
}

pub(super) fn session_list_item(state: &ServiceState, session: &CadSession) -> CadSessionListItem {
    let active_revision = session
        .active_revision_id
        .as_ref()
        .and_then(|revision_id| state.revisions.get(revision_id))
        .map(|revision| revision_summary(state, revision));
    let session_revisions = state
        .revisions
        .values()
        .filter(|revision| revision.session_id == session.id);
    let mut revision_count = 0;
    let mut artifact_count = 0;
    for revision in session_revisions {
        revision_count += 1;
        artifact_count += revision.artifact_count;
    }
    CadSessionListItem {
        id: session.id.clone(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
        last_viewed_at: session.last_viewed_at.clone(),
        title: session.title.clone(),
        title_source: session.title_source.clone(),
        active_revision_id: session.active_revision_id.clone(),
        active_revision,
        selected_runtime: session.selected_runtime.clone(),
        status: session.status.clone(),
        archived: session.archived_at.is_some(),
        archived_at: session.archived_at.clone(),
        revision_count,
        artifact_count,
    }
}

pub(super) fn normalized_query(query: Option<&str>) -> Option<String> {
    query
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(|query| query.to_lowercase())
}

pub(super) fn session_matches_search(
    state: &ServiceState,
    session: &CadSession,
    query: &str,
) -> bool {
    session
        .title
        .as_deref()
        .is_some_and(|title| title.to_lowercase().contains(query))
        || state
            .revisions
            .values()
            .filter(|revision| revision.session_id == session.id)
            .any(|revision| revision.source.to_lowercase().contains(query))
        || state
            .conversation
            .get(&session.id)
            .into_iter()
            .flatten()
            .any(|message| message.content.to_lowercase().contains(query))
}

pub(super) fn revision_run_links(
    state: &ServiceState,
    session_id: &str,
    revision_id: &str,
) -> Vec<CadRevisionRunLink> {
    let mut links = Vec::new();
    for run in state.agent_runs.get(session_id).into_iter().flatten() {
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
