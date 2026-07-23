//! MCP management tools — callable by agents at runtime.

use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use rmcp::model::CallToolRequestParams;
use serde::{Deserialize, Serialize};

use crate::client::McpClientManager;
use crate::config::{McpConfigSource, McpServerConfig, McpTransport};
use crate::error::McpError;

/// Shared cell type used by all MCP tools to resolve the manager.
pub type ManagerCell = Arc<std::sync::Mutex<Option<Arc<McpClientManager>>>>;

/// Resolve the manager from the shared cell.
fn resolve_manager(cell: &ManagerCell) -> Result<Arc<McpClientManager>, McpError> {
    cell.lock()
        .map_err(|e| McpError::Other(format!("lock poisoned: {e}")))?
        .clone()
        .ok_or_else(|| McpError::Other("runtime not fully initialized".to_string()))
}

// ── InstallMcpServer ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct InstallMcpServerArgs {
    /// A unique name for this MCP server.
    pub name: String,
    /// The command to run (e.g. "npx", "uvx", "node").
    pub command: String,
    /// Arguments to pass.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional environment variables.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct InstallMcpServerOutput {
    pub server: String,
    pub status: String,
    pub message: String,
}

/// Tool that installs (configures + connects) an MCP server at runtime.
pub struct InstallMcpServer {
    manager_cell: ManagerCell,
    config_source: McpConfigSource,
}

impl InstallMcpServer {
    /// Create a new tool. The manager cell is populated later by the runtime.
    pub fn new(
        manager_cell: Arc<std::sync::Mutex<Option<Arc<McpClientManager>>>>,
    ) -> Self {
        Self {
            manager_cell,
            config_source: McpConfigSource::new(McpConfigSource::default_path()),
        }
    }

    fn manager(&self) -> Result<Arc<McpClientManager>, McpError> {
        resolve_manager(&self.manager_cell)
    }
}

impl Tool for InstallMcpServer {
    const NAME: &'static str = "install_mcp_server";

    type Error = McpError;
    type Args = InstallMcpServerArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "install_mcp_server".to_string(),
            description: "Install and connect a new MCP (Model Context Protocol) server. \
                Provide a name, the command to run, any arguments, and optional \
                environment variables. The server is persisted to \
                ~/.workflow/mcp_servers.json and connected immediately. \
                All its tools are accessible through the single `call_mcp_tool` dispatch."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique name (e.g. 'filesystem', 'playwright')"
                    },
                    "command": {
                        "type": "string",
                        "description": "Command to execute (e.g. 'npx', 'uvx', 'node')"
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments for the command"
                    },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional environment variables"
                    }
                },
                "required": ["name", "command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let transport = McpTransport::Stdio {
            command: args.command,
            args: args.args,
            env: if args.env.is_empty() { None } else { Some(args.env) },
        };

        let config = McpServerConfig {
            name: args.name.clone(),
            transport,
        };

        // 1. Persist to config file.
        self.config_source.add_server(config.clone())?;

        // 2. Connect to the server.
        let manager = self.manager()?;
        match manager.connect_one(&config).await {
            Ok(()) => Ok(serde_json::to_string(&InstallMcpServerOutput {
                server: args.name,
                status: "connected".to_string(),
                message: "MCP server installed and connected successfully".to_string(),
            })
            .unwrap_or_else(|_| "connected".to_string())),
            Err(e) => Ok(serde_json::to_string(&InstallMcpServerOutput {
                server: args.name,
                status: "error".to_string(),
                message: format!("configured but failed to connect: {e}"),
            })
            .unwrap_or_else(|_| format!("configured but connect failed: {e}"))),
        }
    }
}

// ── ListMcpServers ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListMcpServersArgs {}

pub struct ListMcpServers {
    manager_cell: ManagerCell,
}

impl ListMcpServers {
    pub fn new(manager_cell: ManagerCell) -> Self {
        Self { manager_cell }
    }
}

impl Tool for ListMcpServers {
    const NAME: &'static str = "list_mcp_servers";

    type Error = McpError;
    type Args = ListMcpServersArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_mcp_servers".to_string(),
            description: "List all connected MCP servers and their available tools. \
                Use this to find the server name and tool names needed for \
                `call_mcp_tool`. Tools are NOT registered individually — \
                use `call_mcp_tool` to invoke any tool on any server."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let manager = resolve_manager(&self.manager_cell)?;
        let conns = manager.list_connections().await;
        Ok(serde_json::to_string(&conns).unwrap_or_else(|_| "[]".to_string()))
    }
}

