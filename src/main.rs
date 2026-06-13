use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

use workflow::core::types::*;
use workflow::llm::LlmProvider;
use workflow::llm::embedding::EmbeddingService;
use workflow::runtime::{AgentRuntime, AgentRuntimeConfig};
use workflow::tui::{AppState, Tui};

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

    // Local embedding via fastembed (no API key needed, runs on CPU).
    let svc = EmbeddingService::new();
    let embedding_service: Arc<dyn workflow::llm::EmbeddingService> = Arc::new(svc);
    let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), embedding_service);
    let runtime = Arc::new(RwLock::new(runtime));

    {
        let state_handle = state.clone();
        let mut state = state.write().await;
        state.ui.budget_total = DEFAULT_RUNTIME_BUDGET;
        state.ui.budget_used = 0;
        state.ui.permits_total = DEFAULT_MAX_AGENTS;
        state.ui.permits_available = DEFAULT_MAX_AGENTS;
        state.core.runtime = Some(runtime);
        state.core.tool_server = workflow::tools::create_agent_tool_server(state_handle);
    }

    // Background task: periodic experience pool flush (every 30 seconds)
    let flush_state = state.clone();
    let flush_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let s = flush_state.read().await;
            if let Some(runtime) = &s.core.runtime {
                if let Ok(rt) = runtime.try_read() {
                    let _ = rt.flush_experience_pool();
                }
            }
        }
    });

    tui.run().await?;

    // Stop the background flush task
    flush_handle.abort();

    // Flush experience pool on shutdown
    {
        let state = state.read().await;
        if let Some(runtime) = &state.core.runtime {
            if let Ok(rt) = runtime.try_read() {
                let _ = rt.flush_experience_pool();
            }
        }
    }

    Ok(())
}

async fn run_cli() -> Result<()> {
    let provider = select_provider()?;
    let svc = EmbeddingService::new();
    let embedding_service: Arc<dyn workflow::llm::EmbeddingService> = Arc::new(svc);
    let mut runtime = AgentRuntime::new(AgentRuntimeConfig::default(), embedding_service);
    runtime.set_provider((*provider).clone());

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

    match runtime
        .process_with_text(task, role, value, 1000, 0, None, None)
        .await?
    {
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
