use anyhow::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use wf_core::*;
use wf_llm::LlmProvider;
use wf_llm::embedding::EmbeddingService;
use wf_runtime::runtime::{AgentRuntime, AgentRuntimeConfig};
use wf_tui::tui::{AppState, Tui};

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

/// Register a global panic hook that logs panics to stderr before the
/// default abort behavior.  This ensures panics in spawned tokio tasks
/// (which would otherwise be silently caught by the runtime) are visible
/// in logs.
fn register_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let msg = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            format!("{:?}", panic_info.payload())
        };
        let location = panic_info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        eprintln!("PANIC [{}]: {}", location, msg);
        tracing::error!(target: "panic", "{}", msg);
        // Call the previous hook to preserve default behavior (abort).
        prev(panic_info);
    }));
}

/// Clean up all sandbox directories under ~/.workflow/sandbox/.
/// Called during shutdown to prevent filesystem leaks.
/// Only removes directories whose names are valid UUIDs (AgentIds).
fn cleanup_all_sandboxes() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let sandbox_root = std::path::PathBuf::from(home)
        .join(".workflow")
        .join("sandbox");
    if !sandbox_root.exists() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&sandbox_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Only remove directories whose name is a valid 32-char hex string (16-byte ID).
            if name.len() != 32 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!("Failed to remove sandbox {}: {}", name, e);
            }
        }
    }
}