// ── RemoveMcpServer ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RemoveMcpServerArgs {
    /// Name of the MCP server to disconnect and remove.
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveMcpServerOutput {
    pub server: String,
    pub status: String,
    pub message: String,
}

pub struct RemoveMcpServer {
    manager_cell: ManagerCell,
}

impl RemoveMcpServer {
    pub fn new(manager_cell: ManagerCell) -> Self {
        Self { manager_cell }
    }
}

impl Tool for RemoveMcpServer {
    const NAME: &'static str = "remove_mcp_server";

    type Error = McpError;
    type Args = RemoveMcpServerArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "remove_mcp_server".to_string(),
            description: "Disconnect an MCP server. Since all MCP tools are \
                accessed through the single `call_mcp_tool` dispatch, removing \
                a server does NOT free individual tool slots — but it does \
                free the server connection. Use list_mcp_servers to see what's connected."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the MCP server to remove"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let manager = resolve_manager(&self.manager_cell)?;
        match manager.disconnect(&args.name).await {
            Ok(()) => Ok(serde_json::to_string(&RemoveMcpServerOutput {
                server: args.name,
                status: "removed".to_string(),
                message: "MCP server disconnected".to_string(),
            })
            .unwrap_or_else(|_| "removed".to_string())),
            Err(e) => Ok(serde_json::to_string(&RemoveMcpServerOutput {
                server: args.name,
                status: "error".to_string(),
                message: format!("failed to remove: {e}"),
            })
            .unwrap_or_else(|_| format!("failed: {e}"))),
        }
    }
}

// ── CallMcpTool ──────────────────────────────────────────────

/// Single dispatch tool that calls any MCP server tool by name.
///
/// Instead of registering every MCP tool as a separate static tool (which
/// wastes context tokens), all MCP servers share this ONE tool slot. The
/// LLM specifies which server and which tool to invoke.
#[derive(Debug, Deserialize)]
pub struct CallMcpToolArgs {
    /// Name of the MCP server (from list_mcp_servers).
    pub server: String,
    /// Name of the tool to call on that server.
    pub tool: String,
    /// Arguments to pass to the tool.
    #[serde(default)]
    pub arguments: serde_json::Value,
}

pub struct CallMcpTool {
    manager_cell: ManagerCell,
}

impl CallMcpTool {
    pub fn new(manager_cell: ManagerCell) -> Self {
        Self { manager_cell }
    }
}

impl Tool for CallMcpTool {
    const NAME: &'static str = "call_mcp_tool";

    type Error = McpError;
    type Args = CallMcpToolArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "call_mcp_tool".to_string(),
            description: "Call a tool on a connected MCP server. \
                Specify the server name (from list_mcp_servers), \
                the tool name, and the arguments as a JSON object. \
                This is the only tool needed for ALL MCP servers — \
                each server's individual tools are not registered separately."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "server": {
                        "type": "string",
                        "description": "MCP server name (use list_mcp_servers to see available servers)"
                    },
                    "tool": {
                        "type": "string",
                        "description": "Tool name to invoke on the server"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "JSON object of arguments for the tool"
                    }
                },
                "required": ["server", "tool"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let manager = resolve_manager(&self.manager_cell)?;

        let peer = manager
            .get_peer(&args.server)
            .await
            .ok_or_else(|| McpError::ServerNotFound(args.server.clone()))?;

        let arguments: Option<rmcp::model::JsonObject> = if args.arguments.is_object() {
            args.arguments.as_object().cloned()
        } else {
            None
        };

        let mut request = CallToolRequestParams::new(args.tool);
        if let Some(arguments) = arguments {
            request = request.with_arguments(arguments);
        }

        let result = peer
            .call_tool(request)
            .await
            .map_err(|e| McpError::Other(format!("MCP tool call failed: {e}")))?;

        if result.is_error.unwrap_or(false) {
            let text = extract_text(&result.content);
            return Err(McpError::Other(format!("MCP tool returned error: {text}")));
        }

        Ok(extract_text(&result.content))
    }
}

fn extract_text(content: &[rmcp::model::Content]) -> String {
    content
        .iter()
        .filter_map(|c| c.raw.as_text())
        .map(|t| t.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}
