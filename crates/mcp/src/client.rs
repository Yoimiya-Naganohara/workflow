//! MCP Client connection manager.
//!
//! Manages connections to external MCP (Model Context Protocol) servers.
//! Each connection is established using [`rig::tool::rmcp::McpClientHandler`]
//! and automatically registers the server's tools with the workflow
//! [`rig::tool::server::ToolServerHandle`].

use std::collections::HashMap;
use std::sync::Arc;

use rig::tool::server::ToolServerHandle;
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde::Serialize;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::McpServerConfig;
use crate::error::McpError;

// ── Event callback ───────────────────────────────────────────

/// Events emitted by the MCP client manager.
#[derive(Debug, Clone)]
pub enum McpManagerEvent {
    Connected {
        server: String,
        tool_count: usize,
    },
    Disconnected {
        server: String,
    },
}

/// Callback invoked when MCP connections change.
pub type McpEventCallback = Arc<dyn Fn(McpManagerEvent) + Send + Sync>;

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
    /// Optional callback for connection lifecycle events.
    event_callback: Option<McpEventCallback>,
}

struct ActiveConnection {
    /// Keeps the MCP session alive.
    _running_service: RunningService<RoleClient, ()>,
    /// Peer for dispatching tool calls.
    peer: Peer<RoleClient>,
    /// Tool names available on this server (informational).
    tool_names: Vec<String>,
    _server_config: McpServerConfig,
}

impl McpClientManager {
    /// Create a new manager that will register tools with `tool_server_handle`.
    pub fn new(tool_server_handle: ToolServerHandle) -> Self {
        Self {
            tool_server_handle,
            connections: Arc::new(Mutex::new(HashMap::new())),
            event_callback: None,
        }
    }

    /// Create a new manager with an event callback for connection changes.
    pub fn with_callback(
        tool_server_handle: ToolServerHandle,
        event_callback: McpEventCallback,
    ) -> Self {
        Self {
            tool_server_handle,
            connections: Arc::new(Mutex::new(HashMap::new())),
            event_callback: Some(event_callback),
        }
    }

    fn emit(&self, event: McpManagerEvent) {
        if let Some(ref cb) = self.event_callback {
            cb(event);
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

    /// Connect to a single MCP server.
    ///
    /// Uses raw rmcp (`.serve()`) instead of rig's `McpClientHandler`,
    /// so no individual tools are registered — all tools are accessed
    /// through the single `call_mcp_tool` dispatch.
    pub async fn connect_one(&self, config: &McpServerConfig) -> Result<(), McpError> {
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

                // Use raw rmcp ().serve() — no auto-registration.
                ().serve(child)
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

        let peer = running_service.peer().clone();

        // Fetch tool names for informational display.
        let tool_names = peer
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
        let tool_count = tool_names.len();
        conns.insert(
            config.name.clone(),
            ActiveConnection {
                _running_service: running_service,
                peer,
                tool_names,
                _server_config: config.clone(),
            },
        );

        info!(
            server = %config.name,
            "Connected to MCP server"
        );

        self.emit(McpManagerEvent::Connected {
            server: config.name.clone(),
            tool_count,
        });

        Ok(())
    }

    /// Disconnect from an MCP server.
    pub async fn disconnect(&self, name: &str) -> Result<(), McpError> {
        let mut conns = self.connections.lock().await;
        if conns.remove(name).is_some() {
            info!(server = %name, "Disconnected from MCP server");

            self.emit(McpManagerEvent::Disconnected {
                server: name.to_string(),
            });

            Ok(())
        } else {
            Err(McpError::ServerNotFound(name.to_string()))
        }
    }

    /// Disconnect all MCP servers.
    pub async fn disconnect_all(&self) {
        let mut conns = self.connections.lock().await;
        conns.clear();
        info!("Disconnected from all MCP servers");
    }

    /// Get the peer for a connected server, for dispatching tool calls.
    pub async fn get_peer(&self, name: &str) -> Option<Peer<RoleClient>> {
        self.connections.lock().await.get(name).map(|c| c.peer.clone())
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
