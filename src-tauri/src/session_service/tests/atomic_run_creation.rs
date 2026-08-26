use super::*;

#[test]
fn atomic_run_creation_preserves_title_state_event_and_message_ordering() {
    let app_data_dir = std::env::temp_dir().join(format!("cadgen-ax-atomic-run-test-{}", uuid()));
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
    let mut events = service.subscribe();

    let (run, message, state) = service
        .create_agent_run_with_user_message(
            &created.session_id,
            "Precision mounting bracket".to_string(),
            None,
            Some("codex".to_string()),
            None,
            Some(metadata_from_value(json!({"source": "web-ui"}))),
        )
        .unwrap();

    assert_eq!(run.status, CadAgentRunStatus::Queued);
    assert_eq!(message.role, CadConversationRole::User);
    assert_eq!(message.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(state.agent_runs, vec![run.clone()]);
    assert_eq!(state.conversation, vec![message.clone()]);
    assert_eq!(state.agent_run_events.len(), 1);
    assert_eq!(
        state.agent_run_events[0].event_type,
        CadAgentRunEventType::AgentRunCreated
    );
    assert_eq!(state.agent_run_events[0].sequence, 1);
    assert_eq!(
        state.session.title.as_deref(),
        Some("Precision Mounting Bracket")
    );
    assert_eq!(state.session.title_source, CadSessionTitleSource::Agent);
    let run_created = events.try_recv().unwrap();
    let message_created = events.try_recv().unwrap();
    assert_eq!(run_created.event_type, CadBridgeEventType::AgentRunCreated);
    assert_eq!(
        message_created.event_type,
        CadBridgeEventType::AgentMessageCreated
    );
    assert_eq!(run_created.state.conversation, vec![message.clone()]);
    assert_eq!(message_created.state.conversation, vec![message.clone()]);

    let duplicate_error = service
        .create_agent_run_with_user_message(
            &created.session_id,
            "Second active run".to_string(),
            None,
            Some("codex".to_string()),
            None,
            None,
        )
        .unwrap_err();
    assert!(duplicate_error.contains(&run.id));

    drop(service);
    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let reloaded_state = reloaded.get_session_state(&created.session_id).unwrap();
    assert_eq!(reloaded_state.agent_runs, vec![run]);
    assert_eq!(reloaded_state.conversation, vec![message]);
    assert_eq!(reloaded_state.agent_run_events.len(), 1);
}

#[test]
fn atomic_run_creation_rolls_back_sqlite_and_memory_when_message_insert_fails() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-atomic-run-rollback-test-{}", uuid()));
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
    let before = service.get_session_state(&created.session_id).unwrap();
    let mut bridge_events = service.subscribe();
    let connection = rusqlite::Connection::open(layout.database_path()).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TRIGGER fail_atomic_user_message
            BEFORE INSERT ON conversation_messages
            WHEN NEW.role = 'user'
            BEGIN
              SELECT RAISE(ABORT, 'forced atomic message failure');
            END;
            "#,
        )
        .unwrap();
    drop(connection);

    let error = service
        .create_agent_run_with_user_message(
            &created.session_id,
            "Rollback precision enclosure".to_string(),
            None,
            Some("codex".to_string()),
            None,
            Some(metadata_from_value(json!({"source": "web-ui"}))),
        )
        .unwrap_err();
    assert!(error.contains("forced atomic message failure"));
    let after = service.get_session_state(&created.session_id).unwrap();
    assert_eq!(after.session.title, before.session.title);
    assert_eq!(after.session.updated_at, before.session.updated_at);
    assert_eq!(after.agent_runs, before.agent_runs);
    assert_eq!(after.conversation, before.conversation);
    assert_eq!(after.agent_run_events, before.agent_run_events);
    assert!(matches!(
        bridge_events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    drop(service);
    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let persisted = reloaded.get_session_state(&created.session_id).unwrap();
    assert_eq!(persisted.session.title, before.session.title);
    assert!(persisted.agent_runs.is_empty());
    assert!(persisted.conversation.is_empty());
    assert!(persisted.agent_run_events.is_empty());
}
