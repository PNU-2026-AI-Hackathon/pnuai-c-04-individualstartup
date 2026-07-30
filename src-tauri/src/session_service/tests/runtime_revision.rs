use super::*;

#[test]
fn serializes_camel_case_state() {
    let service =
        SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let value = serde_json::to_value(created.state).unwrap();
    assert!(value["session"]["activeRevisionId"].is_string());
    assert!(value["activeRevision"]["sourceLanguage"].is_string());
}

#[test]
fn render_preview_uses_active_revision_source() {
    let service =
        SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let updated = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "radius = 6; // @param min=1 max=20 step=1 label=Radius\nsphere(r = radius);"
                .to_string(),
            parent_revision_id: created.state.session.active_revision_id.clone(),
            parameters: None,
        })
        .unwrap();

    let (preview, state) = service
        .render_preview(RenderPreviewInput {
            session_id: created.session_id,
            revision_id: Some(updated.revision_id),
        })
        .unwrap();

    assert!(preview.diagnostics.ok);
    let mesh = preview.mesh.unwrap();
    assert!(mesh.vertices.len() / 3 > 100);
    assert!(state
        .active_revision
        .unwrap()
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == CadArtifactKind::PreviewMesh));
}

#[test]
fn openscad_wasm_preview_and_export_share_boolean_stl_output_hash() {
    let service =
        SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let updated = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: r#"
$fn = 24;
difference() {
  union() {
    cube([12, 8, 4], center=true);
    rotate([0, 0, 30]) translate([0, 0, 3]) cylinder(h=4, r=3, center=true);
  }
  translate([0, 0, -4]) cylinder(h=12, r=1.5, center=true);
}
"#
            .to_string(),
            parent_revision_id: created.state.session.active_revision_id.clone(),
            parameters: None,
        })
        .unwrap();

    let (preview, state) = service
        .render_preview(RenderPreviewInput {
            session_id: created.session_id.clone(),
            revision_id: Some(updated.revision_id.clone()),
        })
        .unwrap();
    assert!(preview.diagnostics.ok);
    let preview_stl_hash = state
        .active_revision
        .as_ref()
        .unwrap()
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == CadArtifactKind::PreviewMesh)
        .and_then(|artifact| artifact.metadata.as_ref())
        .and_then(|metadata| metadata.get("stlSha256"))
        .and_then(Value::as_str)
        .unwrap()
        .to_string();

    let (export, exported_state) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id,
            revision_id: Some(updated.revision_id),
            format: "stl".to_string(),
        })
        .unwrap();
    assert!(export.diagnostics.ok);
    let export_stl_hash = export
        .artifact
        .as_ref()
        .and_then(|artifact| artifact.metadata.as_ref())
        .and_then(|metadata| metadata.get("stlSha256"))
        .and_then(Value::as_str)
        .unwrap();
    assert_eq!(preview_stl_hash, export_stl_hash);
    assert_eq!(
        exported_state
            .active_revision
            .as_ref()
            .unwrap()
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == CadArtifactKind::Stl)
            .count(),
        1
    );
}

#[test]
fn revision_switch_restore_and_parameters_use_immutable_snapshots() {
    let service =
        SessionService::new(std::env::temp_dir().join(format!("cadastrophe-test-{}", uuid())));
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let root_revision_id = created.state.session.active_revision_id.clone().unwrap();
    let parameterized = service
        .update_model_source(UpdateModelSourceInput {
            session_id: created.session_id.clone(),
            source_language: CadSourceLanguage::Openscad,
            source: "radius = 4; // @param min=1 max=20 step=1 label=Radius\nsphere(r = radius);"
                .to_string(),
            parent_revision_id: Some(root_revision_id.clone()),
            parameters: None,
        })
        .unwrap();
    let parameterized_revision_id = parameterized.revision_id.clone();

    let parameter_update = service
        .update_parameters(
            &created.session_id,
            metadata_from_value(json!({ "radius": 9 })),
        )
        .unwrap();
    let parameter_revision = parameter_update.active_revision.as_ref().unwrap();
    assert_ne!(parameter_revision.id, parameterized_revision_id);
    assert_eq!(
        parameter_revision.parent_revision_id.as_deref(),
        Some(parameterized_revision_id.as_str())
    );
    assert_eq!(
        service
            .get_session_state(&created.session_id)
            .unwrap()
            .session
            .revisions
            .len(),
        3
    );

    let switched = service
        .set_active_revision(SetActiveRevisionInput {
            session_id: created.session_id.clone(),
            revision_id: root_revision_id.clone(),
        })
        .unwrap();
    assert_eq!(
        switched.session.active_revision_id.as_deref(),
        Some(root_revision_id.as_str())
    );

    let restored = service
        .restore_revision(RestoreRevisionInput {
            session_id: created.session_id.clone(),
            revision_id: parameterized_revision_id.clone(),
        })
        .unwrap();
    let restored_revision = restored.state.active_revision.as_ref().unwrap();
    assert_eq!(
        restored_revision.parent_revision_id.as_deref(),
        Some(root_revision_id.as_str())
    );
    assert_eq!(
        restored_revision.restored_from_revision_id.as_deref(),
        Some(parameterized_revision_id.as_str())
    );
    assert_eq!(restored_revision.source_hash.len(), 64);
    assert_eq!(restored_revision.artifact_count, 0);
}

#[test]
fn export_artifact_uses_session_revision_artifact_layout() {
    let artifact_root = std::env::temp_dir()
        .join(format!("cadastrophe-test-{}", uuid()))
        .join("artifacts");
    let service = SessionService::new(artifact_root.clone());
    let created = service
        .create_session(CreateCadSessionInput::default())
        .unwrap();
    let revision_id = created
        .state
        .session
        .active_revision_id
        .clone()
        .expect("session has initial revision");

    let (export, _) = service
        .export_artifact(ExportArtifactInput {
            session_id: created.session_id.clone(),
            revision_id: Some(revision_id.clone()),
            format: "stl".to_string(),
        })
        .unwrap();

    let artifact = export.artifact.expect("artifact exported");
    let metadata = artifact.metadata.as_ref().expect("artifact metadata");
    let path = PathBuf::from(metadata["path"].as_str().expect("path metadata"));
    assert_eq!(
        path,
        artifact_root
            .join(&created.session_id)
            .join(&revision_id)
            .join(format!("{}.stl", artifact.id))
    );
    assert_eq!(
        metadata["relativePath"].as_str(),
        Some(
            PathBuf::from("artifacts")
                .join(&created.session_id)
                .join(&revision_id)
                .join(format!("{}.stl", artifact.id))
                .to_string_lossy()
                .as_ref()
        )
    );
    assert!(path.exists());
    assert_eq!(artifact.bytes, Some(fs::metadata(path).unwrap().len()));
    assert_eq!(
        metadata["sha256"].as_str().map(str::len),
        Some(64),
        "sha256 metadata should be stored as hex"
    );
}
