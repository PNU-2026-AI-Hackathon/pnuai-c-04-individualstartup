use super::*;

fn test_service() -> SessionService {
    SessionService::new(
        std::env::temp_dir().join(format!("cadgen-ax-operations-recovery-test-{}", uuid())),
    )
}

fn test_thread(session_id: &str, id: &str, external_thread_id: &str) -> CadAgentThread {
    CadAgentThread {
        id: id.to_string(),
        session_id: session_id.to_string(),
        plane: CadAgentPlane::Modeling,
        owner_id: session_id.to_string(),
        external_agent: "codex".to_string(),
        external_thread_id: external_thread_id.to_string(),
        status: CadAgentThreadStatus::Ready,
        connection_generation: Some(1),
        created_at: "2026-08-12T00:00:00.000Z".to_string(),
        updated_at: "2026-08-12T00:00:00.000Z".to_string(),
        last_resumed_at: None,
        archived_at: None,
        replaced_by_id: None,
        metadata: None,
    }
}

#[test]
fn startup_recovery_candidates_never_treat_an_unknown_turn_as_reenqueueable() {
    let service = test_service();
    let queued_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (queued, _) = service
        .create_agent_run(
            &queued_session.session_id,
            "queued".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let history_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (history, _) = service
        .create_agent_run(
            &history_session.session_id,
            "history".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let thread = test_thread(&history_session.session_id, "thread-1", "external-1");
    service.upsert_agent_thread(thread.clone()).unwrap();
    service
        .bind_agent_run_to_thread(
            &history_session.session_id,
            &history.id,
            &thread.id,
            Some("turn-1".to_string()),
            Some(1),
            CadAgentRecoveryStatus::None,
        )
        .unwrap();
    service
        .update_agent_run(
            &history_session.session_id,
            &history.id,
            Some(CadAgentRunStatus::Running),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    let unknown_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (unknown, _) = service
        .create_agent_run(
            &unknown_session.session_id,
            "unknown".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    service
        .update_agent_run(
            &unknown_session.session_id,
            &unknown.id,
            Some(CadAgentRunStatus::Running),
            None,
            None,
            None,
            None,
        )
        .unwrap();

    let candidates = service
        .list_startup_agent_run_recovery_candidates()
        .unwrap();
    assert_eq!(candidates.len(), 3);
    assert_eq!(
        candidates
            .iter()
            .find(|candidate| candidate.run_id == queued.id)
            .unwrap()
            .action,
        CadAgentRunRecoveryAction::Reenqueue
    );
    assert_eq!(
        candidates
            .iter()
            .find(|candidate| candidate.run_id == history.id)
            .unwrap()
            .action,
        CadAgentRunRecoveryAction::QueryHistory
    );
    service
        .mark_agent_run_reconciling(
            &history_session.session_id,
            &history.id,
            "startup history query".to_string(),
        )
        .unwrap();
    let history_unknown = service
        .mark_agent_run_unknown_outcome(
            &history_session.session_id,
            &history.id,
            "history request failed and no terminal outcome could be verified".to_string(),
        )
        .unwrap();
    assert_eq!(history_unknown.status, CadAgentRunStatus::Failed);
    assert_eq!(
        history_unknown.recovery_status,
        CadAgentRecoveryStatus::UnknownOutcome
    );
    assert_eq!(
        candidates
            .iter()
            .find(|candidate| candidate.run_id == unknown.id)
            .unwrap()
            .action,
        CadAgentRunRecoveryAction::MarkUnknownOutcome
    );
    let unknown = service
        .mark_agent_run_unknown_outcome(
            &unknown_session.session_id,
            &unknown.id,
            "turn/start acknowledgement was never observed".to_string(),
        )
        .unwrap();
    assert_eq!(unknown.status, CadAgentRunStatus::Failed);
    assert_eq!(
        unknown.recovery_status,
        CadAgentRecoveryStatus::UnknownOutcome
    );
}

#[test]
fn completed_history_recovery_backfills_once_and_terminal_replay_is_suppressed() {
    let service = test_service();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "recover".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let thread = test_thread(&created.session_id, "thread-recovery", "external-recovery");
    service.upsert_agent_thread(thread.clone()).unwrap();
    service
        .bind_agent_run_to_thread(
            &created.session_id,
            &run.id,
            &thread.id,
            Some("turn-recovery".to_string()),
            Some(1),
            CadAgentRecoveryStatus::None,
        )
        .unwrap();
    service
        .update_agent_run(
            &created.session_id,
            &run.id,
            Some(CadAgentRunStatus::Running),
            None,
            None,
            None,
            None,
        )
        .unwrap();
    service
        .mark_agent_run_reconciling(&created.session_id, &run.id, "restart".to_string())
        .unwrap();
    let input = CadAgentRunHistoryRecoveryInput {
        session_id: created.session_id.clone(),
        run_id: run.id.clone(),
        outcome: CadAgentRunHistoryOutcome::Completed {
            messages: vec![CadRecoveredAgentMessage {
                external_item_id: "item-final".to_string(),
                content: "Recovered final answer".to_string(),
                phase: Some(CadConversationPhase::FinalAnswer),
                sequence: Some(3),
                is_final: true,
                created_at: "2026-08-12T01:00:00.000Z".to_string(),
                metadata: None,
            }],
        },
    };
    let first = service
        .apply_agent_run_history_recovery(input.clone())
        .unwrap();
    assert_eq!(first.run.status, CadAgentRunStatus::Completed);
    assert_eq!(
        first.run.recovery_status,
        CadAgentRecoveryStatus::RecoveredFromHistory
    );
    assert_eq!(first.inserted_message_count, 1);
    assert!(first.terminal_event_created);
    let before = service.get_session_state(&created.session_id).unwrap();

    let replay = service.apply_agent_run_history_recovery(input).unwrap();
    let after = service.get_session_state(&created.session_id).unwrap();
    assert_eq!(replay.suppressed_message_count, 1);
    assert!(!replay.terminal_event_created);
    assert_eq!(after.conversation.len(), before.conversation.len());
    assert_eq!(after.agent_run_events.len(), before.agent_run_events.len());
}

#[test]
fn history_not_found_is_failed_unknown_outcome_and_never_reenqueueable() {
    let service = test_service();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "unknown".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let thread = test_thread(&created.session_id, "thread-unknown", "external-unknown");
    service.upsert_agent_thread(thread.clone()).unwrap();
    service
        .bind_agent_run_to_thread(
            &created.session_id,
            &run.id,
            &thread.id,
            Some("turn-missing".to_string()),
            Some(1),
            CadAgentRecoveryStatus::Reconciling,
        )
        .unwrap();
    let result = service
        .apply_agent_run_history_recovery(CadAgentRunHistoryRecoveryInput {
            session_id: created.session_id.clone(),
            run_id: run.id,
            outcome: CadAgentRunHistoryOutcome::NotFound,
        })
        .unwrap();
    assert_eq!(result.run.status, CadAgentRunStatus::Failed);
    assert_eq!(
        result.run.recovery_status,
        CadAgentRecoveryStatus::UnknownOutcome
    );
    assert!(service
        .list_startup_agent_run_recovery_candidates()
        .unwrap()
        .is_empty());
    let diagnostics = service
        .agent_session_diagnostics(&created.session_id)
        .unwrap();
    let diagnostic = diagnostics
        .threads
        .iter()
        .flat_map(|thread| &thread.runs)
        .find(|diagnostic| diagnostic.run_id == result.run.id)
        .unwrap();
    assert_eq!(
        diagnostic.recovery_status,
        CadAgentRecoveryStatus::UnknownOutcome
    );
    assert!(diagnostic
        .last_error
        .as_deref()
        .unwrap()
        .contains("not found"));
}

#[test]
fn thread_replacement_is_atomic_persists_reason_and_allows_only_named_recovery_run() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-thread-replace-test-{}", uuid()));
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
    let old = test_thread(&created.session_id, "old-thread", "external-old");
    service.upsert_agent_thread(old.clone()).unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "resume recovery".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let conflict_session = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    service
        .upsert_agent_thread(test_thread(
            &conflict_session.session_id,
            "conflict-thread",
            "external-conflict",
        ))
        .unwrap();
    let conflict_replacement = test_thread(
        &created.session_id,
        "failed-new-thread",
        "external-conflict",
    );
    assert!(service
        .replace_active_agent_thread(
            &old.id,
            conflict_replacement,
            "must roll back".to_string(),
            Some(&run.id),
        )
        .is_err());
    assert_eq!(
        service
            .get_active_agent_thread(
                &ThreadScope {
                    session_id: created.session_id.clone(),
                    plane: CadAgentPlane::Modeling,
                    owner_id: created.session_id.clone(),
                },
                "codex",
            )
            .unwrap()
            .unwrap()
            .id,
        old.id
    );
    assert_eq!(
        service.list_agent_threads(&created.session_id).unwrap(),
        vec![old.clone()]
    );
    let replacement = test_thread(&created.session_id, "new-thread", "external-new");
    assert!(service
        .replace_active_agent_thread(
            &old.id,
            replacement.clone(),
            "resume not found".to_string(),
            None,
        )
        .unwrap_err()
        .contains("active"));
    let replaced = service
        .replace_active_agent_thread(
            &old.id,
            replacement.clone(),
            "resume not found".to_string(),
            Some(&run.id),
        )
        .unwrap();
    assert_eq!(replaced.active_thread.id, replacement.id);
    assert_eq!(
        replaced.archived_thread.replaced_by_id.as_deref(),
        Some(replacement.id.as_str())
    );
    assert_eq!(
        replaced
            .archived_thread
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("replacedByReason"))
            .and_then(Value::as_str),
        Some("resume not found")
    );
    drop(service);

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let threads = reloaded.list_agent_threads(&created.session_id).unwrap();
    assert_eq!(threads.len(), 2);
    assert_eq!(
        reloaded
            .get_active_agent_thread(
                &ThreadScope {
                    session_id: created.session_id.clone(),
                    plane: CadAgentPlane::Modeling,
                    owner_id: created.session_id.clone(),
                },
                "codex",
            )
            .unwrap()
            .unwrap()
            .id,
        replacement.id
    );
}

#[test]
fn transport_cleanup_returns_exact_deleted_ids_and_diagnostics_link_graph() {
    let service = test_service();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    for sequence in 1..=3 {
        service
            .save_agent_transport_event(CadAgentTransportEvent {
                id: format!("event-{sequence}"),
                session_id: created.session_id.clone(),
                run_id: None,
                agent_thread_id: None,
                external_turn_id: None,
                external_item_id: None,
                method: "test/event".to_string(),
                sequence,
                payload: json!({"sequence": sequence}),
                created_at: format!("2026-08-12T00:00:0{sequence}.000Z"),
            })
            .unwrap();
    }
    let cleanup = service
        .cleanup_agent_transport_events(CadAgentTransportCleanupInput {
            session_id: Some(created.session_id.clone()),
            created_before: None,
            max_events_per_session: Some(1),
        })
        .unwrap();
    assert_eq!(cleanup.deleted_count, 2);
    assert_eq!(
        cleanup.deleted_event_ids,
        vec!["event-1".to_string(), "event-2".to_string()]
    );
    service
        .save_agent_transport_event(CadAgentTransportEvent {
            id: "expired-event".to_string(),
            session_id: created.session_id.clone(),
            run_id: None,
            agent_thread_id: None,
            external_turn_id: None,
            external_item_id: None,
            method: "test/expired".to_string(),
            sequence: 4,
            payload: json!({}),
            created_at: "2026-08-11T23:59:59.000Z".to_string(),
        })
        .unwrap();
    let expired = service
        .cleanup_agent_transport_events(CadAgentTransportCleanupInput {
            session_id: Some(created.session_id.clone()),
            created_before: Some("2026-08-12T00:00:00.000Z".to_string()),
            max_events_per_session: None,
        })
        .unwrap();
    assert_eq!(expired.deleted_count, 1);
    assert_eq!(expired.deleted_event_ids, vec!["expired-event".to_string()]);
    let diagnostics = service
        .agent_session_diagnostics(&created.session_id)
        .unwrap();
    assert_eq!(diagnostics.transport_event_count, 1);
}

#[test]
fn duplicate_archive_restore_and_delete_follow_external_thread_mapping_policy() {
    let service = test_service();
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let thread = test_thread(
        &created.session_id,
        "lifecycle-thread",
        "external-lifecycle",
    );
    service.upsert_agent_thread(thread.clone()).unwrap();

    let duplicated = service
        .duplicate_session(DuplicateCadSessionInput {
            session_id: created.session_id.clone(),
            title: None,
        })
        .unwrap();
    assert!(duplicated.state.agent_threads.is_empty());
    let archived = service
        .archive_session(ArchiveCadSessionInput {
            session_id: created.session_id.clone(),
            archived: Some(true),
        })
        .unwrap();
    assert_eq!(archived.agent_threads, vec![thread.clone()]);
    let restored = service
        .archive_session(ArchiveCadSessionInput {
            session_id: created.session_id.clone(),
            archived: Some(false),
        })
        .unwrap();
    assert!(restored.session.archived_at.is_none());
    assert_eq!(restored.agent_threads, vec![thread]);
    service.delete_session(&duplicated.session_id).unwrap();
    assert!(service
        .get_session_state(&duplicated.session_id)
        .unwrap_err()
        .contains("deleted"));
}
