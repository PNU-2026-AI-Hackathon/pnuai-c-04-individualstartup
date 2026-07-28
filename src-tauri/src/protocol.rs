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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRevisionSummary {
    pub id: String,
    pub source_language: CadSourceLanguage,
    pub created_at: String,
    pub diagnostics: CadDiagnostics,
    pub artifact_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRevision {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision_id: Option<String>,
    pub source_language: CadSourceLanguage,
    pub source: String,
    pub parameters: Vec<CadParameter>,
    pub created_at: String,
    pub diagnostics: CadDiagnostics,
    pub artifact_count: usize,
    pub artifacts: Vec<CadArtifact>,
    pub user_events: Vec<CadUserEvent>,
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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum CadBridgeEventType {
    #[serde(rename = "session.created")]
    SessionCreated,
    #[serde(rename = "session.updated")]
    SessionUpdated,
    #[serde(rename = "revision.created")]
    RevisionCreated,
    #[serde(rename = "preview.rendered")]
    PreviewRendered,
    #[serde(rename = "message.created")]
    MessageCreated,
    #[serde(rename = "artifact.exported")]
    ArtifactExported,
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
pub struct RenderPreviewInput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
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
