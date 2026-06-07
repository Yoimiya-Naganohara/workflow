//! MCP tool system using rig's ToolServer and ToolDyn infrastructure.
//!
//! Built-in tools are registered on a shared [`ToolServerHandle`] and
//! agents connect via `.tool_server_handle()`.

pub mod builtin;

pub use rig::tool::server::{ToolServer, ToolServerHandle};

/// Create a [`ToolServerHandle`] pre-loaded with all built-in tools.
///
/// The handle is cheaply cloneable and can be shared across agents.
pub fn create_tool_server() -> ToolServerHandle {
    builtin::register_tools(ToolServer::new()).run()
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
