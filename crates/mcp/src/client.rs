//! MCP Client connection manager.
//!
//! Manages connections to external MCP (Model Context Protocol) servers.
//! Each connection is established using [`rig::tool::rmcp::McpClientHandler`]
//! and automatically registers the server's tools with the workflow
//! [`rig::tool::server::ToolServerHandle`].

use std::collections::HashMap;
use std::sync::Arc;

use rig::tool::server::ToolServerHandle;
use rmcp::ServiceExt;
use rmcp::model::Tool;
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use serde::Serialize;
use tokio::sync::{Mutex, oneshot};
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
    /// A dangerous MCP tool needs user approval before execution.
    ToolNeedsApproval {
        request_id: String,
        server: String,
        tool: String,
        arguments: serde_json::Value,
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
    #[cfg(not(windows))]
    {}
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
    /// Pending approvals: request_id → oneshot sender for user decision.
    pending_approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
}

struct ActiveConnection {
    /// Keeps the MCP session alive.
    _running_service: RunningService<RoleClient, ()>,
    /// Peer for dispatching tool calls.
    peer: Peer<RoleClient>,
    /// Tool definitions from the server, including annotations like `destructive_hint`.
    tools: Vec<Tool>,
    server_config: McpServerConfig,
}

impl McpClientManager {
    /// Create a new manager that will register tools with `tool_server_handle`.
    pub fn new(tool_server_handle: ToolServerHandle) -> Self {
        Self {
            tool_server_handle,
            connections: Arc::new(Mutex::new(HashMap::new())),
            event_callback: None,
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
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
            pending_approvals: Arc::new(Mutex::new(HashMap::new())),
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

    /// Connect to all configured MCP servers in parallel.
    ///
    /// Failed connections are logged as warnings but do not abort other
    /// connections — best-effort semantics.
    pub async fn connect_all(&self, configs: &[McpServerConfig]) {
        let tasks: Vec<_> = configs
            .iter()
            .map(|config| {
                let config = config.clone();
                async move {
                    if let Err(e) = self.connect_one(&config).await {
                        warn!(
                            server = %config.name,
                            error = %e,
                            "Failed to connect to MCP server"
                        );
                    }
                }
            })
            .collect();
        futures::future::join_all(tasks).await;
    }

    /// Connect to a single MCP server.
    ///
    /// Idempotent: if the server is already connected, this is a no-op.
    /// Uses raw rmcp (`.serve()`) instead of rig's `McpClientHandler`,
    /// so no individual tools are registered — all tools are accessed
    /// through the single `call_mcp_tool` dispatch.
    pub async fn connect_one(&self, config: &McpServerConfig) -> Result<(), McpError> {
        // Idempotency: skip if already connected.
        {
            let conns = self.connections.lock().await;
            if conns.contains_key(&config.name) {
                info!(
                    server = %config.name,
                    "Already connected, skipping"
                );
                return Ok(());
            }
        }

        // Build the transport from the config.
        let running_service = match &config.transport {
            crate::config::McpTransport::Stdio { command, args, env } => {
                let mut cmd = resolve_command(command);
                cmd.args(args);
                if let Some(env) = env {
                    for (key, value) in env {
                        cmd.env(key, value);
                    }
                }

                let (child, _stderr) =
                    TokioChildProcess::builder(cmd)
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

        // Fetch full tool definitions (including annotations like `destructive_hint`).
        let tools = peer
            .list_all_tools()
            .await
            .map_err(|e| McpError::ListTools {
                server: config.name.clone(),
                source: Box::new(e),
            })?;

        let mut conns = self.connections.lock().await;
        let tool_count = tools.len();
        conns.insert(
            config.name.clone(),
            ActiveConnection {
                _running_service: running_service,
                peer,
                tools,
                server_config: config.clone(),
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

    /// Disconnect all MCP servers, shutting down each service gracefully.
    pub async fn disconnect_all(&self) {
        let mut conns = self.connections.lock().await;
        for (name, _) in conns.drain() {
            info!(server = %name, "Disconnected from MCP server");
        }
    }

    /// Get the peer for a connected server, for dispatching tool calls.
    pub async fn get_peer(&self, name: &str) -> Option<Peer<RoleClient>> {
        self.connections
            .lock()
            .await
            .get(name)
            .map(|c| c.peer.clone())
    }

    /// List all active MCP connections with their tool names.
    pub async fn list_connections(&self) -> Vec<McpConnectionInfo> {
        let conns = self.connections.lock().await;
        conns
            .iter()
            .map(|(name, conn)| McpConnectionInfo {
                name: name.clone(),
                tool_names: conn.tools.iter().map(|t| t.name.to_string()).collect(),
            })
            .collect()
    }

    /// Check if a server connection is active.
    pub async fn is_connected(&self, name: &str) -> bool {
        self.connections.lock().await.contains_key(name)
    }

    /// Resolve a pending approval with the user's decision.
    /// Returns false if the request_id is not found or already resolved.
    pub async fn resolve_approval(&self, request_id: &str, approved: bool) -> bool {
        let mut pending = self.pending_approvals.lock().await;
        if let Some(sender) = pending.remove(request_id) {
            sender.send(approved).ok();
            true
        } else {
            false
        }
    }

    /// Check if a tool on a connected server requires user approval.
    ///
    /// Priority:
    /// 1. If `dangerous_tools` is configured in `mcp_servers.json`, it takes
    ///    precedence — a list containing `"*"` marks all tools dangerous.
    /// 2. Otherwise, the server's [`ToolAnnotations.destructive_hint`] is used.
    ///    If unset, the MCP spec defaults it to `true`.
    pub async fn is_tool_dangerous(&self, server: &str, tool: &str) -> bool {
        let conns = self.connections.lock().await;
        let conn = match conns.get(server) {
            Some(c) => c,
            None => return false,
        };

        // User config takes precedence.
        if let Some(dangerous) = &conn.server_config.dangerous_tools {
            return dangerous.iter().any(|d| d == "*") || dangerous.iter().any(|d| d == tool);
        }

        // Fall back to the server's own annotation.
        conn.tools
            .iter()
            .find(|t| t.name.as_ref() == tool)
            .and_then(|t| t.annotations.as_ref())
            .map(|a| a.destructive_hint.unwrap_or(false))
            .unwrap_or(false)
    }

    /// Wait for user approval for a dangerous tool call.
    /// Returns true if approved, false if denied or timed out.
    pub async fn request_approval(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<bool, McpError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();

        self.pending_approvals
            .lock()
            .await
            .insert(request_id.clone(), tx);

        self.emit(McpManagerEvent::ToolNeedsApproval {
            request_id: request_id.clone(),
            server: server.to_string(),
            tool: tool.to_string(),
            arguments,
        });

        // Wait for user decision with a 5-minute timeout.
        match tokio::time::timeout(std::time::Duration::from_secs(300), rx).await {
            Ok(Ok(true)) => Ok(true),
            Ok(Ok(false)) => Ok(false),
            _ => {
                // Timeout or sender dropped — clean up and treat as denied.
                self.pending_approvals.lock().await.remove(&request_id);
                Ok(false)
            }
        }
    }
}
