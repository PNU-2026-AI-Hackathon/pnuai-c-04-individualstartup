use super::*;

impl SessionService {
    pub fn get_session_state(&self, session_id: &str) -> Result<CadSessionState, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        build_state(&state, session_id)
    }

    pub fn refresh_session_from_repository(
        &self,
        session_id: &str,
    ) -> Result<CadSessionState, String> {
        let snapshot_state = ServiceState::from(self.repository.load()?);
        let mut refreshed_session = snapshot_state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("CAD session not found: {session_id}"))?;
        let refreshed_run_ids = snapshot_state
            .agent_runs
            .get(session_id)
            .into_iter()
            .flatten()
            .map(|run| run.id.clone())
            .collect::<HashSet<_>>();
        let refreshed_revision_ids = snapshot_state
            .revisions
            .values()
            .filter(|revision| revision.session_id == session_id)
            .map(|revision| revision.id.clone())
            .collect::<HashSet<_>>();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            if let Some(existing_session) = state.sessions.get(session_id) {
                refreshed_session.connected_ui_clients = existing_session.connected_ui_clients;
            }
            let previous_revision_ids = state
                .revisions
                .values()
                .filter(|revision| revision.session_id == session_id)
                .map(|revision| revision.id.clone())
                .collect::<HashSet<_>>();
            let previous_run_ids = state
                .agent_runs
                .get(session_id)
                .into_iter()
                .flatten()
                .map(|run| run.id.clone())
                .collect::<HashSet<_>>();
            state
                .sessions
                .insert(session_id.to_string(), refreshed_session);
            state
                .revisions
                .retain(|revision_id, _| !previous_revision_ids.contains(revision_id));
            state.revisions.extend(
                snapshot_state
                    .revisions
                    .iter()
                    .filter(|(_, revision)| revision.session_id == session_id)
                    .map(|(revision_id, revision)| (revision_id.clone(), revision.clone())),
            );
            state
                .artifacts
                .retain(|_, artifact| !previous_revision_ids.contains(&artifact.revision_id));
            state.artifacts.extend(
                snapshot_state
                    .artifacts
                    .iter()
                    .filter(|(_, artifact)| refreshed_revision_ids.contains(&artifact.revision_id))
                    .map(|(artifact_id, artifact)| (artifact_id.clone(), artifact.clone())),
            );
            state.messages.entry(session_id.to_string()).or_default();
            state.conversation.insert(
                session_id.to_string(),
                snapshot_state
                    .conversation
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            state.agent_runs.insert(
                session_id.to_string(),
                snapshot_state
                    .agent_runs
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            state.agent_run_events.insert(
                session_id.to_string(),
                snapshot_state
                    .agent_run_events
                    .get(session_id)
                    .cloned()
                    .unwrap_or_default(),
            );
            for run_id in previous_run_ids.difference(&refreshed_run_ids) {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            for run_id in &refreshed_run_ids {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            for (run_id, plan) in snapshot_state.workflow_plans {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_plans.insert(run_id, plan);
                }
            }
            for (run_id, iterations) in snapshot_state.workflow_outer_iterations {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_outer_iterations.insert(run_id, iterations);
                }
            }
            for (run_id, pending_vlm) in snapshot_state.workflow_pending_vlm {
                if refreshed_run_ids.contains(&run_id) {
                    state.workflow_pending_vlm.insert(run_id, pending_vlm);
                }
            }
            rebuild_revision_summaries(&mut state, session_id);
            build_state(&state, session_id)?
        };
        self.emit(
            CadBridgeEventType::AgentRunUpdated,
            session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }
}
