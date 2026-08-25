use super::*;

impl SessionService {
    pub fn update_model_source(
        &self,
        input: UpdateModelSourceInput,
    ) -> Result<UpdateModelSourceResult, String> {
        let revision_id = uuid();
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, &input.session_id)?;
            let now = timestamp();
            let parameters = input
                .parameters
                .unwrap_or_else(|| match input.source_language {
                    CadSourceLanguage::Openscad => extract_open_scad_parameters(&input.source),
                    _ => Vec::new(),
                });
            let revision = CadRevision {
                id: revision_id.clone(),
                session_id: input.session_id.clone(),
                parent_revision_id: input.parent_revision_id,
                restored_from_revision_id: None,
                source_hash: source_hash(&input.source),
                source_language: input.source_language,
                source: input.source,
                parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
                run_links: Vec::new(),
            };
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(revision_id.clone());
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionCreated,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok(UpdateModelSourceResult {
            revision_id,
            state: state_snapshot,
        })
    }

    pub fn set_active_revision(
        &self,
        input: SetActiveRevisionInput,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, &input.revision_id)?;
            if revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {} does not belong to session {}.",
                    input.revision_id, input.session_id
                ));
            }
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(input.revision_id.clone());
            session.updated_at = timestamp();
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionActivated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn restore_revision(
        &self,
        input: RestoreRevisionInput,
    ) -> Result<RestoreRevisionResult, String> {
        let revision_id = uuid();
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?.clone();
            let source_revision = require_revision(&state, &input.revision_id)?.clone();
            if source_revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {} does not belong to session {}.",
                    input.revision_id, input.session_id
                ));
            }
            let now = timestamp();
            let mut revision = CadRevision {
                id: revision_id.clone(),
                session_id: input.session_id.clone(),
                parent_revision_id: session.active_revision_id.clone(),
                restored_from_revision_id: Some(input.revision_id.clone()),
                source_hash: source_hash(&source_revision.source),
                source_language: source_revision.source_language,
                source: source_revision.source,
                parameters: source_revision.parameters,
                created_at: now.clone(),
                diagnostics: ok_diagnostics(0),
                artifact_count: 0,
                artifacts: Vec::new(),
                user_events: Vec::new(),
                run_links: Vec::new(),
            };
            add_user_event(
                &mut revision,
                "revision.restored",
                json!({ "restoredFromRevisionId": input.revision_id }),
            );
            state.revisions.insert(revision_id.clone(), revision);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.active_revision_id = Some(revision_id.clone());
            session.updated_at = now;
            session.status = CadSessionStatus::Idle;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::RevisionRestored,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok(RestoreRevisionResult {
            revision_id,
            state: state_snapshot,
        })
    }
}
