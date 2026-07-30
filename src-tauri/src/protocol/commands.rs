use super::*;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateCadSessionInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_runtime: Option<CadRuntimeKind>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateCadSessionResult {
    pub session_id: String,
    pub ui_url: String,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BootCadSessionResult {
    pub session_id: String,
    pub ui_url: String,
    pub state: CadSessionState,
    pub is_first_run: bool,
    pub created_session: bool,
    pub should_use_example_session: bool,
    pub should_auto_render: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurrentCadSessionResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<CadSessionState>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListCadSessionsInput {
    #[serde(default)]
    pub include_archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadSessionListItem {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_viewed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub title_source: CadSessionTitleSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision: Option<CadRevisionSummary>,
    pub selected_runtime: CadRuntimeKind,
    pub status: CadSessionStatus,
    pub archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub revision_count: usize,
    pub artifact_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListCadSessionsResult {
    pub sessions: Vec<CadSessionListItem>,
    pub search_fields: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenameCadSessionInput {
    pub session_id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveCadSessionInput {
    pub session_id: String,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateCadSessionInput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCadSessionResult {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelSourceInput {
    pub session_id: String,
    pub source_language: CadSourceLanguage,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    #[serde(default)]
    pub parameters: Option<Vec<CadParameter>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateModelSourceResult {
    pub revision_id: String,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SetActiveRevisionInput {
    pub session_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRevisionInput {
    pub session_id: String,
    pub revision_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRevisionResult {
    pub revision_id: String,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenderPreviewInput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistRuntimeArtifactInput {
    pub session_id: String,
    pub revision_id: String,
    pub kind: CadArtifactKind,
    pub format: String,
    pub contents_base64: String,
    pub diagnostics: CadDiagnostics,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersistRuntimeArtifactResult {
    pub artifact: CadArtifact,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PostUserMessageInput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRunInput {
    pub session_id: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_of_run_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRunResult {
    pub message: CadConversationMessage,
    pub run: CadAgentRun,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtifactInput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub format: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteArtifactInput {
    pub session_id: String,
    pub artifact_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteArtifactResult {
    pub artifact_id: String,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OpenArtifactResult {
    pub artifact: CadArtifact,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RevealArtifactResult {
    pub artifact: CadArtifact,
    pub path: String,
    pub revealed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VerifyArtifactFilesInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VerifyArtifactFilesResult {
    pub checked_count: usize,
    pub missing_artifact_ids: Vec<String>,
    pub hash_mismatch_artifact_ids: Vec<String>,
    pub size_mismatch_artifact_ids: Vec<String>,
    pub corrupt_metadata_artifact_ids: Vec<String>,
    pub invalid_path_artifact_ids: Vec<String>,
    pub orphan_paths: Vec<String>,
    pub diagnostics: Vec<CadDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<CadSessionState>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOrphanArtifactsInput {
    #[serde(default)]
    pub dry_run: bool,
}

impl Default for CleanupOrphanArtifactsInput {
    fn default() -> Self {
        Self { dry_run: true }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOrphanArtifactsResult {
    pub checked_file_count: usize,
    pub orphan_paths: Vec<String>,
    pub deleted_paths: Vec<String>,
}
