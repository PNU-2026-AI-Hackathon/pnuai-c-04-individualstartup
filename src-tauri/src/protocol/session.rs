use super::*;

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