/// Run the TUI with crash recovery and graceful shutdown.
async fn run_tui() -> Result<()> {
    register_panic_hook();

    // Register signal handler for graceful shutdown on Ctrl+C / SIGTERM.
    let (shutdown_tx, _) = tokio::sync::watch::channel(false);
    {
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => {
                    info!("Received SIGINT — initiating graceful shutdown...");
                    let _ = shutdown_tx.send(true);
                }
                Err(e) => {
                    error!("Failed to register SIGINT handler: {}", e);
                }
            }
        });
    }

    let state = Arc::new(RwLock::new(AppState::default()));

    // Create background task handles as Option so they survive across
    // potential retry loop iterations.
    // Background task handles — set inside the retry loop below.
    #[allow(unused_assignments)]
    let mut flush_handle: Option<tokio::task::JoinHandle<()>> = None;
    #[allow(unused_assignments)]
    let mut evict_handle: Option<tokio::task::JoinHandle<()>> = None;
    #[allow(unused_assignments)]
    let mut provider_evict_handle: Option<tokio::task::JoinHandle<()>> = None;

    // Wrap TUI initialization and run in a retry loop for crash recovery.
    // If the TUI panics or returns an error non-fatally, we attempt to restart.
    const MAX_RESTART_ATTEMPTS: u32 = 3;
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        if attempt > 1 {
            info!("TUI restart attempt {}/{}", attempt, MAX_RESTART_ATTEMPTS);
        }

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
        let embedding_service: Arc<dyn wf_llm::EmbeddingService> = Arc::new(svc);
        let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), embedding_service);
        let runtime = Arc::new(RwLock::new(runtime));

        {
            let mut state = state.write().await;
            state.ui.budget_total = DEFAULT_RUNTIME_BUDGET;
            state.ui.budget_used = 0;
            state.ui.permits_total = DEFAULT_MAX_AGENTS;
            state.ui.permits_available = DEFAULT_MAX_AGENTS;
            state.core.runtime = Some(runtime);
        }
        // Build tool server — extract deps from state to avoid wf-tui dep in wf-tools.
        {
            let s = state.read().await;
            let pool = s.core.agent_pool.clone();
            let tx = s.core.runtime_event_tx.clone();
            let agent_id = s.core.responsible_agent_id;
            drop(s);
            let tool_server = wf_tools::create_agent_tool_server(pool, tx, agent_id);
            let mut s = state.write().await;
            s.core.tool_server = tool_server;
        }

        // Try to restore agent pool + task graph from last checkpoint.
        // If successful, the agents and task DAG survive restarts.
        {
            let state = state.read().await;
            if let Some(rt) = &state.core.runtime {
                let pool = state.core.agent_pool.clone();
                let tg = rt.read().await.task_graph.clone();
                // Drop the read lock before awaiting (restore_checkpoint is async).
                drop(state);
                wf_runtime::checkpoint::restore_checkpoint(&pool, &tg).await;
            }
        }

        // Load persisted role memos into the agent pool (with dedup).
        {
            let state = state.write().await;
            let persisted_memos = wf_persistence::load_role_memos();
            if !persisted_memos.is_empty()
                && let Ok(mut pool) = state.core.agent_pool.try_write()
            {
                for (role, memos) in persisted_memos {
                    for entry in memos {
                        pool.write_role_memo(&role, entry);
                    }
                }
            }
        }

        // Background task: periodic experience pool flush (every 30 seconds).
        // Uses try_read to avoid deadlocks — if the lock is contended it
        // skips the cycle rather than blocking.
        let flush_state = state.clone();
        let mut flush_shutdown_rx = shutdown_tx.subscribe();
        flush_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = flush_shutdown_rx.changed() => {
                        break;
                    }
                }
                let s = flush_state.read().await;
                if let Some(runtime) = &s.core.runtime
                    && let Ok(rt) = runtime.try_read()
                    && let Err(e) = rt.flush_experience_pool()
                {
                    error!("Periodic flush failed: {}", e);
                }
            }
            info!("Flush background task stopped");
        }));

        // Background task: periodic agent pool eviction (every 5 minutes).
        let evict_state = state.clone();
        let mut evict_shutdown_rx = shutdown_tx.subscribe();
        evict_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = evict_shutdown_rx.changed() => {
                        break;
                    }
                }
                let s = evict_state.read().await;
                if let Ok(mut pool) = s.core.agent_pool.try_write() {
                    let count = pool.evict_stale(s.core.responsible_agent_id.as_ref());
                    if count > 0 {
                        info!("Evicted {} stale agent(s)", count);
                    }
                }
            }
            info!("Agent eviction background task stopped");
        }));

        // Background task: periodic provider client eviction (every 5 minutes).
        // Removes unhealthy provider clients so they are re-created on next use.
        let provider_state = state.clone();
        let mut provider_shutdown_rx = shutdown_tx.subscribe();
        provider_evict_handle = Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = provider_shutdown_rx.changed() => {
                        break;
                    }
                }
                let mut s = match provider_state.try_write() {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let before = s.core.provider_clients.len();
                s.core
                    .provider_clients
                    .retain(|_, client| client.is_healthy());
                let evicted = before - s.core.provider_clients.len();
                if evicted > 0 {
                    info!("Evicted {} unhealthy provider client(s)", evicted);
                }
            }
            info!("Provider eviction background task stopped");
        }));

        // ── Run the TUI event loop ──
        let tui_result = tui.run().await;

        match tui_result {
            Ok(()) => {
                // Normal exit (user quit)
                break;
            }
            Err(e) => {
                error!("TUI returned error: {}", e);
                if attempt >= MAX_RESTART_ATTEMPTS {
                    error!(
                        "Max restart attempts ({}) reached — exiting",
                        MAX_RESTART_ATTEMPTS
                    );
                    return Err(e);
                }
                // Small delay before restart to avoid busy-loop on persistent failures
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                // Reset state for restart (keep messages, clear transient state)
                {
                    let mut s = state.write().await;
                    s.popup_mode = wf_tui::tui::state::PopupMode::None;
                    s.ui.focus = wf_tui::tui::state::Focus::Input;
                    s.ui.input.clear();
                    s.ui.input_cursor = 0;
                    s.ui.active_chat_requests = 0;
                    s.ui.active_chat_abort = None;
                }
                continue;
            }
        }
    }

    // ── Graceful shutdown ──
    info!("Shutting down...");

    // Signal all background tasks to stop (with timeout).
    let _ = shutdown_tx.send(true);

    // Stop background tasks (signal-driven shutdown via watch channel above;
    // abort as fallback to guarantee they don't outlive the process).
    if let Some(h) = flush_handle {
        h.abort();
    }
    if let Some(h) = evict_handle {
        h.abort();
    }
    if let Some(h) = provider_evict_handle {
        h.abort();
    }

    // Flush experience pool on shutdown (best-effort).
    {
        let state = state.read().await;
        if let Some(runtime) = &state.core.runtime
            && let Ok(rt) = runtime.try_read()
            && let Err(e) = rt.flush_experience_pool()
        {
            error!("Failed to flush experience pool on shutdown: {}", e);
        }
    }

    // Clean up all sandbox directories.
    cleanup_all_sandboxes();

    info!("Shutdown complete");
    Ok(())
}

async fn run_cli() -> Result<()> {
    register_panic_hook();
    let provider = select_provider()?;
    let svc = EmbeddingService::new();
    let embedding_service: Arc<dyn wf_llm::EmbeddingService> = Arc::new(svc);
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
