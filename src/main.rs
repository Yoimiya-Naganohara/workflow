use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use workflow::embedding::EmbeddingService;
use workflow::llm::LlmProvider;
use workflow::runtime::{AgentRuntime, AgentRuntimeConfig};
use workflow::tui::{AppState, Tui};
use workflow::types::*;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.contains(&"--cli".to_string()) {
        tracing_subscriber::fmt::init();
        run_cli().await
    } else {
        run_tui().await
    }
}

async fn run_tui() -> Result<()> {
    let state = Arc::new(RwLock::new(AppState::default()));

    let mut tui = match Tui::new(state.clone()) {
        Ok(tui) => tui,
        Err(e) => {
            eprintln!("Failed to initialize TUI: {}", e);
            eprintln!("Make sure you are running in an interactive terminal.");
            return Err(e);
        }
    };

    let provider = select_provider()?;
    let svc = EmbeddingService::new(provider.clone());
    let embedding_service: Arc<dyn workflow::traits::EmbeddingService> = Arc::new(svc);
    let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), embedding_service);
    let runtime = Arc::new(RwLock::new(runtime));

    {
        let mut state = state.write().await;
        state.budget_total = 10000;
        state.budget_used = 0;
        state.permits_total = 10;
        state.permits_available = 10;
        state.runtime = Some(runtime);
    }

    tui.run().await?;

    Ok(())
}

async fn run_cli() -> Result<()> {
    let provider = select_provider()?;
    let svc = EmbeddingService::new(provider);
    let embedding_service: Arc<dyn workflow::traits::EmbeddingService> = Arc::new(svc);
    let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), embedding_service);

    info!("Holographic Self-Evolving Multi-Agent System v0.1.0");
    info!("Architecture: L-1/L0/L1/L2 Decision Pipeline");
    info!("Available permits: {}", runtime.available_permits());
    info!("Remaining budget: {}", runtime.remaining_budget());

    let task = "Implement a REST API for user management with CRUD operations";
    let role = "Senior Rust developer specializing in web services";
    let value = "Write secure, well-tested, maintainable code with proper error handling";

    info!("Processing spawn request...");
    info!("Task: {}", task);
    info!("Role: {}", role);

    match runtime.process_with_text(task, role, value, 1000, 0).await? {
        SpawnDecision::Approved(config) => {
            info!("Spawn APPROVED");
            info!("  Agent ID: {:?}", config.agent_id);
            info!("  Task ID: {:?}", config.task_id);
            info!("  Budget: {}", config.allocated_budget);
            info!("  Tools: {:064b}", config.allowed_tools);
        }
        SpawnDecision::Rejected(rejection) => {
            warn!("Spawn REJECTED: {:?}", rejection);
        }
    }

    info!("Remaining budget: {}", runtime.remaining_budget());
    info!("Pending suspended: {}", runtime.pending_suspended());

    Ok(())
}

fn select_provider() -> Result<Arc<LlmProvider>> {
    if std::env::var("OPENAI_API_KEY").is_ok() {
        info!("Using OpenAI provider");
        return Ok(Arc::new(LlmProvider::from_env()?));
    }

    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        info!("Using Anthropic provider");
        return Ok(Arc::new(LlmProvider::from_env()?));
    }

    warn!("No API key found, using OpenAI provider with test key");
    Ok(Arc::new(LlmProvider::OpenAi(
        rig::providers::openai::CompletionsClient::new("test-key")?,
    )))
}
