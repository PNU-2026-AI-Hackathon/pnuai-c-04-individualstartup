use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CadAgentRunRecoveryAction {
    Reenqueue,
    QueryHistory,
    MarkUnknownOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRunRecoveryCandidate {
    pub session_id: String,
    pub run_id: String,
    pub status: CadAgentRunStatus,
    pub recovery_status: CadAgentRecoveryStatus,
    pub action: CadAgentRunRecoveryAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadRecoveredAgentMessage {
    pub external_item_id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<CadConversationPhase>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    pub is_final: bool,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CadAgentRunHistoryOutcome {
    Completed {
        messages: Vec<CadRecoveredAgentMessage>,
    },
    Failed {
        error: String,
    },
    Interrupted {
        reason: String,
    },
    NotFound,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRunHistoryRecoveryInput {
    pub session_id: String,
    pub run_id: String,
    pub outcome: CadAgentRunHistoryOutcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRunRecoveryResult {
    pub run: CadAgentRun,
    pub inserted_message_count: usize,
    pub updated_message_count: usize,
    pub suppressed_message_count: usize,
    pub terminal_event_created: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentThreadReplacementPreparation {
    pub session_id: String,
    pub external_agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_thread: Option<CadAgentThread>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentThreadReplacementResult {
    pub archived_thread: CadAgentThread,
    pub active_thread: CadAgentThread,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartNewAgentConversationInput {
    pub session_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartNewAgentConversationResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_thread: Option<CadAgentThread>,
    pub active_thread: CadAgentThread,
    pub state: CadSessionState,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentRunDiagnostic {
    pub run_id: String,
    pub status: CadAgentRunStatus,
    pub recovery_status: CadAgentRecoveryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_generation: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentThreadDiagnostic {
    pub thread: CadAgentThread,
    pub runs: Vec<CadAgentRunDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentSessionDiagnostics {
    pub session_id: String,
    pub archived: bool,
    pub threads: Vec<CadAgentThreadDiagnostic>,
    pub unbound_runs: Vec<CadAgentRunDiagnostic>,
    pub transport_event_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentTransportCleanupInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_events_per_session: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CadAgentTransportCleanupResult {
    pub deleted_count: usize,
    pub deleted_event_ids: Vec<String>,
}
