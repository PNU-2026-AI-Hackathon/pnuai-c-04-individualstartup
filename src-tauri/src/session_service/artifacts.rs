use super::*;

impl SessionService {
    pub fn render_preview(
        &self,
        input: RenderPreviewInput,
    ) -> Result<(CadPreviewResult, CadSessionState), String> {
        {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = CadSessionStatus::Rendering;
            session.updated_at = timestamp();
            let snapshot = build_state(&state, &input.session_id)?;
            self.emit(
                CadBridgeEventType::SessionUpdated,
                &input.session_id,
                snapshot,
            );
        }

        let (revision_id, mesh, diagnostics, preview_artifact, stl_artifact) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone())
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let revision = require_revision(&state, &revision_id)?.clone();
            drop(state);
            let rendered =
                render_open_scad_wasm_node(&revision.source, self.storage_layout.app_data_dir())?;
            let diagnostics = rendered.diagnostics.clone();
            if !diagnostics.ok {
                (revision_id, None, diagnostics, None, None)
            } else {
                let mesh = rendered.mesh.clone().ok_or_else(|| {
                    "OpenSCAD WASM render did not return preview mesh.".to_string()
                })?;
                let metadata = Some(runtime_artifact_metadata(
                    &revision.source,
                    &revision.parameters,
                    &rendered,
                    "backend-preview",
                )?);
                let preview_artifact = self.write_artifact(
                    &revision_id,
                    CadArtifactKind::PreviewMesh,
                    "json",
                    &serde_json::to_string(&mesh).map_err(|error| error.to_string())?,
                    metadata.clone(),
                )?;
                let stl_bytes = rendered
                    .stl_base64
                    .as_ref()
                    .map(|contents| {
                        base64::engine::general_purpose::STANDARD
                            .decode(contents.as_bytes())
                            .map_err(|error| error.to_string())
                    })
                    .transpose()?
                    .ok_or_else(|| "OpenSCAD WASM render did not return STL bytes.".to_string())?;
                let stl_artifact = self.write_artifact_bytes(
                    &revision_id,
                    CadArtifactKind::Stl,
                    "stl",
                    &stl_bytes,
                    metadata,
                )?;
                (
                    revision_id,
                    Some(mesh),
                    diagnostics,
                    Some(preview_artifact),
                    Some(stl_artifact),
                )
            }
        };
        let state_snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &revision_id)?;
            revision.diagnostics = diagnostics.clone();
            revision
                .artifacts
                .retain(|candidate| candidate.kind != CadArtifactKind::PreviewMesh);
            if let Some(artifact) = preview_artifact.clone() {
                revision.artifacts.push(artifact);
            }
            if let Some(artifact) = stl_artifact.clone() {
                revision.artifacts.push(artifact);
            }
            revision.artifact_count = revision.artifacts.len();
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = if diagnostics.ok {
                CadSessionStatus::Idle
            } else {
                CadSessionStatus::Failed
            };
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::PreviewRendered,
            &input.session_id,
            state_snapshot.clone(),
        );
        Ok((
            CadPreviewResult {
                diagnostics,
                mesh,
                artifacts: preview_artifact.into_iter().chain(stl_artifact).collect(),
            },
            state_snapshot,
        ))
    }

    pub fn persist_runtime_artifact(
        &self,
        input: PersistRuntimeArtifactInput,
    ) -> Result<PersistRuntimeArtifactResult, String> {
        if !matches!(
            input.kind,
            CadArtifactKind::PreviewMesh | CadArtifactKind::Stl | CadArtifactKind::RenderImage
        ) {
            return Err(
                "Runtime artifact persistence supports preview-mesh, stl, and render-image only."
                    .to_string(),
            );
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(input.contents_base64.as_bytes())
            .map_err(|error| format!("Runtime artifact contents are not valid base64: {error}"))?;
        {
            let state = self.inner.lock().map_err(lock_error)?;
            validate_revision_session(&state, &input.session_id, &input.revision_id)?;
        }
        let artifact = self.write_artifact_bytes(
            &input.revision_id,
            input.kind.clone(),
            &input.format,
            &bytes,
            Some(Value::Object(input.metadata)),
        )?;
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision_mut(&mut state, &input.revision_id)?;
            if input.kind == CadArtifactKind::PreviewMesh {
                revision
                    .artifacts
                    .retain(|candidate| candidate.kind != CadArtifactKind::PreviewMesh);
            }
            revision.artifacts.push(artifact.clone());
            revision.artifact_count = revision.artifacts.len();
            revision.diagnostics = input.diagnostics.clone();
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.status = if input.diagnostics.ok {
                CadSessionStatus::Idle
            } else {
                CadSessionStatus::Failed
            };
            session.updated_at = timestamp();
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        let event_type = match input.kind {
            CadArtifactKind::PreviewMesh => CadBridgeEventType::PreviewRendered,
            _ => CadBridgeEventType::ArtifactExported,
        };
        self.emit(event_type, &input.session_id, snapshot.clone());
        Ok(PersistRuntimeArtifactResult {
            artifact,
            state: snapshot,
        })
    }
    pub fn export_artifact(
        &self,
        input: ExportArtifactInput,
    ) -> Result<(CadExportResult, CadSessionState), String> {
        let (revision_id, format) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let session = require_session(&state, &input.session_id)?;
            let revision_id = input
                .revision_id
                .clone()
                .or_else(|| session.active_revision_id.clone())
                .ok_or_else(|| "No active revision is available.".to_string())?;
            let revision = require_revision(&state, &revision_id)?;
            if revision.session_id != input.session_id {
                return Err(format!(
                    "CAD revision {revision_id} does not belong to session {}.",
                    input.session_id
                ));
            }
            (revision_id, input.format.clone())
        };

        let (diagnostics, artifact) = if format == "metadata" {
            let contents = serde_json::to_string_pretty(
                &json!({"revisionId": revision_id, "runtime": "openscad-wasm"}),
            )
            .map_err(|error| error.to_string())?;
            (
                ok_diagnostics(1),
                Some(self.write_artifact(
                    &revision_id,
                    CadArtifactKind::Metadata,
                    &format,
                    &contents,
                    None,
                )?),
            )
        } else {
            let artifact = {
                let state = self.inner.lock().map_err(lock_error)?;
                require_revision(&state, &revision_id)?
                    .artifacts
                    .iter()
                    .rev()
                    .find(|artifact| {
                        artifact.kind == CadArtifactKind::Stl && artifact.format == format
                    })
                    .cloned()
            };
            if artifact.is_some() {
                (ok_diagnostics(0), artifact)
            } else {
                let revision = {
                    let state = self.inner.lock().map_err(lock_error)?;
                    require_revision(&state, &revision_id)?.clone()
                };
                let rendered = render_open_scad_wasm_node(
                    &revision.source,
                    self.storage_layout.app_data_dir(),
                )?;
                if !rendered.diagnostics.ok {
                    (rendered.diagnostics, None)
                } else {
                    let stl_base64 = rendered.stl_base64.as_ref().ok_or_else(|| {
                        "OpenSCAD WASM render did not return STL bytes.".to_string()
                    })?;
                    let stl_bytes = base64::engine::general_purpose::STANDARD
                        .decode(stl_base64.as_bytes())
                        .map_err(|error| error.to_string())?;
                    (
                        rendered.diagnostics.clone(),
                        Some(self.write_artifact_bytes(
                            &revision_id,
                            CadArtifactKind::Stl,
                            &format,
                            &stl_bytes,
                            Some(runtime_artifact_metadata(
                                &revision.source,
                                &revision.parameters,
                                &rendered,
                                "backend-export",
                            )?),
                        )?),
                    )
                }
            }
        };
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            if let Some(artifact) = &artifact {
                let revision = require_revision_mut(&mut state, &revision_id)?;
                if !revision
                    .artifacts
                    .iter()
                    .any(|candidate| candidate.id == artifact.id)
                {
                    revision.artifacts.push(artifact.clone());
                }
                revision.artifact_count = revision.artifacts.len();
            }
            if let Some(revision) = state.revisions.get_mut(&revision_id) {
                revision.diagnostics = diagnostics.clone();
            }
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::ArtifactExported,
            &input.session_id,
            snapshot.clone(),
        );
        Ok((
            CadExportResult {
                diagnostics,
                artifact,
            },
            snapshot,
        ))
    }
}
