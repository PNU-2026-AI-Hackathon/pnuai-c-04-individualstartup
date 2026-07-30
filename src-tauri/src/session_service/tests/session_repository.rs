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
    assert_eq!(
        state
            .active_revision
            .as_ref()
            .map(|revision| revision.source.as_str()),
        Some(DEFAULT_SAMPLE_SOURCE)
    );
    assert_eq!(reloaded.list_sessions(false).unwrap().len(), 1);
}

#[test]
fn first_run_boot_creates_example_once_and_persists_completion() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-first-run-test-{}", uuid()));
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
    assert_eq!(
        first
            .state
            .active_revision
            .as_ref()
            .map(|revision| revision.source.as_str()),
        Some(DEFAULT_SAMPLE_SOURCE)
    );
    assert_eq!(first.state.session.title_source, CadSessionTitleSource::System);

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
    let created = service.create_session(CreateCadSessionInput::default()).unwrap();

    let (_, prompted) = service
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
    assert_eq!(listed.sessions[0].revision_count, 2);
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
    assert_eq!(
        reloaded
            .get_session_state(&duplicated.session_id)
            .unwrap()
            .active_revision
            .as_ref()
            .map(|revision| revision.source.as_str()),
        Some(DEFAULT_SAMPLE_SOURCE)
    );

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
    let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
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
