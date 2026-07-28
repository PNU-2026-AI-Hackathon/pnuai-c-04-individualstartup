use cadastrophe_lib::agent_adapter::{AgentAdapter, AgentAdapterEvent, AgentAdapterRunInput};
use cadastrophe_lib::codex_agent_adapter::CodexAgentAdapter;
use cadastrophe_lib::codex_process_client::{CodexProcessClient, CodexProcessConfig};
use cadastrophe_lib::protocol::CadSourceLanguage;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Create a simple OpenSCAD sphere with radius 6.".to_string());
    let command =
        std::env::var("CADASTROPHE_CODEX_COMMAND").unwrap_or_else(|_| "codex".to_string());
    let client = CodexProcessClient::new(CodexProcessConfig {
        command,
        request_timeout: Duration::from_secs(30),
    });

    if let Err(error) = run_smoke(&client, prompt).await {
        eprintln!("Codex adapter smoke failed: {error}");
        let _ = client.shutdown().await;
        std::process::exit(1);
    }
    let _ = client.shutdown().await;
}

async fn run_smoke(client: &CodexProcessClient, prompt: String) -> Result<(), String> {
    let adapter = CodexAgentAdapter::new(client.clone());
    let events = adapter
        .run(AgentAdapterRunInput {
            session_id: "codex-smoke-session".to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            prompt,
            revision_id: None,
        })
        .await?;
    let source = events.iter().find_map(|event| match event {
        AgentAdapterEvent::SourceUpdated {
            source_language,
            source,
        } if *source_language == CadSourceLanguage::Openscad => Some(source),
        _ => None,
    });
    let source =
        source.ok_or_else(|| "Codex adapter did not produce OpenSCAD source.".to_string())?;
    if source.trim().is_empty() {
        return Err("Codex adapter produced empty OpenSCAD source.".to_string());
    }
    println!("Codex adapter produced OpenSCAD source:\n{source}");
    if !events
        .iter()
        .any(|event| matches!(event, AgentAdapterEvent::MessageCreated { .. }))
    {
        return Err("Codex adapter did not produce an assistant message.".to_string());
    }
    Ok(())
}
