use super::*;

const MINIMAL_ASCII_STL: &[u8] = br#"solid triangle
  facet normal 0 0 1
    outer loop
      vertex 0 0 0
      vertex 1 0 0
      vertex 0 1 0
    endloop
  endfacet
endsolid triangle
"#;

#[test]
fn stl_artifact_exports_to_a_named_external_path_and_reopens_byte_for_byte() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-external-stl-source-test-{}", uuid()));
    let export_dir =
        std::env::temp_dir().join(format!("cadgen-ax-external-stl-target-test-{}", uuid()));
    fs::create_dir_all(&export_dir).unwrap();
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
    let revision_id = create_test_revision(&service, &created.session_id, "cube([1, 1, 1]);");
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::Stl,
            "stl",
            MINIMAL_ASCII_STL,
            None,
        )
        .unwrap();
    let destination = export_dir.join("custom-model-name.stl");
    fs::write(&destination, b"previous export").unwrap();

    let result = service
        .export_artifact_file(ExportArtifactFileInput {
            artifact_id: artifact.id.clone(),
            path: destination.to_string_lossy().to_string(),
        })
        .unwrap();

    assert_eq!(result.artifact.id, artifact.id);
    assert_eq!(result.path, destination.to_string_lossy());
    assert_eq!(result.bytes, MINIMAL_ASCII_STL.len() as u64);
    assert_eq!(result.sha256, storage::sha256_hex(MINIMAL_ASCII_STL));
    assert_eq!(fs::read(&destination).unwrap(), MINIMAL_ASCII_STL);
}

#[test]
fn stl_file_export_rejects_invalid_destinations_and_corrupt_artifacts() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-external-stl-errors-test-{}", uuid()));
    let export_dir =
        std::env::temp_dir().join(format!("cadgen-ax-external-stl-errors-target-{}", uuid()));
    fs::create_dir_all(&export_dir).unwrap();
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
    let revision_id = create_test_revision(&service, &created.session_id, "sphere(1);");
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::Stl,
            "stl",
            MINIMAL_ASCII_STL,
            None,
        )
        .unwrap();

    let relative_error = service
        .export_artifact_file(ExportArtifactFileInput {
            artifact_id: artifact.id.clone(),
            path: "relative.stl".to_string(),
        })
        .expect_err("relative export path must fail");
    assert!(relative_error.contains("must be absolute"));

    let extension_error = service
        .export_artifact_file(ExportArtifactFileInput {
            artifact_id: artifact.id.clone(),
            path: export_dir.join("wrong.obj").to_string_lossy().to_string(),
        })
        .expect_err("non-STL extension must fail");
    assert!(extension_error.contains(".stl extension"));

    let internal_path = PathBuf::from(
        artifact
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("path"))
            .and_then(Value::as_str)
            .unwrap(),
    );
    fs::write(&internal_path, b"corrupt").unwrap();
    let corrupt_destination = export_dir.join("must-not-be-created.stl");
    let corrupt_error = service
        .export_artifact_file(ExportArtifactFileInput {
            artifact_id: artifact.id,
            path: corrupt_destination.to_string_lossy().to_string(),
        })
        .expect_err("corrupt artifact must fail before destination write");
    assert!(corrupt_error.contains("size mismatch"));
    assert!(!corrupt_destination.exists());
}

#[test]
fn sqlite_repository_restores_artifact_manifest_after_restart() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-artifact-repo-test-{}", uuid()));
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
    let revision_id = create_test_revision(&service, &created.session_id, "cube([4, 4, 4]);");
    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(revision_id),
            format: "stl".to_string(),
        })
        .unwrap();
    let artifact = export.artifact.unwrap();
    assert_eq!(
        artifact.revision_hash,
        storage::sha256_hex(b"cube([4, 4, 4]);")
    );
    assert!(artifact.profile_hash.is_none());
    let invalid_profile_hash = service
        .update_artifact_profile_hash(&created.session_id, &artifact.id, "invalid")
        .expect_err("invalid profile hash must fail fast");
    assert!(invalid_profile_hash.contains("lowercase SHA-256"));
    let profile_hash = storage::sha256_hex(b"dfm-profile-fixture");
    let artifact = service
        .update_artifact_profile_hash(&created.session_id, &artifact.id, &profile_hash)
        .unwrap();
    assert_eq!(
        artifact.profile_hash.as_deref(),
        Some(profile_hash.as_str())
    );
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
    assert_eq!(restored_artifact.revision_hash, artifact.revision_hash);
    assert_eq!(
        restored_artifact.profile_hash.as_deref(),
        Some(profile_hash.as_str())
    );
    assert_eq!(
        restored_artifact
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("profileHash"))
            .and_then(Value::as_str),
        Some(profile_hash.as_str())
    );
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
fn sqlite_repository_restores_gcode_lineage_from_profile_metadata() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-gcode-lineage-test-{}", uuid()));
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
    let source = "cube([11, 12, 13]);";
    let revision_id = create_test_revision(&service, &created.session_id, source);
    let profile_hash = storage::sha256_hex(b"profile.ini contents");
    let artifact = service
        .write_artifact_bytes(
            &revision_id,
            CadArtifactKind::Gcode,
            "gcode",
            b"G28\nM84\n",
            Some(serde_json::json!({"profileHash": profile_hash})),
        )
        .unwrap();
    assert_eq!(
        artifact.revision_hash,
        storage::sha256_hex(source.as_bytes())
    );
    assert_eq!(
        artifact.profile_hash.as_deref(),
        Some(profile_hash.as_str())
    );

    let reloaded = SessionService::with_repository(
        layout.clone(),
        Arc::new(SqliteSessionRepository::new(layout)),
    )
    .unwrap();
    let restored = reloaded
        .get_session_state(&created.session_id)
        .unwrap()
        .active_revision
        .unwrap()
        .artifacts
        .into_iter()
        .find(|candidate| candidate.id == artifact.id)
        .expect("gcode artifact lineage restored");
    assert_eq!(restored.kind, CadArtifactKind::Gcode);
    assert_eq!(restored.revision_hash, artifact.revision_hash);
    assert_eq!(restored.profile_hash, artifact.profile_hash);
}

#[test]
fn sqlite_repository_marks_missing_artifacts_on_startup_and_verify() {
    let app_data_dir =
        std::env::temp_dir().join(format!("cadgen-ax-artifact-missing-test-{}", uuid()));
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
            revision_id: Some(revision_id),
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
