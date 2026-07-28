use crate::protocol::CadConversationRole;

#[derive(Clone, Debug)]
pub struct AgentAdapterRunInput {
    pub session_id: String,
    pub run_id: String,
    pub prompt: String,
    pub revision_id: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AgentAdapterEvent {
    MessageCreated {
        role: CadConversationRole,
        content: String,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    },
    ToolStarted {
        name: String,
    },
    ToolCompleted {
        name: String,
    },
    SourceUpdated {
        source_language: crate::protocol::CadSourceLanguage,
        source: String,
    },
}

#[async_trait::async_trait]
pub trait AgentAdapter: Send + Sync {
    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String>;
}
