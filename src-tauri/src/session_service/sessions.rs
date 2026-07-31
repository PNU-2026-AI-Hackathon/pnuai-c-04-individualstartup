use super::*;

impl SessionService {
    pub fn create_session(
        &self,
        input: CreateCadSessionInput,
    ) -> Result<CreateCadSessionResult, String> {
        let title_source = if input
            .title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
        {
            CadSessionTitleSource::User
        } else {
            CadSessionTitleSource::System
        };
        self.create_session_with_title_source(input, title_source)
    }

    fn create_session_with_title_source(
        &self,
        input: CreateCadSessionInput,
        title_source: CadSessionTitleSource,
    ) -> Result<CreateCadSessionResult, String> {
        let now = timestamp();
        let session_id = uuid();
        let session = CadSession {
            id: session_id.clone(),
            created_at: now.clone(),
            updated_at: now,
            last_viewed_at: None,
            connected_ui_clients: 0,
            title: Some(
                input
                    .title
                    .unwrap_or_else(|| "Untitled CAD session".to_string()),
            ),
            title_source,
            active_revision_id: None,
            selected_runtime: input
                .selected_runtime
                .unwrap_or(CadRuntimeKind::OpenscadWasm),
            status: CadSessionStatus::Idle,
            recovery_diagnostics: Vec::new(),
            archived_at: None,
            deleted_at: None,
            revisions: Vec::new(),
        };
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            state.sessions.insert(session_id.clone(), session);
            state.messages.insert(session_id.clone(), Vec::new());
            state.conversation.insert(session_id.clone(), Vec::new());
            state.agent_runs.insert(session_id.clone(), Vec::new());
            state
                .agent_run_events
                .insert(session_id.clone(), Vec::new());
        }
        self.update_model_source(UpdateModelSourceInput {
            session_id: session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: DEFAULT_SAMPLE_SOURCE.to_string(),
            parent_revision_id: None,
            parameters: None,
        })?;
        let state = self.get_session_state(&session_id)?;
        self.emit(
            CadBridgeEventType::SessionCreated,
            &session_id,
            state.clone(),
        );
        Ok(CreateCadSessionResult {
            session_id: session_id.clone(),
            ui_url: format!("/sessions/{session_id}"),
            state,
        })
    }

    pub fn boot_session(&self) -> Result<BootCadSessionResult, String> {
        let (has_completed_first_run, current_session_id) = {
            let state = self.inner.lock().map_err(lock_error)?;
            (
                state.has_completed_first_run,
                state.current_interactive_session_id.clone(),
            )
        };
        if !has_completed_first_run {
            let created = self.create_session_with_title_source(
                CreateCadSessionInput {
                    title: Some("Example OpenSCAD session".to_string()),
                    selected_runtime: Some(CadRuntimeKind::OpenscadWasm),
                },
                CadSessionTitleSource::System,
            )?;
            self.mark_session_viewed(&created.session_id)?;
            self.mark_first_run_completed()?;
            let state = self.get_session_state(&created.session_id)?;
            return Ok(BootCadSessionResult {
                session_id: created.session_id.clone(),
                ui_url: format!("/sessions/{}", created.session_id),
                state,
                is_first_run: true,
                created_session: true,
                should_use_example_session: true,
                should_auto_render: true,
            });
        }

        if let Some(session_id) = current_session_id {
            return Ok(BootCadSessionResult {
                ui_url: format!("/sessions/{session_id}"),
                state: self.get_session_state(&session_id)?,
                session_id,
                is_first_run: false,
                created_session: false,
                should_use_example_session: false,
                should_auto_render: false,
            });
        }

        let created = self.create_session(CreateCadSessionInput::default())?;
        self.mark_session_viewed(&created.session_id)?;
        let state = self.get_session_state(&created.session_id)?;
        Ok(BootCadSessionResult {
            session_id: created.session_id.clone(),
            ui_url: format!("/sessions/{}", created.session_id),
            state,
            is_first_run: false,
            created_session: true,
            should_use_example_session: false,
            should_auto_render: false,
        })
    }

    fn mark_first_run_completed(&self) -> Result<(), String> {
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            if state.has_completed_first_run {
                return Ok(());
            }
            state.has_completed_first_run = true;
        }
        self.repository
            .set_app_kv_json("hasCompletedFirstRun", &Value::Bool(true))
    }

    pub fn get_current_session(&self) -> Result<CurrentCadSessionResult, String> {
        let session_id = {
            let state = self.inner.lock().map_err(lock_error)?;
            state.current_interactive_session_id.clone()
        };
        let Some(session_id) = session_id else {
            return Ok(CurrentCadSessionResult::default());
        };
        Ok(CurrentCadSessionResult {
            ui_url: Some(format!("/sessions/{session_id}")),
            state: Some(self.get_session_state(&session_id)?),
            session_id: Some(session_id),
        })
    }

    pub fn mark_session_viewed(&self, session_id: &str) -> Result<CadSessionState, String> {
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session_mut(&mut state, session_id)?;
            let now = timestamp();
            let session = state.sessions.get_mut(session_id).expect("session checked");
            session.last_viewed_at = Some(now.clone());
            session.updated_at = now;
            if session.archived_at.is_none() {
                state.current_interactive_session_id = Some(session_id.to_string());
            }
            self.persist_session_graph(&state, session_id)?;
        }
        self.get_session_state(session_id)
    }

    #[cfg(test)]
    pub fn list_sessions(&self, include_archived: bool) -> Result<Vec<CadSessionListItem>, String> {
        self.list_sessions_for_input(ListCadSessionsInput {
            include_archived,
            query: None,
        })
        .map(|result| result.sessions)
    }

    pub fn list_sessions_for_input(
        &self,
        input: ListCadSessionsInput,
    ) -> Result<ListCadSessionsResult, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let query = normalized_query(input.query.as_deref());
        let mut sessions: Vec<CadSessionListItem> = state
            .sessions
            .values()
            .filter(|session| session.deleted_at.is_none())
            .filter(|session| input.include_archived || session.archived_at.is_none())
            .filter(|session| {
                query
                    .as_deref()
                    .is_none_or(|query| session_matches_search(&state, session, query))
            })
            .map(|session| session_list_item(&state, session))
            .collect();
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(ListCadSessionsResult {
            sessions,
            search_fields: vec![
                "title".to_string(),
                "source".to_string(),
                "conversation".to_string(),
            ],
        })
    }

    pub fn rename_session(&self, input: RenameCadSessionInput) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.title = Some(input.title.trim().to_string());
            session.title_source = CadSessionTitleSource::User;
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn archive_session(
        &self,
        input: ArchiveCadSessionInput,
    ) -> Result<CadSessionState, String> {
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let now = timestamp();
            let archived = input.archived.unwrap_or(true);
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.archived_at = archived.then_some(now.clone());
            session.updated_at = now;
            if archived
                && state
                    .current_interactive_session_id
                    .as_deref()
                    .is_some_and(|current| current == input.session_id)
            {
                state.current_interactive_session_id = None;
            }
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionUpdated,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(snapshot)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<DeleteCadSessionResult, String> {
        let deleted_at = timestamp();
        eprintln!(
            "[cadastrophe:delete-session] service delete started session_id={} deleted_at={}",
            session_id, deleted_at
        );
        let current_session_id = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            require_session(&state, session_id)?;
            eprintln!(
                "[cadastrophe:delete-session] service session found session_id={}",
                session_id
            );
            state.sessions.remove(session_id);
            state.messages.remove(session_id);
            state.conversation.remove(session_id);
            let run_ids: Vec<String> = state
                .agent_runs
                .get(session_id)
                .into_iter()
                .flatten()
                .map(|run| run.id.clone())
                .collect();
            state.agent_runs.remove(session_id);
            state.agent_run_events.remove(session_id);
            for run_id in &run_ids {
                state.workflow_plans.remove(run_id);
                state.workflow_outer_iterations.remove(run_id);
                state.workflow_pending_vlm.remove(run_id);
            }
            let revision_ids: Vec<String> = state
                .revisions
                .values()
                .filter(|revision| revision.session_id == session_id)
                .map(|revision| revision.id.clone())
                .collect();
            eprintln!(
                "[cadastrophe:delete-session] service removing related state session_id={} run_count={} revision_count={}",
                session_id,
                run_ids.len(),
                revision_ids.len()
            );
            for revision_id in &revision_ids {
                state.revisions.remove(revision_id);
            }
            state
                .artifacts
                .retain(|_, artifact| !revision_ids.contains(&artifact.revision_id));
            if state
                .current_interactive_session_id
                .as_deref()
                .is_some_and(|current| current == session_id)
            {
                state.current_interactive_session_id = None;
            }
            state.current_interactive_session_id.clone()
        };
        self.repository.delete_session(session_id, &deleted_at)?;
        eprintln!(
            "[cadastrophe:delete-session] service delete persisted session_id={} current_session_id={:?}",
            session_id, current_session_id
        );
        Ok(DeleteCadSessionResult {
            session_id: session_id.to_string(),
            current_session_id,
        })
    }

    pub fn duplicate_session(
        &self,
        input: DuplicateCadSessionInput,
    ) -> Result<CreateCadSessionResult, String> {
        let now = timestamp();
        let new_session_id = uuid();
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let source_session = require_session(&state, &input.session_id)?.clone();
            let active_revision = source_session
                .active_revision_id
                .as_ref()
                .and_then(|revision_id| state.revisions.get(revision_id))
                .cloned();
            let provided_title = input.title;
            let title_source = if provided_title
                .as_deref()
                .is_some_and(|title| !title.trim().is_empty())
            {
                CadSessionTitleSource::User
            } else {
                source_session.title_source.clone()
            };
            let title = provided_title.or_else(|| {
                source_session
                    .title
                    .as_ref()
                    .map(|title| format!("{title} copy"))
            });
            let active_revision_id = active_revision.as_ref().map(|_| uuid());
            let session = CadSession {
                id: new_session_id.clone(),
                created_at: now.clone(),
                updated_at: now.clone(),
                last_viewed_at: None,
                connected_ui_clients: 0,
                title,
                title_source,
                active_revision_id: active_revision_id.clone(),
                selected_runtime: source_session.selected_runtime,
                status: CadSessionStatus::Idle,
                recovery_diagnostics: source_session.recovery_diagnostics,
                archived_at: None,
                deleted_at: None,
                revisions: Vec::new(),
            };
            state.sessions.insert(new_session_id.clone(), session);
            state.messages.insert(new_session_id.clone(), Vec::new());
            state
                .conversation
                .insert(new_session_id.clone(), Vec::new());
            state.agent_runs.insert(new_session_id.clone(), Vec::new());
            state
                .agent_run_events
                .insert(new_session_id.clone(), Vec::new());
            if let (Some(mut revision), Some(new_revision_id)) =
                (active_revision, active_revision_id)
            {
                revision.id = new_revision_id.clone();
                revision.session_id = new_session_id.clone();
                revision.parent_revision_id = None;
                revision.restored_from_revision_id = None;
                revision.source_hash = source_hash(&revision.source);
                revision.created_at = now.clone();
                revision.artifact_count = 0;
                revision.artifacts = Vec::new();
                revision.user_events = Vec::new();
                revision.run_links = Vec::new();
                state.revisions.insert(new_revision_id, revision);
            }
            rebuild_revision_summaries(&mut state, &new_session_id);
            self.persist_session_graph(&state, &new_session_id)?;
            build_state(&state, &new_session_id)?
        };
        self.emit(
            CadBridgeEventType::SessionCreated,
            &new_session_id,
            snapshot.clone(),
        );
        Ok(CreateCadSessionResult {
            session_id: new_session_id.clone(),
            ui_url: format!("/sessions/{new_session_id}"),
            state: snapshot,
        })
    }

    pub(super) fn maybe_update_session_title_from_text(
        &self,
        state: &mut ServiceState,
        session_id: &str,
        text: &str,
    ) -> Result<bool, String> {
        let Some(proposed_title) = propose_session_title(text) else {
            return Ok(false);
        };
        let session = require_session_mut(state, session_id)?;
        if session.title_source == CadSessionTitleSource::User {
            return Ok(false);
        }
        if session.title.as_deref() == Some(proposed_title.as_str()) {
            return Ok(false);
        }
        session.title = Some(proposed_title);
        session.title_source = CadSessionTitleSource::Agent;
        session.updated_at = timestamp();
        rebuild_revision_summaries(state, session_id);
        Ok(true)
    }
}
