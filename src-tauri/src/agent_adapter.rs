use crate::protocol::{CadConversationRole, CadSourceLanguage};

#[derive(Clone, Debug)]
pub struct AgentAdapterRunInput {
    pub session_id: String,
    pub run_id: String,
    pub prompt: String,
    pub revision_id: Option<String>,
    pub revision_source_language: Option<CadSourceLanguage>,
    pub revision_source: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AgentAdapterEvent {
    RunMetadata {
        external_agent: Option<String>,
        external_thread_id: Option<String>,
        external_turn_id: Option<String>,
    },
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
    fn external_agent(&self) -> &'static str {
        "unknown"
    }

    async fn run(&self, input: AgentAdapterRunInput) -> Result<Vec<AgentAdapterEvent>, String>;
}
