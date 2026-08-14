use super::*;

fn batch_create(
    session_id: &str,
    run_id: &str,
    revision_id: &str,
    artifact_id: &str,
) -> CadValidationBatchCreate {
    CadValidationBatchCreate {
        session_id: session_id.to_string(),
        run_id: run_id.to_string(),
        revision_id: revision_id.to_string(),
        artifact_id: artifact_id.to_string(),
        checks: vec![
            CadValidationCheckCreate {
                kind: CadValidationCheckKind::Structural,
                input_contract: json!({"validator":"structural"}),
            },
            CadValidationCheckCreate {
                kind: CadValidationCheckKind::Dfm,
                input_contract: json!({"validator":"dfm"}),
            },
            CadValidationCheckCreate {
                kind: CadValidationCheckKind::Vlm,
                input_contract: json!({"validator":"vlm"}),
            },
        ],
    }
}

#[test]
fn validation_batch_atomic_cas_restart_and_effect_claim_blocks_racing_attempt() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-validation-batch-test-{}", uuid()));
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
            "parallel validation".to_string(),
            None,
            Some("codex".to_string()),
            None,
        )
        .unwrap();
    let revision_id = create_test_revision(&service, &created.session_id, "cube([2,2,2]);");
    service
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::Stl,
            "stl",
            b"immutable-stl",
            None,
        )
        .unwrap();

    let (batch, checks) = service
        .create_validation_batch(batch_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &artifact.id,
        ))
        .unwrap();
    assert_eq!(batch.attempt, 1);
    assert_eq!(checks.len(), 3);
    for check in &checks {
        assert_eq!(check.input_contract["batchId"], batch.id);
        assert_eq!(check.input_contract["checkId"], check.id);
        assert_eq!(check.input_contract["attempt"], 1);
    }
    let structural = checks
        .iter()
        .find(|check| check.kind == CadValidationCheckKind::Structural)
        .unwrap();
    let dfm = checks
        .iter()
        .find(|check| check.kind == CadValidationCheckKind::Dfm)
        .unwrap();
    let vlm = checks
        .iter()
        .find(|check| check.kind == CadValidationCheckKind::Vlm)
        .unwrap();
    service
        .start_validation_check(&created.session_id, &structural.id)
        .unwrap();
    service
        .complete_validation_check(
            &created.session_id,
            &structural.id,
            json!({"ok":true}),
            true,
        )
        .unwrap();
    service
        .start_validation_check(&created.session_id, &dfm.id)
        .unwrap();
    service
        .complete_validation_check(&created.session_id, &dfm.id, json!({"ok":false}), false)
        .unwrap();
    let now = timestamp();
    let thread = service
        .upsert_agent_thread(CadAgentThread {
            id: uuid(),
            session_id: created.session_id.clone(),
            plane: CadAgentPlane::Validation,
            owner_id: vlm.id.clone(),
            external_agent: "codex".to_string(),
            external_thread_id: "batch-validation-thread".to_string(),
            status: CadAgentThreadStatus::Active,
            connection_generation: Some(1),
            created_at: now.clone(),
            updated_at: now,
            last_resumed_at: None,
            archived_at: None,
            replaced_by_id: None,
            metadata: None,
        })
        .unwrap();
    service
        .bind_validation_check(
            &created.session_id,
            &vlm.id,
            &thread.id,
            "batch-validation-turn",
        )
        .unwrap();
    service
        .complete_validation_check(&created.session_id, &vlm.id, json!({"ok":true}), true)
        .unwrap();

    let claimed = service
        .try_claim_validation_batch_settlement(&created.session_id, &batch.id)
        .unwrap()
        .expect("all checks are terminal");
    assert!(claimed.settlement_claimed_at.is_some());
    assert!(service
        .try_claim_validation_batch_settlement(&created.session_id, &batch.id)
        .unwrap()
        .is_none());
    drop(service);

    let restarted = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    restarted
        .normalize_validation_batches_after_process_restart()
        .unwrap();
    let recovered = restarted
        .get_validation_batch(&created.session_id, &batch.id)
        .unwrap()
        .unwrap();
    assert!(recovered.settlement_claimed_at.is_none());
    let reclaimed = restarted
        .try_claim_validation_batch_settlement(&created.session_id, &batch.id)
        .unwrap()
        .unwrap();
    assert!(reclaimed.settlement_claimed_at.is_some());
    let settled = restarted
        .settle_validation_batch(
            &created.session_id,
            &batch.id,
            reclaimed.settlement_claimed_at.as_deref().unwrap(),
            CadValidationBatchStatus::Succeeded,
            Some(json!({"passed":false})),
        )
        .unwrap();
    assert_eq!(settled.status, CadValidationBatchStatus::Succeeded);
    assert!(settled.effects_applied_at.is_none());
    let revision_b = create_test_revision(
        &restarted,
        &created.session_id,
        "translate([3, 0, 0]) cube([2,2,2]);",
    );
    let first_effect_claim = restarted
        .try_claim_validation_batch_effects(&created.session_id, &batch.id)
        .unwrap()
        .unwrap();
    let contender = SessionService::with_repository_without_startup_verification(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let blocked = contender
        .create_validation_batch(batch_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &artifact.id,
        ))
        .unwrap_err();
    assert!(blocked.contains("Validation effects are currently owned"));
    let blocked_revision_link = contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_b.clone())
        .unwrap_err();
    assert!(blocked_revision_link
        .contains("agent run output revision is locked by validation effects owner"));
    restarted
        .release_validation_batch_effects(&created.session_id, &batch.id)
        .unwrap();
    contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_b.clone())
        .unwrap();
    contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();

    let effect_claim = restarted
        .try_claim_validation_batch_effects(&created.session_id, &batch.id)
        .unwrap()
        .unwrap();
    assert_ne!(
        first_effect_claim.effects_claimed_at,
        effect_claim.effects_claimed_at
    );
    let blocked_revision_link = contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_b.clone())
        .unwrap_err();
    assert!(blocked_revision_link
        .contains("agent run output revision is locked by validation effects owner"));
    assert!(restarted
        .mark_validation_batch_effects_applied(
            &created.session_id,
            &batch.id,
            effect_claim.effects_claimed_at.as_deref().unwrap(),
        )
        .unwrap()
        .effects_applied_at
        .is_some());
    assert!(restarted
        .mark_validation_batch_effects_applied(
            &created.session_id,
            &batch.id,
            effect_claim.effects_claimed_at.as_deref().unwrap(),
        )
        .is_err());
    contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_b)
        .unwrap();
    contender
        .link_agent_run_output_revision(&created.session_id, &run.id, revision_id.clone())
        .unwrap();

    let (recovery_batch, recovery_checks) = restarted
        .create_validation_batch(batch_create(
            &created.session_id,
            &run.id,
            &revision_id,
            &artifact.id,
        ))
        .unwrap();
    assert_eq!(recovery_batch.attempt, 2);
    let recovery_structural = recovery_checks
        .iter()
        .find(|check| check.kind == CadValidationCheckKind::Structural)
        .unwrap();
    restarted
        .start_validation_check(&created.session_id, &recovery_structural.id)
        .unwrap();
    let recovery_structural_id = recovery_structural.id.clone();
    drop(restarted);

    let final_restart = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    assert_eq!(
        final_restart
            .list_validation_checks(&created.session_id, &batch.id)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        final_restart
            .get_validation_check(&created.session_id, &recovery_structural_id)
            .unwrap()
            .unwrap()
            .status,
        CadValidationCheckStatus::Queued
    );
    assert!(final_restart
        .get_validation_batch(&created.session_id, &batch.id)
        .unwrap()
        .unwrap()
        .effects_applied_at
        .is_some());
}

#[test]
fn validation_batch_rejects_missing_or_duplicate_kinds() {
    let service = SessionService::new(std::env::temp_dir().join(format!(
        "cadastrophe-validation-batch-shape-test-{}",
        uuid()
    )));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let mut input = CadValidationBatchCreate {
        session_id: created.session_id,
        run_id: "run".to_string(),
        revision_id: "revision".to_string(),
        artifact_id: "artifact".to_string(),
        checks: vec![],
    };
    assert!(service.create_validation_batch(input.clone()).is_err());
    input.checks = vec![
        CadValidationCheckCreate {
            kind: CadValidationCheckKind::Structural,
            input_contract: json!({}),
        },
        CadValidationCheckCreate {
            kind: CadValidationCheckKind::Structural,
            input_contract: json!({}),
        },
        CadValidationCheckCreate {
            kind: CadValidationCheckKind::Vlm,
            input_contract: json!({}),
        },
    ];
    assert!(service.create_validation_batch(input).is_err());
}
