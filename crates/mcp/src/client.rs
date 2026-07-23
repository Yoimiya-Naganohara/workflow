//! MCP Client connection manager.
//!
//! Manages connections to external MCP (Model Context Protocol) servers.
//! Each connection is established using [`rig::tool::rmcp::McpClientHandler`]
//! and automatically registers the server's tools with the workflow
//! [`rig::tool::server::ToolServerHandle`].

use std::collections::HashMap;
use std::sync::Arc;

use rig::tool::rmcp::McpClientHandler;
use rig::tool::server::ToolServerHandle;
use rmcp::model::ClientInfo;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::McpServerConfig;
use crate::error::McpError;

/// Resolve a command name to a `tokio::process::Command`, using the system
/// `PATH` resolver from `rmcp` (which handles Windows `.cmd`/`.exe` shims
/// that `tokio::process::Command` alone cannot find).
#[allow(unused_mut)]
fn resolve_command(name: &str) -> tokio::process::Command {
    // `which_command` resolves the full path via the `which` crate.
    // On Windows this is essential for .cmd / .exe shims like `npx.cmd`.
    let mut cmd = match rmcp::transport::which_command(name) {
        Ok(cmd) => cmd,
        Err(_) => {
            // Fallback: use the name as-is (absolute path, already has extension, etc.)
            tokio::process::Command::new(name)
        }
    };
    // On Windows, suppress the console window that would otherwise pop up
    // when spawning a child process.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Information about an active MCP connection.
#[derive(Debug, Clone, Serialize)]
pub struct McpConnectionInfo {
    /// Server name.
    pub name: String,
    /// Tool names registered by this server.
    pub tool_names: Vec<String>,
}

/// Manages lifecycle of MCP client connections.
pub struct McpClientManager {
    /// Handle to the workflow tool server — MCP tools are registered here.
    tool_server_handle: ToolServerHandle,
    /// Active connections indexed by server name.
    connections: Arc<Mutex<HashMap<String, ActiveConnection>>>,
}

struct ActiveConnection {
    /// Keeps the MCP session alive. Dropping this gracefully shuts down the transport.
    _running_service: RunningService<RoleClient, McpClientHandler>,
    /// Tool names registered by this server, so we can remove them on disconnect.
    tool_names: Vec<String>,
    _server_config: McpServerConfig,
}

impl McpClientManager {
    /// Create a new manager that will register tools with `tool_server_handle`.
    pub fn new(tool_server_handle: ToolServerHandle) -> Self {
        Self {
            tool_server_handle,
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return the underlying handle (for cloning, etc.).
    pub fn tool_server_handle(&self) -> &ToolServerHandle {
        &self.tool_server_handle
    }

    /// Connect to all configured MCP servers.
    ///
    /// Failed connections are logged as warnings but do not abort other
    /// connections — best-effort semantics.
    pub async fn connect_all(&self, configs: &[McpServerConfig]) {
        for config in configs {
            if let Err(e) = self.connect_one(config).await {
                warn!(
                    server = %config.name,
                    error = %e,
                    "Failed to connect to MCP server"
                );
            }
        }
    }

    /// Connect to a single MCP server and register its tools.
    pub async fn connect_one(&self, config: &McpServerConfig) -> Result<(), McpError> {
        let client_info = ClientInfo::default();

        let handler = McpClientHandler::new(client_info, self.tool_server_handle.clone());

        // Build the transport from the config.
        let running_service = match &config.transport {
            crate::config::McpTransport::Stdio {
                command,
                args,
                env,
            } => {
                let mut cmd = resolve_command(command);
                cmd.args(args);
                if let Some(env) = env {
                    for (key, value) in env {
                        cmd.env(key, value);
                    }
                }

                let (child, _stderr) = TokioChildProcess::builder(cmd)
                    .spawn()
                    .map_err(|source| McpError::Spawn {
                        server: config.name.clone(),
                        source,
                    })?;

                handler
                    .connect(child)
                    .await
                    .map_err(|e| McpError::connect(&config.name, e))?
            }
            #[allow(unreachable_patterns)]
            other => {
                return Err(McpError::UnsupportedTransport {
                    server: config.name.clone(),
                    transport: format!("{other:?}"),
                });
            }
        };

        // Fetch tool names so we can clean them up on disconnect.
        let tool_names = running_service
            .peer()
            .list_all_tools()
            .await
            .map_err(|e| McpError::ListTools {
                server: config.name.clone(),
                source: Box::new(e),
            })?
            .into_iter()
            .map(|t| t.name.to_string())
            .collect::<Vec<_>>();

        let mut conns = self.connections.lock().await;
        conns.insert(
            config.name.clone(),
            ActiveConnection {
                _running_service: running_service,
                tool_names,
                _server_config: config.clone(),
            },
        );

        info!(
            server = %config.name,
            "Connected to MCP server"
        );

        Ok(())
    }

    /// Disconnect from an MCP server, removing its tools from the tool server.
    pub async fn disconnect(&self, name: &str) -> Result<(), McpError> {
        let mut conns = self.connections.lock().await;
        if let Some(conn) = conns.remove(name) {
            // Remove all tools this server registered.
            for tool_name in &conn.tool_names {
                if let Err(e) = self.tool_server_handle.remove_tool(tool_name).await {
                    warn!(
                        server = %name,
                        tool = %tool_name,
                        error = %e,
                        "Failed to remove MCP tool"
                    );
                }
            }
            info!(
                server = %name,
                tool_count = conn.tool_names.len(),
                "Disconnected from MCP server"
            );
            Ok(())
        } else {
            Err(McpError::ServerNotFound(name.to_string()))
        }
    }

    /// Disconnect all MCP servers, removing their tools.
    pub async fn disconnect_all(&self) {
        let mut conns = self.connections.lock().await;
        for (_name, conn) in conns.drain() {
            for tool_name in &conn.tool_names {
                let _ = self.tool_server_handle.remove_tool(tool_name).await;
            }
        }
        info!("Disconnected from all MCP servers");
    }

    /// List all active MCP connections with their tool names.
    pub async fn list_connections(&self) -> Vec<McpConnectionInfo> {
        let conns = self.connections.lock().await;
        conns
            .iter()
            .map(|(name, conn)| McpConnectionInfo {
                name: name.clone(),
                tool_names: conn.tool_names.clone(),
            })
            .collect()
    }

    /// Check if a server connection is active.
    pub async fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().await.contains_key(name)
    }
}
