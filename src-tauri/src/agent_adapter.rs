use crate::protocol::{CadConversationRole, CadSourceLanguage};
use std::path::PathBuf;
use std::sync::Arc;

pub type AgentAdapterEventSink = Arc<dyn Fn(AgentAdapterEvent) -> Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct AgentAdapterRunInput {
    pub session_id: String,
    pub run_id: String,
    pub app_data_dir: PathBuf,
    pub prompt: String,
    pub revision_id: Option<String>,
    pub revision_source_language: Option<CadSourceLanguage>,
    pub revision_source: Option<String>,
    pub latest_workflow_failure_report: Option<serde_json::Value>,
    pub event_sink: Option<AgentAdapterEventSink>,
}

impl std::fmt::Debug for AgentAdapterRunInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentAdapterRunInput")
            .field("session_id", &self.session_id)
            .field("run_id", &self.run_id)
            .field("app_data_dir", &self.app_data_dir)
            .field("prompt", &self.prompt)
            .field("revision_id", &self.revision_id)
            .field("revision_source_language", &self.revision_source_language)
            .field("revision_source", &self.revision_source)
            .field(
                "latest_workflow_failure_report",
                &self.latest_workflow_failure_report,
            )
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<event sink>"),
            )
            .finish()
    }
}

impl AgentAdapterRunInput {
    pub fn emit_event(
        &self,
        buffered_events: &mut Vec<AgentAdapterEvent>,
        event: AgentAdapterEvent,
    ) -> Result<(), String> {
        if let Some(event_sink) = &self.event_sink {
            event_sink(event)
        } else {
            buffered_events.push(event);
            Ok(())
        }
    }
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
    AgentMessageDelta {
        external_thread_id: String,
        external_turn_id: String,
        external_item_id: String,
        phase: crate::protocol::CadConversationPhase,
        delta: String,
        sequence: u64,
    },
    AgentMessageCompleted {
        external_thread_id: String,
        external_turn_id: String,
        external_item_id: String,
        phase: crate::protocol::CadConversationPhase,
        content: String,
        sequence: u64,
        is_final: bool,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    },
    TransportNotification {
        agent_thread_id: String,
        external_turn_id: String,
        external_item_id: Option<String>,
        method: String,
        sequence: u64,
        payload: serde_json::Value,
    },
    ToolStarted {
        name: String,
    },
    ToolCompleted {
        name: String,
    },
    Progress {
        label: String,
        message: Option<String>,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
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

    async fn interrupt_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        Err(format!(
            "Agent {} does not support interrupting run {session_id}/{run_id}.",
            self.external_agent()
        ))
    }

    async fn reconcile_run(&self, session_id: &str, run_id: &str) -> Result<(), String> {
        Err(format!(
            "Agent {} does not support history reconciliation for run {session_id}/{run_id}.",
            self.external_agent()
        ))
    }
}
