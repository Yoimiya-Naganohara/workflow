//! MCP tool system using rig's ToolServer and ToolDyn infrastructure.
//!
//! Built-in tools are registered on a shared [`ToolServerHandle`] and
//! agents connect via `.tool_server_handle()`.

pub mod agent;
pub mod builtin;
pub mod memo;
pub mod sandbox;

pub use rig::tool::server::{ToolServer, ToolServerHandle};

pub use memo::MemoToolDeps;

/// Create a [`ToolServerHandle`] pre-loaded with all built-in tools.
///
/// The handle is cheaply cloneable and can be shared across agents.
pub fn create_tool_server() -> ToolServerHandle {
    builtin::register_tools(ToolServer::new()).run()
}

/// Create a [`ToolServerHandle`] with built-ins plus workflow agent tools.
/// Create a [`ToolServerHandle`] with built-ins, agent tools, and memo tools.
///
/// The `state` is used for both the agent tools (spawn_agent) and to derive
/// the memo tool dependencies (agent pool, responsible agent ID).
pub fn create_agent_tool_server(
    state: std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>,
) -> ToolServerHandle {
    let server = builtin::register_tools(ToolServer::new());
    let server = agent::register_tools(server, state.clone());
    // Derive memo deps from the state
    let memo_deps = memo::MemoToolDeps::from_state(&state);
    memo::register_memo_tools(server, memo_deps).run()
}

/// Create a [`ToolServerHandle`] with sandbox-aware tools for a specific agent.
pub fn create_sandboxed_agent_tool_server(
    base_state: std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>,
    sandbox: Option<std::sync::Arc<crate::tools::sandbox::SandboxHandle>>,
) -> ToolServerHandle {
    let server = builtin::register_sandboxed_tools(ToolServer::new(), sandbox);
    let server = agent::register_tools(server, base_state.clone());
    let memo_deps = memo::MemoToolDeps::from_state(&base_state);
    memo::register_memo_tools(server, memo_deps).run()
}

/// Create a [`ToolServerHandle`] and register one extra tool.
pub fn create_tool_server_with<T>(extra: T) -> ToolServerHandle
where
    T: rig::tool::Tool + 'static,
{
    let mut server = builtin::register_tools(ToolServer::new());
    server = server.tool(extra);
    server.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_tool_server_returns_handle() {
        let handle = create_tool_server();
        // Handle should be cloneable (cheaply)
        let cloned = handle.clone();
        drop(cloned);
        // Handle is Send + Sync (verified in test below)
    }

    #[test]
    fn test_create_tool_server_is_send_sync() {
        let handle = create_tool_server();
        // Verify it can be sent across threads
        let result = std::thread::spawn(move || {
            let _h = handle;
            true
        })
        .join();
        assert!(result.unwrap());
    }

    #[test]
    fn test_tool_server_types_are_public() {
        // Verify the re-exports compile correctly
        let _server = ToolServer::new();
        // ToolServerHandle::new is not public, but we can create via run()
        let handle = builtin::register_tools(ToolServer::new()).run();
        let _cloned = handle.clone();
    }
}
