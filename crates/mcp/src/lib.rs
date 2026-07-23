//! Workflow MCP integration.
//!
//! This crate manages connections to external **MCP** (Model Context Protocol)
//! servers, allowing the workflow runtime to import and use their tools.
//!
//! # Client mode (primary)
//!
//! MCP server definitions are read from `~/.workflow/mcp_servers.json`.
//! Each server is connected on startup using the appropriate transport
//! (stdio, SSE, Streamable HTTP), and its tools are registered with the
//! workflow [`rig::tool::server::ToolServerHandle`] so agents can use them.
//!
//! # Server mode (future)
//!
//! A future phase will add the ability to expose workflow's own tools as an
//! MCP server for external clients.

pub mod client;
pub mod config;
pub mod error;
pub mod tool;

// Server mode is a placeholder for future use.
#[cfg(feature = "server")]
pub mod server;

pub use client::{McpClientManager, McpConnectionInfo, McpEventCallback, McpManagerEvent};
pub use config::{McpConfigFile, McpConfigSource, McpServerConfig, McpTransport};
pub use error::McpError;
