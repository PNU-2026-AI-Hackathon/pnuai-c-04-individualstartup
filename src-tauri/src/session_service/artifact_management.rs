use super::*;

impl SessionService {
    pub fn read_artifact(&self, artifact_id: &str) -> Result<String, String> {
        let artifact = self.load_artifact_manifest(artifact_id)?;
        if artifact.deleted_at.is_some() {
            return Err("Artifact has been deleted.".to_string());
        }
        let path = self.artifact_manifest_path(&artifact)?;
        match fs::read_to_string(&path) {
            Ok(contents) => Ok(contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.mark_artifact_missing(artifact_id, Some(timestamp()))?;
                Err(format!("Artifact file is missing: {}", path.display()))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn open_artifact(&self, artifact_id: &str) -> Result<OpenArtifactResult, String> {
        let artifact = self.load_artifact_manifest(artifact_id)?;
        if artifact.deleted_at.is_some() {
            return Err("Artifact has been deleted.".to_string());
        }
        let path = self.artifact_manifest_path(&artifact)?;
        if !path.exists() {
            self.mark_artifact_missing(artifact_id, Some(timestamp()))?;
            return Err(format!("Artifact file is missing: {}", path.display()));
        }
        Ok(OpenArtifactResult {
            artifact,
            path: path.to_string_lossy().to_string(),
        })
    }

    pub fn delete_artifact(
        &self,
        input: DeleteArtifactInput,
    ) -> Result<DeleteArtifactResult, String> {
        let artifact = self.load_artifact_manifest(&input.artifact_id)?;
        if self.artifact_session_id(&artifact)? != input.session_id {
            return Err(format!(
                "Artifact {} does not belong to session {}.",
                input.artifact_id, input.session_id
            ));
        }
        let deleted_at = timestamp();
        let path = self.artifact_manifest_path(&artifact)?;
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        self.repository
            .mark_artifact_deleted(&input.artifact_id, &deleted_at)?;
        let snapshot = {
            let mut state = self.inner.lock().map_err(lock_error)?;
            state.artifacts.remove(&input.artifact_id);
            for revision in state
                .revisions
                .values_mut()
                .filter(|revision| revision.session_id == input.session_id)
            {
                revision
                    .artifacts
                    .retain(|artifact| artifact.id != input.artifact_id);
                revision.artifact_count = revision.artifacts.len();
            }
            let session = require_session_mut(&mut state, &input.session_id)?;
            session.updated_at = deleted_at;
            rebuild_revision_summaries(&mut state, &input.session_id);
            self.persist_session_graph(&state, &input.session_id)?;
            build_state(&state, &input.session_id)?
        };
        self.emit(
            CadBridgeEventType::ArtifactDeleted,
            &input.session_id,
            snapshot.clone(),
        );
        Ok(DeleteArtifactResult {
            artifact_id: input.artifact_id,
            state: snapshot,
        })
    }

    pub fn verify_artifact_files(
        &self,
        session_id: Option<String>,
    ) -> Result<VerifyArtifactFilesResult, String> {
        let result = self.verify_artifact_files_inner(session_id.as_deref())?;
        let state = match session_id {
            Some(session_id) => Some(self.get_session_state(&session_id)?),
            None => None,
        };
        if let Some(state) = &state {
            self.emit(
                CadBridgeEventType::ArtifactVerified,
                &state.session.id,
                state.clone(),
            );
        }
        Ok(VerifyArtifactFilesResult { state, ..result })
    }

    pub fn cleanup_orphan_artifacts(
        &self,
        input: CleanupOrphanArtifactsInput,
    ) -> Result<CleanupOrphanArtifactsResult, String> {
        let known_paths = {
            let state = self.inner.lock().map_err(lock_error)?;
            state
                .artifacts
                .values()
                .filter_map(|artifact| self.artifact_manifest_path(artifact).ok())
                .collect::<std::collections::HashSet<_>>()
        };
        let mut checked_file_count = 0;
        let mut orphan_paths = Vec::new();
        let mut deleted_paths = Vec::new();
        for file_path in collect_artifact_files(self.storage_layout.artifact_root())? {
            checked_file_count += 1;
            if known_paths.contains(&file_path) {
                continue;
            }
            let display_path = file_path.to_string_lossy().to_string();
            orphan_paths.push(display_path.clone());
            if !input.dry_run {
                fs::remove_file(&file_path).map_err(|error| error.to_string())?;
                deleted_paths.push(display_path);
            }
        }
        Ok(CleanupOrphanArtifactsResult {
            checked_file_count,
            orphan_paths,
            deleted_paths,
        })
    }

    pub(super) fn write_artifact(
        &self,
        revision_id: &str,
        kind: CadArtifactKind,
        format: &str,
        contents: &str,
        metadata: Option<Value>,
    ) -> Result<CadArtifact, String> {
        self.write_artifact_bytes(revision_id, kind, format, contents.as_bytes(), metadata)
    }

    pub(super) fn write_artifact_bytes(
        &self,
        revision_id: &str,
        kind: CadArtifactKind,
        format: &str,
        contents_bytes: &[u8],
        metadata: Option<Value>,
    ) -> Result<CadArtifact, String> {
        let id = uuid();
        let (session_id, path, relative_path) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let revision = require_revision(&state, revision_id)?;
            (
                revision.session_id.clone(),
                self.storage_layout
                    .artifact_path(&revision.session_id, revision_id, &id, format)
                    .map_err(|error| error.to_string())?,
                self.storage_layout
                    .artifact_relative_path(&revision.session_id, revision_id, &id, format)
                    .map_err(|error| error.to_string())?,
            )
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(&path, contents_bytes).map_err(|error| error.to_string())?;
        let sha256 = storage::sha256_hex(contents_bytes);
        let mut metadata_map = metadata.map(metadata_from_value).unwrap_or_default();
        metadata_map.insert(
            "path".to_string(),
            Value::String(path.to_string_lossy().to_string()),
        );
        metadata_map.insert(
            "relativePath".to_string(),
            Value::String(relative_path.to_string_lossy().to_string()),
        );
        metadata_map.insert("sha256".to_string(), Value::String(sha256));
        let artifact = CadArtifact {
            id: id.clone(),
            revision_id: revision_id.to_string(),
            kind,
            format: format.to_string(),
            uri: format!("tauri://artifact/{id}"),
            bytes: Some(contents_bytes.len() as u64),
            created_at: timestamp(),
            deleted_at: None,
            missing_at: None,
            metadata: Some(metadata_map),
        };
        self.repository
            .save_artifact_manifest(&session_id, &artifact)?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        state.artifacts.insert(id, artifact.clone());
        Ok(artifact)
    }
    fn load_artifact_manifest(&self, artifact_id: &str) -> Result<CadArtifact, String> {
        if let Some(artifact) = self.repository.load_artifact_manifest(artifact_id)? {
            return Ok(artifact);
        }
        let state = self.inner.lock().map_err(lock_error)?;
        state
            .artifacts
            .get(artifact_id)
            .cloned()
            .ok_or_else(|| "Artifact not found.".to_string())
    }

    fn artifact_manifest_path(&self, artifact: &CadArtifact) -> Result<PathBuf, String> {
        let metadata = artifact
            .metadata
            .as_ref()
            .ok_or_else(|| "Artifact metadata missing.".to_string())?;
        if let Some(relative_path) = metadata.get("relativePath").and_then(Value::as_str) {
            let relative_path = PathBuf::from(relative_path);
            validate_artifact_relative_path(&relative_path)?;
            return Ok(self.storage_layout.app_data_dir().join(relative_path));
        }
        let path = metadata
            .get("path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .ok_or_else(|| "Artifact path missing.".to_string())?;
        validate_artifact_absolute_path(&path, self.storage_layout.artifact_root())?;
        Ok(path)
    }

    fn artifact_session_id(&self, artifact: &CadArtifact) -> Result<String, String> {
        let state = self.inner.lock().map_err(lock_error)?;
        let revision = require_revision(&state, &artifact.revision_id)?;
        Ok(revision.session_id.clone())
    }

    fn mark_artifact_missing(
        &self,
        artifact_id: &str,
        missing_at: Option<String>,
    ) -> Result<(), String> {
        self.repository
            .set_artifact_missing_at(artifact_id, missing_at.as_deref())?;
        let mut state = self.inner.lock().map_err(lock_error)?;
        if let Some(artifact) = state.artifacts.get_mut(artifact_id) {
            artifact.missing_at = missing_at.clone();
        }
        for revision in state.revisions.values_mut() {
            for artifact in &mut revision.artifacts {
                if artifact.id == artifact_id {
                    artifact.missing_at = missing_at.clone();
                }
            }
        }
        Ok(())
    }

    pub(super) fn verify_artifact_files_inner(
        &self,
        session_id: Option<&str>,
    ) -> Result<VerifyArtifactFilesResult, String> {
        let (artifacts, recovery_diagnostics) = {
            let state = self.inner.lock().map_err(lock_error)?;
            let artifacts = state
                .artifacts
                .values()
                .filter(|artifact| artifact.deleted_at.is_none())
                .filter(|artifact| {
                    session_id.is_none_or(|session_id| {
                        state
                            .revisions
                            .get(&artifact.revision_id)
                            .is_some_and(|revision| revision.session_id == session_id)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut recovery_diagnostics = state
                .sessions
                .values()
                .filter(|session| {
                    session_id
                        .map(|session_id| session.id == session_id)
                        .unwrap_or(true)
                })
                .flat_map(|session| session.recovery_diagnostics.clone())
                .collect::<Vec<_>>();
            recovery_diagnostics.extend(
                state
                    .revisions
                    .values()
                    .filter(|revision| {
                        session_id
                            .map(|session_id| revision.session_id == session_id)
                            .unwrap_or(true)
                    })
                    .flat_map(|revision| {
                        revision
                            .diagnostics
                            .items
                            .iter()
                            .filter(|diagnostic| diagnostic.message.contains("persisted"))
                            .cloned()
                    }),
            );
            (artifacts, recovery_diagnostics)
        };
        let mut missing_artifact_ids = Vec::new();
        let mut hash_mismatch_artifact_ids = Vec::new();
        let mut size_mismatch_artifact_ids = Vec::new();
        let mut corrupt_metadata_artifact_ids = Vec::new();
        let mut invalid_path_artifact_ids = Vec::new();
        let mut diagnostics = recovery_diagnostics;
        let mut known_paths = std::collections::HashSet::new();

        for artifact in &artifacts {
            let metadata = artifact.metadata.as_ref();
            if metadata
                .and_then(|metadata| metadata.get("metadataRecovery"))
                .is_some()
            {
                corrupt_metadata_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "warning",
                    format!("Artifact {} has corrupt persisted metadata.", artifact.id),
                ));
            }

            let path = match self.artifact_manifest_path(artifact) {
                Ok(path) => path,
                Err(error) => {
                    invalid_path_artifact_ids.push(artifact.id.clone());
                    diagnostics.push(verify_diagnostic(
                        "error",
                        format!(
                            "Artifact {} has an invalid manifest path: {error}",
                            artifact.id
                        ),
                    ));
                    let missing_at = artifact.missing_at.clone().unwrap_or_else(timestamp);
                    if artifact.missing_at.as_deref() != Some(missing_at.as_str()) {
                        self.mark_artifact_missing(&artifact.id, Some(missing_at))?;
                    }
                    continue;
                }
            };
            known_paths.insert(path.clone());

            let missing_at = if path.exists() {
                None
            } else {
                let missing_at = artifact.missing_at.clone().unwrap_or_else(timestamp);
                missing_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} file is missing at {}.",
                        artifact.id,
                        path.display()
                    ),
                ));
                Some(missing_at)
            };
            if artifact.missing_at != missing_at {
                self.mark_artifact_missing(&artifact.id, missing_at)?;
            }
            if !path.exists() {
                continue;
            }

            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            if artifact.bytes != Some(bytes.len() as u64) {
                size_mismatch_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} size mismatch: manifest {:?}, file {} bytes.",
                        artifact.id,
                        artifact.bytes,
                        bytes.len()
                    ),
                ));
            }
            let actual_sha256 = storage::sha256_hex(&bytes);
            let expected_sha256 = metadata
                .and_then(|metadata| metadata.get("sha256"))
                .and_then(Value::as_str);
            if expected_sha256 != Some(actual_sha256.as_str()) {
                hash_mismatch_artifact_ids.push(artifact.id.clone());
                diagnostics.push(verify_diagnostic(
                    "error",
                    format!(
                        "Artifact {} sha256 does not match its manifest.",
                        artifact.id
                    ),
                ));
            }
        }
        let mut orphan_paths = Vec::new();
        for file_path in collect_artifact_files(self.storage_layout.artifact_root())? {
            if known_paths.contains(&file_path) {
                continue;
            }
            let path = file_path.to_string_lossy().to_string();
            diagnostics.push(verify_diagnostic(
                "warning",
                format!("Found artifact file without a SQLite manifest: {path}."),
            ));
            orphan_paths.push(path);
        }
        Ok(VerifyArtifactFilesResult {
            checked_count: artifacts.len(),
            missing_artifact_ids,
            hash_mismatch_artifact_ids,
            size_mismatch_artifact_ids,
            corrupt_metadata_artifact_ids,
            invalid_path_artifact_ids,
            orphan_paths,
            diagnostics,
            state: None,
        })
    }
}
