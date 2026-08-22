use super::*;

#[test]
fn create_agent_run_rejects_a_second_nonterminal_run_in_the_same_session() {
    let service = SessionService::new(
        std::env::temp_dir().join(format!("cadastrophe-active-run-test-{}", uuid())),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (first, _) = service
        .create_agent_run(
            &created.session_id,
            "first".to_string(),
            None,
            Some("test".to_string()),
            None,
        )
        .unwrap();

    let error = service
        .create_agent_run(
            &created.session_id,
            "second".to_string(),
            None,
            Some("test".to_string()),
            None,
        )
        .unwrap_err();

    assert!(error.contains(&first.id));
    assert!(error.contains("already has an active agent run"));

    service
        .update_agent_run(
            &created.session_id,
            &first.id,
            Some(CadAgentRunStatus::Completed),
            Some(None),
            None,
            None,
            None,
        )
        .unwrap();
    service
        .create_agent_run(
            &created.session_id,
            "after terminal".to_string(),
            None,
            Some("test".to_string()),
            None,
        )
        .unwrap();
}

#[test]
fn sqlite_restart_restores_session_revision_artifacts_conversation_and_runs_together() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-restart-integrity-test-{}", uuid()));
    let layout = StorageLayout::from_app_data_dir(app_data_dir);
    storage::initialize_storage(&layout).unwrap();

    let service = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let created = service
        .create_session(CreateCadSessionInput {
            title: Some("Restart fixture".to_string()),
            selected_runtime: None,
        })
        .unwrap();
    service.mark_session_viewed(&created.session_id).unwrap();
    let updated = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "sphere(r = 5);".to_string(),
            parent_revision_id: None,
            parameters: None,
        })
        .unwrap();
    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(updated.revision_id.clone()),
            format: "metadata".to_string(),
        })
        .unwrap();
    let artifact = export.artifact.unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "Persist all restart surfaces.".to_string(),
            Some(updated.revision_id.clone()),
            Some("test-agent".to_string()),
            None,
        )
        .unwrap();
    service
        .create_conversation_message(
            &created.session_id,
            Some(updated.revision_id.clone()),
            CadConversationRole::Assistant,
            "Restart state is durable.".to_string(),
            Some(run.id.clone()),
            None,
        )
        .unwrap();
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, updated.revision_id.clone())
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            Some(CadAgentRunStatus::Completed),
            Some(None),
            None,
            Some(CadBridgeEventType::AgentRunCompleted),
            Some(json!({"status": "completed"})),
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let current = reloaded.get_current_session().unwrap();
    assert_eq!(
        current.session_id.as_deref(),
        Some(created.session_id.as_str())
    );
    let state = current.state.unwrap();
    assert_eq!(state.session.title.as_deref(), Some("Restart fixture"));
    assert_eq!(
        state.session.active_revision_id.as_deref(),
        Some(updated.revision_id.as_str())
    );
    assert_eq!(
        state
            .active_revision
            .as_ref()
            .map(|revision| revision.source.as_str()),
        Some("sphere(r = 5);")
    );
    assert!(state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .any(|candidate| candidate.id == artifact.id));
    assert!(state
        .conversation
        .iter()
        .any(|message| message.content == "Restart state is durable."));
    assert!(state
        .agent_runs
        .iter()
        .any(|candidate| candidate.id == run.id
            && candidate.status == CadAgentRunStatus::Completed
            && candidate.output_revision_id.as_deref()
                == state.session.active_revision_id.as_deref()));
    assert!(state
        .agent_run_events
        .iter()
        .any(|event| event.run_id == run.id
            && event.event_type == CadAgentRunEventType::AgentRunCompleted));
}

#[test]
fn sqlite_repository_recovers_interrupted_missing_corrupt_and_unknown_persistence_state() {
    let app_data_dir = std::env::temp_dir().join(format!("cadastrophe-recovery-test-{}", uuid()));
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
    let revision_id = create_test_revision(&service, &created.session_id, "cube([4, 4, 4]);");
    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(revision_id.clone()),
            format: "metadata".to_string(),
        })
        .unwrap();
    let artifact = export.artifact.unwrap();
    let artifact_path = service.open_artifact(&artifact.id).unwrap().path;

    let orphan_path = layout
        .artifact_path(
            &created.session_id,
            &revision_id,
            "interrupted-write-without-manifest",
            "stl",
        )
        .unwrap();
    fs::create_dir_all(orphan_path.parent().unwrap()).unwrap();
    fs::write(&orphan_path, b"partial artifact").unwrap();
    fs::write(&artifact_path, b"tampered metadata artifact").unwrap();

    let connection = rusqlite::Connection::open(layout.database_path()).unwrap();
    connection
        .execute(
            "UPDATE sessions SET selected_runtime = 'runtime-from-the-future' WHERE id = ?1",
            rusqlite::params![created.session_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE revisions SET source_language = 'braincad' WHERE id = ?1",
            rusqlite::params![revision_id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE artifacts SET metadata_json = '{not-json' WHERE id = ?1",
            rusqlite::params![artifact.id],
        )
        .unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    assert_eq!(state.session.selected_runtime, CadRuntimeKind::OpenscadWasm);
    assert!(state
        .session
        .recovery_diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message.contains("Unknown persisted runtime") }));
    let active_revision = state.active_revision.as_ref().unwrap();
    assert_eq!(active_revision.source_language, CadSourceLanguage::Openscad);
    assert!(active_revision.diagnostics.items.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("Unknown persisted source language")
    }));

    let verified = reloaded
        .verify_artifact_files(Some(created.session_id.clone()))
        .unwrap();
    assert_eq!(verified.checked_count, 1);
    assert_eq!(
        verified.hash_mismatch_artifact_ids,
        vec![artifact.id.clone()]
    );
    assert_eq!(
        verified.size_mismatch_artifact_ids,
        vec![artifact.id.clone()]
    );
    assert_eq!(verified.corrupt_metadata_artifact_ids, vec![artifact.id]);
    assert!(verified
        .orphan_paths
        .iter()
        .any(|path| path.ends_with("interrupted-write-without-manifest.stl")));
    assert!(verified
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message.contains("Unknown persisted runtime") }));
    assert!(verified.diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("Unknown persisted source language")
    }));
    assert!(verified
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message.contains("corrupt persisted metadata") }));
    assert!(verified
        .diagnostics
        .iter()
        .any(|diagnostic| { diagnostic.message.contains("without a SQLite manifest") }));
}
