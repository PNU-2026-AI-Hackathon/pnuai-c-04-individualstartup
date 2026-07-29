use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub type Metadata = Map<String, Value>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CadRuntimeKind {
    OpenscadWasm,
    CadqueryLocal,
    FreecadLocal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CadSourceLanguage {
    Openscad,
    Cadquery,
    FreecadPython,
    CadastropheIr,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadSessionStatus {
    Idle,
    Rendering,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CadArtifactKind {
    PreviewMesh,
    Stl,
    Metadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CadUserMessageChannel {
    WebUi,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadConversationRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadAgentRunStatus {
    Queued,
    Running,
    WaitingForUser,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum CadAgentRunEventType {
    #[serde(rename = "agent.run.created")]
    AgentRunCreated,
    #[serde(rename = "agent.run.updated")]
    AgentRunUpdated,
    #[serde(rename = "agent.message.created")]
    AgentMessageCreated,
    #[serde(rename = "agent.tool.started")]
    AgentToolStarted,
    #[serde(rename = "agent.tool.completed")]
    AgentToolCompleted,
    #[serde(rename = "agent.run.completed")]
    AgentRunCompleted,
    #[serde(rename = "agent.run.failed")]
    AgentRunFailed,
    #[serde(rename = "agent.run.cancelled")]
    AgentRunCancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum CadParameterValue {
    Number(f64),
    String(String),
    Boolean(bool),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadDiagnostic {
    pub severity: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadDiagnostics {
    pub ok: bool,
    pub elapsed_ms: u64,
    pub items: Vec<CadDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadParameter {
    pub name: String,
    pub value: CadParameterValue,
    #[serde(rename = "type")]
    pub parameter_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadMesh {
    pub vertices: Vec<f64>,
    pub normals: Vec<f64>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadArtifact {
    pub id: String,
    pub revision_id: String,
    pub kind: CadArtifactKind,
    pub format: String,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadPreviewResult {
    pub diagnostics: CadDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<CadMesh>,
    pub artifacts: Vec<CadArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadExportResult {
    pub diagnostics: CadDiagnostics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<CadArtifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadUserEvent {
    pub id: String,
    pub revision_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub created_at: String,
    pub payload: Metadata,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadUserMessage {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    pub channel: CadUserMessageChannel,
    pub message: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadConversationMessage {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub role: CadConversationRole,
    pub content: String,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRun {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_revision_id: Option<String>,
    pub status: CadAgentRunStatus,
    pub prompt: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRunEvent {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: CadAgentRunEventType,
    pub sequence: u64,
    pub created_at: String,
    pub payload: Metadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelPlanComponent {
    pub name: String,
    pub purpose: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_features: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelAspectRatio {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub tolerance: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelRuntimeConstraints {
    pub runtime: CadRuntimeKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbidden_features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_component_annotation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadModelPlan {
    pub schema_version: String,
    pub summary: String,
    pub main_component: CadModelPlanComponent,
    #[serde(default)]
    pub supporting_components: Vec<CadModelPlanComponent>,
    pub expected_aspect_ratio: CadModelAspectRatio,
    pub source_language: CadSourceLanguage,
    pub runtime_constraints: CadModelRuntimeConstraints,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowPlan {
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub plan: CadModelPlan,
    pub source_language: CadSourceLanguage,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowOuterIteration {
    pub id: String,
    pub run_id: String,
    pub iteration: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    pub structural_report: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlm_report: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_report: Option<Value>,
    pub passed: bool,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowPendingVlm {
    pub run_id: String,
    pub artifact_id: String,
    pub contract: Value,
    pub pass_threshold: f64,
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadWorkflowState {
    pub plans: Vec<CadWorkflowPlan>,
    pub outer_iterations: Vec<CadWorkflowOuterIteration>,
    pub pending_vlm: Vec<CadWorkflowPendingVlm>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRevisionRunLink {
    pub run_id: String,
    pub role: String,
    pub status: CadAgentRunStatus,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRevisionSummary {
    pub id: String,
    pub source_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision_id: Option<String>,
    pub source_language: CadSourceLanguage,
    pub created_at: String,
    pub diagnostics: CadDiagnostics,
    pub artifact_count: usize,
    pub run_links: Vec<CadRevisionRunLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRevision {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_from_revision_id: Option<String>,
    pub source_hash: String,
    pub source_language: CadSourceLanguage,
    pub source: String,
    pub parameters: Vec<CadParameter>,
    pub created_at: String,
    pub diagnostics: CadDiagnostics,
    pub artifact_count: usize,
    pub artifacts: Vec<CadArtifact>,
    pub user_events: Vec<CadUserEvent>,
    pub run_links: Vec<CadRevisionRunLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadSession {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_viewed_at: Option<String>,
    pub connected_ui_clients: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision_id: Option<String>,
    pub selected_runtime: CadRuntimeKind,
    pub status: CadSessionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recovery_diagnostics: Vec<CadDiagnostic>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
    pub revisions: Vec<CadRevisionSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadSessionState {
    pub session: CadSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_revision: Option<CadRevision>,
    pub messages: Vec<CadUserMessage>,
    pub conversation: Vec<CadConversationMessage>,
    pub agent_runs: Vec<CadAgentRun>,
    pub agent_run_events: Vec<CadAgentRunEvent>,
    pub workflow: CadWorkflowState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum CadBridgeEventType {
    #[serde(rename = "session.created")]
    SessionCreated,
    #[serde(rename = "session.updated")]
    SessionUpdated,
    #[serde(rename = "revision.created")]
    RevisionCreated,
    #[serde(rename = "revision.activated")]
    RevisionActivated,
    #[serde(rename = "revision.restored")]
    RevisionRestored,
    #[serde(rename = "preview.rendered")]
    PreviewRendered,
    #[serde(rename = "message.created")]
    MessageCreated,
    #[serde(rename = "artifact.exported")]
    ArtifactExported,
    #[serde(rename = "artifact.deleted")]
    ArtifactDeleted,
    #[serde(rename = "artifact.verified")]
    ArtifactVerified,
    #[serde(rename = "agent.run.created")]
    AgentRunCreated,
    #[serde(rename = "agent.run.updated")]
    AgentRunUpdated,
    #[serde(rename = "agent.message.created")]
    AgentMessageCreated,
    #[serde(rename = "agent.tool.started")]
    AgentToolStarted,
    #[serde(rename = "agent.tool.completed")]
    AgentToolCompleted,
    #[serde(rename = "agent.run.completed")]
    AgentRunCompleted,
    #[serde(rename = "agent.run.failed")]
    AgentRunFailed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadBridgeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: CadBridgeEventType,
    pub session_id: String,
    pub created_at: String,
    pub state: CadSessionState,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cad_model_plan_fixture_matches_rust_schema() {
        let plan: CadModelPlan = serde_json::from_str(include_str!(
            "../../fixtures/contracts/cad_model_plan.v1.json"
        ))
        .unwrap();

        assert_eq!(plan.schema_version, "cad_model_plan.v1");
        assert_eq!(plan.main_component.name, "wall_bracket");
        assert_eq!(plan.source_language, CadSourceLanguage::Openscad);
        assert_eq!(
            plan.runtime_constraints.runtime,
            CadRuntimeKind::OpenscadWasm
        );
        assert!(plan.expected_aspect_ratio.tolerance > 0.0);
    }

    #[test]
    fn workflow_state_fixture_matches_rust_schema() {
        let workflow: CadWorkflowState = serde_json::from_str(include_str!(
            "../../fixtures/contracts/workflow_state.v1.json"
        ))
        .unwrap();

        assert_eq!(workflow.plans.len(), 1);
        assert_eq!(workflow.outer_iterations.len(), 1);
        assert_eq!(workflow.pending_vlm.len(), 1);
        assert_eq!(workflow.plans[0].plan.schema_version, "cad_model_plan.v1");
        assert_eq!(
            workflow.outer_iterations[0]
                .failure_report
                .as_ref()
                .and_then(|report| report.get("contractType"))
                .and_then(Value::as_str),
            Some("cadastrophe.failure_report.v1")
        );
        assert_eq!(
            workflow.pending_vlm[0]
                .contract
                .get("contractType")
                .and_then(Value::as_str),
            Some("cadastrophe.vlm_judge.v1")
        );
    }

    #[test]
    fn report_contract_fixtures_keep_expected_discriminators() {
        for (fixture, contract_type) in [
            (
                include_str!("../../fixtures/contracts/structural_report.v1.json"),
                "cadastrophe.structural_report.v1",
            ),
            (
                include_str!("../../fixtures/contracts/vlm_judge_contract.v1.json"),
                "cadastrophe.vlm_judge.v1",
            ),
            (
                include_str!("../../fixtures/contracts/vlm_judge_report.v1.json"),
                "cadastrophe.vlm_judge_report.v1",
            ),
        ] {
            let value: Value = serde_json::from_str(fixture).unwrap();
            assert_eq!(
                value.get("contractType").and_then(Value::as_str),
                Some(contract_type)
            );
        }
    }
}
