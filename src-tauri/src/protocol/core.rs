use super::*;

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
#[serde(rename_all = "snake_case")]
pub enum CadSessionTitleSource {
    Agent,
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CadArtifactKind {
    PreviewMesh,
    Stl,
    Gcode,
    Metadata,
    RenderImage,
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadAgentRecoveryStatus {
    #[default]
    None,
    Reconciling,
    Resumed,
    RecoveredFromHistory,
    OrphanedThread,
    UnknownOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadAgentThreadStatus {
    Starting,
    Ready,
    Active,
    NotLoaded,
    Failed,
    Archived,
    Replaced,
    Legacy,
    Orphaned,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadAgentPlane {
    Modeling,
    Validation,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadValidationEvaluationKind {
    Vlm,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadValidationEvaluationStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CadValidationCheckKind {
    Structural,
    Dfm,
    Vlm,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadValidationCheckStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadValidationBatchStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadConversationPhase {
    Commentary,
    FinalAnswer,
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
