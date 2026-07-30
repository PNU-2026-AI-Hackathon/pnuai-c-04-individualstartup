use super::*;

#[test]
fn sqlite_repository_restores_artifact_manifest_after_restart() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-artifact-repo-test-{}", uuid()));
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
    service.mark_session_viewed(&created.session_id).unwrap();
    let revision_id = created.state.session.active_revision_id.clone().unwrap();
    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(revision_id),
            format: "stl".to_string(),
        })
        .unwrap();
    let artifact = export.artifact.unwrap();
    let original_contents = service.read_artifact(&artifact.id).unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout.clone())),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    let restored_artifact = state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .find(|candidate| candidate.id == artifact.id)
        .expect("artifact manifest restored");
    assert_eq!(restored_artifact.kind, CadArtifactKind::Stl);
    assert_eq!(
        reloaded.read_artifact(&artifact.id).unwrap(),
        original_contents
    );
    let opened = reloaded.open_artifact(&artifact.id).unwrap();
    assert!(PathBuf::from(&opened.path).exists());

    let deleted = reloaded
        .delete_artifact(DeleteArtifactInput {
            session_id: created.session_id.clone(),
            artifact_id: artifact.id.clone(),
        })
        .unwrap();
    assert!(!PathBuf::from(opened.path).exists());
    assert!(!deleted
        .state
        .active_revision
        .unwrap()
        .artifacts
        .iter()
        .any(|candidate| candidate.id == artifact.id));

    let reloaded_after_delete = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state_after_delete = reloaded_after_delete
        .get_session_state(&created.session_id)
        .unwrap();
    assert!(!state_after_delete
        .active_revision
        .unwrap()
        .artifacts
        .iter()
        .any(|candidate| candidate.id == artifact.id));
    assert!(reloaded_after_delete.read_artifact(&artifact.id).is_err());
}

#[test]
fn sqlite_repository_marks_missing_artifacts_on_startup_and_verify() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadastrophe-artifact-missing-test-{}", uuid()));
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
    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: created.state.session.active_revision_id.clone(),
            format: "metadata".to_string(),
        })
        .unwrap();
    let artifact = export.artifact.unwrap();
    let path = service.open_artifact(&artifact.id).unwrap().path;
    fs::remove_file(&path).unwrap();

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let state = reloaded.get_session_state(&created.session_id).unwrap();
    let missing_artifact = state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .find(|candidate| candidate.id == artifact.id)
        .expect("missing artifact remains visible");
    assert!(missing_artifact.missing_at.is_some());
    assert!(reloaded.read_artifact(&artifact.id).is_err());

    let verified = reloaded
        .verify_artifact_files(Some(created.session_id.clone()))
        .unwrap();
    assert_eq!(verified.checked_count, 1);
    assert_eq!(verified.missing_artifact_ids, vec![artifact.id]);
    assert!(verified
        .state
        .unwrap()
        .active_revision
        .unwrap()
        .artifacts
        .iter()
        .any(|candidate| candidate.missing_at.is_some()));
}
