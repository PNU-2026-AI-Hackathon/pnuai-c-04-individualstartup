use super::*;

#[test]
fn sqlite_repository_restores_current_session_and_session_index() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput {
            title: Some("Original title".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    service.mark_session_viewed(&created.session_id).unwrap();
    service
        .rename_session(RenameCadSessionInput {
            session_id: created.session_id.clone(),
            title: "Persisted title".to_string(),
        })
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let current = reloaded.get_current_session().unwrap();

    assert_eq!(
        current.session_id.as_deref(),
        Some(created.session_id.as_str())
    );
    let state = current.state.expect("current session state");
    assert_eq!(state.session.title.as_deref(), Some("Persisted title"));
    assert_eq!(state.session.title_source, CadSessionTitleSource::User);
    assert!(state.session.active_revision_id.is_none());
    assert!(state.active_revision.is_none());
    assert!(state.session.revisions.is_empty());
    assert_eq!(reloaded.list_sessions(false).unwrap().len(), 1);
}

#[test]
fn first_run_boot_creates_example_once_and_persists_completion() {
    let app_data_dir = std::env::temp_dir().join(format!("cadastrophe-first-run-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let first = service.boot_session().unwrap();

    assert!(first.is_first_run);
    assert!(first.created_session);
    assert!(first.should_use_example_session);
    assert!(first.should_auto_render);
    assert!(first.state.session.active_revision_id.is_none());
    assert!(first.state.active_revision.is_none());
    assert!(first.state.session.revisions.is_empty());
    assert_eq!(
        first.state.session.title_source,
        CadSessionTitleSource::System
    );

    let second = service.boot_session().unwrap();
    assert!(!second.is_first_run);
    assert!(!second.created_session);
    assert_eq!(second.session_id, first.session_id);

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let after_reload = reloaded.boot_session().unwrap();
    assert!(!after_reload.is_first_run);
    assert!(!after_reload.created_session);
    assert_eq!(after_reload.session_id, first.session_id);
    assert_eq!(reloaded.list_sessions(false).unwrap().len(), 1);
}

#[test]
fn generated_title_updates_from_prompt_until_user_rename() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-title-source-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();

    let (first_run, prompted) = service
        .create_agent_run(
            &created.session_id,
            "Create a slotted fixture plate with rounded corners".to_string(),
            created.state.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();
    assert_eq!(
        prompted.session.title.as_deref(),
        Some("Slotted Fixture Plate Rounded")
    );
    assert_eq!(prompted.session.title_source, CadSessionTitleSource::Agent);
    service
        .update_agent_run(
            &created.session_id,
            &first_run.id,
            Some(CadAgentRunStatus::Completed),
            Some(None),
            None,
            None,
            None,
        )
        .unwrap();

    let renamed = service
        .rename_session(RenameCadSessionInput {
            session_id: created.session_id.clone(),
            title: "My saved name".to_string(),
        })
        .unwrap();
    assert_eq!(renamed.session.title_source, CadSessionTitleSource::User);

    service
        .create_agent_run(
            &created.session_id,
            "Create a different hinge bracket".to_string(),
            prompted.session.active_revision_id.clone(),
            Some("test".to_string()),
            None,
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    assert_eq!(state.session.title.as_deref(), Some("My saved name"));
    assert_eq!(state.session.title_source, CadSessionTitleSource::User);
    assert_eq!(
        reloaded.list_sessions(false).unwrap()[0].title_source,
        CadSessionTitleSource::User
    );
}

#[test]
fn session_list_returns_active_revision_summary_and_searches_title_source_conversation() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-list-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let bracket = service
        .create_session(CreateCadSessionInput {
            title: Some("Bracket Assembly".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    service
        .update_model_source(UpdateModelSourceInput {
            session_id: bracket.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "// mounting_slot_fixture\ndifference() { cube([8, 8, 2]); cylinder(r = 2); }"
                .to_string(),
            parent_revision_id: bracket.state.session.active_revision_id.clone(),
            parameters: None,
        })
        .unwrap();
    service
        .post_user_message(PostUserMessageInput {
            session_id: bracket.session_id.clone(),
            revision_id: None,
            message: "Needs a mounting tab".to_string(),
        })
        .unwrap();
    let _other = service
        .create_session(CreateCadSessionInput {
            title: Some("Plain block".to_string()),
            selected_runtime: None,
        })
        .unwrap();

    let listed = service
        .list_sessions_for_input(ListCadSessionsInput {
            include_archived: false,
            query: Some("mounting_slot_fixture".to_string()),
        })
        .unwrap();
    assert_eq!(
        listed.search_fields,
        vec!["title", "source", "conversation"]
    );
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].id, bracket.session_id);
    assert_eq!(
        listed.sessions[0].title.as_deref(),
        Some("Bracket Assembly")
    );
    assert!(listed.sessions[0].active_revision.is_some());
    assert_eq!(listed.sessions[0].revision_count, 1);
    assert_eq!(listed.sessions[0].archived, false);

    let conversation_match = service
        .list_sessions_for_input(ListCadSessionsInput {
            include_archived: false,
            query: Some("mounting tab".to_string()),
        })
        .unwrap();
    assert_eq!(conversation_match.sessions.len(), 1);
    assert_eq!(conversation_match.sessions[0].id, bracket.session_id);
}

#[test]
fn archived_sessions_open_readable_but_do_not_become_current_and_deleted_is_explicit_error() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-state-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let active = service
        .create_session(CreateCadSessionInput {
            title: Some("Active".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    let archived = service
        .create_session(CreateCadSessionInput {
            title: Some("Archived".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    service.mark_session_viewed(&active.session_id).unwrap();
    service
        .archive_session(ArchiveCadSessionInput {
            session_id: archived.session_id.clone(),
            archived: Some(true),
        })
        .unwrap();

    let archived_state = service.get_session_state(&archived.session_id).unwrap();
    assert!(archived_state.session.archived_at.is_some());
    service.mark_session_viewed(&archived.session_id).unwrap();
    assert_eq!(
        service.get_current_session().unwrap().session_id.as_deref(),
        Some(active.session_id.as_str())
    );

    service.delete_session(&archived.session_id).unwrap();
    let error = service
        .get_session_state(&archived.session_id)
        .expect_err("deleted session should not open");
    assert!(error.contains("missing or has been deleted"));
}

#[test]
fn sqlite_repository_persists_duplicate_archive_and_delete() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput {
            title: Some("Original".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    let duplicated = service
        .duplicate_session(DuplicateCadSessionInput {
            session_id: created.session_id.clone(),
            title: Some("Copy".to_string()),
        })
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let sessions = reloaded.list_sessions(false).unwrap();
    assert_eq!(sessions.len(), 2);
    assert!(sessions
        .iter()
        .any(|session| session.title.as_deref() == Some("Copy")));
    let duplicated_state = reloaded.get_session_state(&duplicated.session_id).unwrap();
    assert!(duplicated_state.session.active_revision_id.is_none());
    assert!(duplicated_state.active_revision.is_none());

    reloaded
        .archive_session(ArchiveCadSessionInput {
            session_id: created.session_id.clone(),
            archived: None,
        })
        .unwrap();
    reloaded.delete_session(&duplicated.session_id).unwrap();

    let reloaded_again = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    assert!(reloaded_again
        .list_sessions(false)
        .unwrap()
        .iter()
        .all(|session| session.id != created.session_id));
    let archived_sessions = reloaded_again.list_sessions(true).unwrap();
    assert_eq!(archived_sessions.len(), 1);
    assert_eq!(archived_sessions[0].id, created.session_id);
    assert!(archived_sessions[0].archived_at.is_some());
    assert!(reloaded_again
        .get_session_state(&duplicated.session_id)
        .is_err());
}

#[test]
fn duplicate_preserves_source_and_starts_without_artifacts() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-duplicate-source-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let source = "difference() { cube([8, 8, 2]); cylinder(r = 2); }";
    let revision_id = create_test_revision(&service, &created.session_id, source);
    service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(revision_id),
            format: "metadata".to_string(),
        })
        .unwrap();

    let original = service.get_session_state(&created.session_id).unwrap();
    let original_revision = original.active_revision.expect("original active revision");
    assert_eq!(original_revision.source, source);
    assert_eq!(original_revision.artifact_count, 1);
    assert_eq!(original_revision.artifacts.len(), 1);

    let duplicated = service
        .duplicate_session(DuplicateCadSessionInput {
            session_id: created.session_id,
            title: None,
        })
        .unwrap();
    let duplicated_revision = duplicated
        .state
        .active_revision
        .expect("duplicated active revision");
    assert_eq!(duplicated_revision.source, source);
    assert_eq!(
        duplicated_revision.source_hash,
        storage::sha256_hex(source.as_bytes())
    );
    assert_eq!(duplicated_revision.artifact_count, 0);
    assert!(duplicated_revision.artifacts.is_empty());
    assert_eq!(duplicated.state.session.revisions.len(), 1);
    assert_eq!(duplicated.state.session.revisions[0].artifact_count, 0);

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let persisted = reloaded
        .get_session_state(&duplicated.session_id)
        .unwrap()
        .active_revision
        .expect("persisted duplicated active revision");
    assert_eq!(persisted.source, source);
    assert_eq!(persisted.artifact_count, 0);
    assert!(persisted.artifacts.is_empty());
}

#[test]
fn restart_marks_persisted_loaded_thread_not_loaded_without_changing_mapping() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-thread-restart-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();
    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let now = timestamp();
    let thread = service
        .upsert_agent_thread(CadAgentThread {
            id: uuid(),
            session_id: created.session_id.clone(),
            plane: CadAgentPlane::Modeling,
            owner_id: created.session_id.clone(),
            external_agent: "codex".to_string(),
            external_thread_id: "persisted-thread".to_string(),
            status: CadAgentThreadStatus::Active,
            connection_generation: Some(7),
            created_at: now.clone(),
            updated_at: now,
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: None,
        })
        .unwrap();
    drop(service);

    let restarted = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let loaded = restarted.list_agent_threads(&created.session_id).unwrap();
    let loaded = loaded
        .iter()
        .find(|candidate| candidate.id == thread.id)
        .unwrap();
    assert_eq!(loaded.status, CadAgentThreadStatus::NotLoaded);
    assert_eq!(loaded.external_thread_id, "persisted-thread");
    assert_eq!(loaded.connection_generation, None);

    drop(restarted);
    let persisted = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    assert_eq!(
        persisted.list_agent_threads(&created.session_id).unwrap()[0].status,
        CadAgentThreadStatus::NotLoaded
    );
}

#[test]
fn sqlite_repository_rejects_stale_save_after_session_delete() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-delete-stale-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let first_process = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = first_process
        .create_session(CreateCadSessionInput {
            title: Some("Delete target".to_string()),
            selected_runtime: None,
        })
        .unwrap();

    let stale_process = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    assert_eq!(stale_process.list_sessions(false).unwrap().len(), 1);

    first_process.delete_session(&created.session_id).unwrap();

    let stale_error = stale_process
        .mark_session_viewed(&created.session_id)
        .expect_err("stale process must not resurrect a deleted session");
    assert!(stale_error.contains("stale process"));

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    assert!(reloaded.list_sessions(true).unwrap().is_empty());
    assert!(reloaded.get_session_state(&created.session_id).is_err());
}

#[test]
fn sqlite_repository_restores_agent_thread_run_message_and_transport_graph_idempotently() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-agent-graph-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "continue the same design".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let thread = CadAgentThread {
        id: uuid(),
        session_id: created.session_id.clone(),
        plane: CadAgentPlane::Modeling,
        owner_id: created.session_id.clone(),
        external_agent: "codex".to_string(),
        external_thread_id: "external-thread-1".to_string(),
        status: CadAgentThreadStatus::Ready,
        connection_generation: Some(7),
        created_at: "2026-08-12T00:00:00.000Z".to_string(),
        updated_at: "2026-08-12T00:00:00.000Z".to_string(),
        last_resumed_at: Some("2026-08-12T00:00:00.000Z".to_string()),
        archived_at: None,
        replaced_by_id: None,
        metadata: None,
    };
    service.upsert_agent_thread(thread.clone()).unwrap();
    let bound_run = service
        .bind_agent_run_to_thread(
            &created.session_id,
            &run.id,
            &thread.id,
            Some("turn-1".to_string()),
            Some(7),
            CadAgentRecoveryStatus::Resumed,
        )
        .unwrap();
    assert_eq!(
        bound_run.agent_thread_id.as_deref(),
        Some(thread.id.as_str())
    );

    let message = CadConversationMessage {
        id: uuid(),
        session_id: created.session_id.clone(),
        revision_id: None,
        role: CadConversationRole::Assistant,
        content: "partial".to_string(),
        created_at: "2026-08-12T00:00:01.000Z".to_string(),
        run_id: Some(run.id.clone()),
        external_thread_id: Some("external-thread-1".to_string()),
        external_turn_id: Some("turn-1".to_string()),
        external_item_id: Some("item-1".to_string()),
        phase: Some(CadConversationPhase::FinalAnswer),
        sequence: Some(1),
        is_final: false,
        metadata: None,
    };
    let first_saved = service
        .upsert_agent_conversation_message(message.clone())
        .unwrap();
    let completed = CadConversationMessage {
        id: uuid(),
        content: "authoritative completed answer".to_string(),
        is_final: true,
        ..message
    };
    let second_saved = service
        .upsert_agent_conversation_message(completed)
        .unwrap();
    assert_eq!(first_saved.id, second_saved.id);
    assert_eq!(second_saved.content, "authoritative completed answer");
    assert!(second_saved.is_final);

    let transport_event = CadAgentTransportEvent {
        id: uuid(),
        session_id: created.session_id.clone(),
        run_id: Some(run.id.clone()),
        agent_thread_id: Some(thread.id.clone()),
        external_turn_id: Some("turn-1".to_string()),
        external_item_id: Some("item-1".to_string()),
        method: "item/completed".to_string(),
        sequence: 1,
        payload: json!({"item": {"id": "item-1"}}),
        created_at: "2026-08-12T00:00:02.000Z".to_string(),
    };
    service
        .save_agent_transport_event(transport_event.clone())
        .unwrap();
    service.save_agent_transport_event(transport_event).unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    assert_eq!(state.agent_threads.len(), 1);
    assert_eq!(state.agent_threads[0].id, thread.id);
    assert_eq!(
        state.agent_threads[0].external_thread_id,
        thread.external_thread_id
    );
    assert_eq!(
        state.agent_threads[0].status,
        CadAgentThreadStatus::NotLoaded
    );
    assert_eq!(state.agent_threads[0].connection_generation, None);
    let restored_run = state
        .agent_runs
        .iter()
        .find(|candidate| candidate.id == run.id)
        .unwrap();
    assert_eq!(
        restored_run.agent_thread_id.as_deref(),
        Some(thread.id.as_str())
    );
    assert_eq!(
        restored_run.external_thread_id.as_deref(),
        Some("external-thread-1")
    );
    assert_eq!(restored_run.external_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(restored_run.connection_generation, Some(7));
    assert_eq!(
        restored_run.recovery_status,
        CadAgentRecoveryStatus::Resumed
    );
    let restored_messages = state
        .conversation
        .iter()
        .filter(|candidate| candidate.external_item_id.as_deref() == Some("item-1"))
        .collect::<Vec<_>>();
    assert_eq!(restored_messages.len(), 1);
    assert_eq!(
        restored_messages[0].content,
        "authoritative completed answer"
    );
    assert!(restored_messages[0].is_final);

    let snapshot = reloaded.repository.load().unwrap();
    assert_eq!(
        snapshot
            .agent_transport_events
            .get(&created.session_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn duplicate_never_shares_agent_thread_and_archive_requires_terminal_runs() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-thread-lifecycle-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();
    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "active run".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let thread = CadAgentThread {
        id: uuid(),
        session_id: created.session_id.clone(),
        plane: CadAgentPlane::Modeling,
        owner_id: created.session_id.clone(),
        external_agent: "codex".to_string(),
        external_thread_id: "external-original".to_string(),
        status: CadAgentThreadStatus::Ready,
        connection_generation: None,
        created_at: "2026-08-12T00:00:00.000Z".to_string(),
        updated_at: "2026-08-12T00:00:00.000Z".to_string(),
        last_resumed_at: None,
        archived_at: None,
        replaced_by_id: None,
        metadata: None,
    };
    service.upsert_agent_thread(thread.clone()).unwrap();
    let duplicated = service
        .duplicate_session(DuplicateCadSessionInput {
            session_id: created.session_id.clone(),
            title: None,
        })
        .unwrap();
    assert!(duplicated.state.agent_threads.is_empty());
    assert!(service
        .archive_session(ArchiveCadSessionInput {
            session_id: created.session_id.clone(),
            archived: Some(true),
        })
        .unwrap_err()
        .contains("active agent run"));

    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            Some(CadAgentRunStatus::Cancelled),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let archived = service
        .archive_session(ArchiveCadSessionInput {
            session_id: created.session_id.clone(),
            archived: Some(true),
        })
        .unwrap();
    assert!(archived.session.archived_at.is_some());
    assert_eq!(archived.agent_threads, vec![thread]);
}

#[test]
fn sqlite_repository_persists_restore_summary_fields_and_artifact_count() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-session-repo-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let root = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "cube([8, 8, 2]);".to_string(),
            parent_revision_id: None,
            parameters: None,
        })
        .unwrap();
    let root_revision_id = root.revision_id.clone();
    let updated = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "sphere(r = 8);".to_string(),
            parent_revision_id: Some(root_revision_id.clone()),
            parameters: None,
        })
        .unwrap();
    service
        .set_active_revision(SetActiveRevisionInput {
            session_id: created.session_id.clone(),
            revision_id: root_revision_id.clone(),
        })
        .unwrap();
    let restored = service
        .restore_revision(RestoreRevisionInput {
            session_id: created.session_id.clone(),
            revision_id: updated.revision_id.clone(),
        })
        .unwrap();
    service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(restored.revision_id.clone()),
            format: "metadata".to_string(),
        })
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    let restored_summary = state
        .session
        .revisions
        .iter()
        .find(|revision| revision.id == restored.revision_id)
        .unwrap();
    assert_eq!(
        restored_summary.parent_revision_id.as_deref(),
        Some(root_revision_id.as_str())
    );
    assert_eq!(
        restored_summary.restored_from_revision_id.as_deref(),
        Some(updated.revision_id.as_str())
    );
    assert_eq!(restored_summary.source_hash.len(), 64);
    assert_eq!(restored_summary.artifact_count, 1);
}
