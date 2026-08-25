use super::*;

fn evaluation_create(
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    artifact_id: &str,
) -> CadValidationEvaluationCreate {
    CadValidationEvaluationCreate {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        revision_id: revision_id.to_string(),
        artifact_id: artifact_id.to_string(),
        kind: CadValidationEvaluationKind::Vlm,
        input_contract: json!({"contractType":"cadgen-ax.vlm_judge.v1"}),
        pass_threshold: 0.8,
    }
}

#[test]
fn validation_evaluation_and_raw_events_survive_restart() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-validation-repo-test-{}", uuid()));
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
            "model then validate".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let revision_id = create_test_revision(&service, &created.session_id, "cube([3,3,3]);");
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::RenderImage,
            "png",
            b"real-image-bytes",
            None,
        )
        .unwrap();
    let queued_artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::RenderImage,
            "png",
            b"second-real-image-bytes",
            None,
        )
        .unwrap();
    let queued = service
        .create_next_validation_evaluation(evaluation_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &queued_artifact.id,
        ))
        .unwrap();
    let evaluation = service
        .create_next_validation_evaluation(evaluation_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &artifact.id,
        ))
        .unwrap();
    let now = timestamp();
    let thread = service
        .upsert_agent_thread(CadAgentThread {
            id: uuid(),
            session_id: created.session_id.clone(),
            plane: CadAgentPlane::Validation,
            owner_id: evaluation.id.clone(),
            external_agent: "codex".to_string(),
            external_thread_id: "validation-external-thread".to_string(),
            status: CadAgentThreadStatus::Active,
            connection_generation: Some(4),
            created_at: now.clone(),
            updated_at: now,
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: None,
        })
        .unwrap();
    let running = service
        .bind_validation_evaluation(
            &created.session_id,
            &evaluation.id,
            &thread.id,
            "validation-turn-1",
        )
        .unwrap();
    assert_eq!(running.status, CadValidationEvaluationStatus::Running);
    let event = service
        .save_validation_evaluation_event(CadValidationEvaluationEvent {
            id: uuid(),
            session_id: created.session_id.clone(),
            evaluation_id: evaluation.id.clone(),
            evaluator_thread_id: thread.id.clone(),
            external_turn_id: Some("validation-turn-1".to_string()),
            external_item_id: Some("item-1".to_string()),
            method: "item/started".to_string(),
            sequence: 1,
            payload: json!({"method":"item/started","params":{"raw":true}}),
            created_at: timestamp(),
        })
        .unwrap();
    drop(service);

    let restarted = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let restored = restarted
        .get_validation_evaluation(&created.session_id, &evaluation.id)
        .unwrap()
        .expect("evaluation restored");
    assert_eq!(restored.status, CadValidationEvaluationStatus::Running);
    assert_eq!(restored.input_contract, evaluation.input_contract);
    assert_eq!(
        restarted
            .get_validation_evaluation(&created.session_id, &queued.id)
            .unwrap()
            .expect("queued evaluation restored")
            .status,
        CadValidationEvaluationStatus::Queued
    );
    assert_eq!(
        restarted
            .list_validation_evaluation_events(&created.session_id, &evaluation.id)
            .unwrap(),
        vec![event]
    );
    let state_evaluations = restarted
        .get_session_state(&created.session_id)
        .unwrap()
        .validation_evaluations;
    assert_eq!(state_evaluations.len(), 2);
    assert!(state_evaluations.contains(&restored));
}

#[test]
fn validation_attempts_reject_graph_mismatch_duplicates_and_immutable_changes() {
    let service = SessionService::new(
        std::env::temp_dir().join(format!("cadgen-ax-validation-service-test-{}", uuid())),
    );
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let (run, _) = service
        .create_agent_run(
            &created.session_id,
            "model then validate".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let revision_id = create_test_revision(&service, &created.session_id, "sphere(3);");
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::RenderImage,
            "png",
            b"image",
            None,
        )
        .unwrap();
    let mut evaluation = service
        .create_next_validation_evaluation(evaluation_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &artifact.id,
        ))
        .unwrap();
    evaluation.attempt = 2;
    assert!(service.update_validation_evaluation(evaluation).is_err());

    let other = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let mut bad_graph = evaluation_create(&created.session_id, &run.id, &revision_id, &artifact.id);
    bad_graph.session_id = other.session_id;
    assert!(service
        .create_next_validation_evaluation(bad_graph)
        .is_err());
}

#[test]
fn create_next_validation_evaluation_allocates_new_immutable_attempt_rows() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-validation-attempt-test-{}", uuid()));
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
            "retry validation".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let revision_id = create_test_revision(&service, &created.session_id, "cylinder(h=3,r=2);");
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::RenderImage,
            "png",
            b"attempt-image",
            None,
        )
        .unwrap();
    let input = CadValidationEvaluationCreate {
        session_id: created.session_id.clone(),
        run_id: run.id,
        revision_id,
        artifact_id: artifact.id,
        kind: CadValidationEvaluationKind::Vlm,
        input_contract: json!({"contractType":"cadgen-ax.vlm_judge.v1"}),
        pass_threshold: 0.8,
    };
    let first = service
        .create_next_validation_evaluation(input.clone())
        .unwrap();
    let second = service.create_next_validation_evaluation(input).unwrap();
    assert_eq!((first.attempt, second.attempt), (1, 2));
    assert_ne!(first.id, second.id);
    assert_eq!(first.input_contract["evaluationId"], first.id);
    assert_eq!(first.input_contract["attempt"], 1);
    assert_eq!(second.input_contract["evaluationId"], second.id);
    assert_eq!(second.input_contract["attempt"], 2);
    assert_eq!(
        service
            .list_validation_evaluations(&created.session_id)
            .unwrap()
            .len(),
        2
    );

    let conflicting_identity = CadValidationEvaluationCreate {
        session_id: created.session_id,
        run_id: second.run_id,
        revision_id: second.revision_id,
        artifact_id: second.artifact_id,
        kind: CadValidationEvaluationKind::Vlm,
        input_contract: json!({
            "contractType":"cadgen-ax.vlm_judge.v1",
            "evaluationId":"caller-must-not-predict-id"
        }),
        pass_threshold: 0.8,
    };
    assert!(service
        .create_next_validation_evaluation(conflicting_identity)
        .unwrap_err()
        .contains("evaluationId does not match"));
}
