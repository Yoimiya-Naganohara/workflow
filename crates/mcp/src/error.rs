//! Error types for the MCP client/server.

use std::path::PathBuf;

/// Errors that can occur during MCP operations.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Config file could not be read or parsed.
    #[error("failed to read MCP config from {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Config JSON could not be parsed.
    #[error("failed to parse MCP config at {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// Failed to spawn an MCP server process.
    #[error("failed to spawn MCP server '{server}': {source}")]
    Spawn {
        server: String,
        #[source]
        source: std::io::Error,
    },

    /// Failed to connect to an MCP server.
    #[error("failed to connect to MCP server '{server}': {source}")]
    Connect {
        server: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Failed to fetch tools from an MCP server.
    #[error("failed to fetch tools from MCP server '{server}': {source}")]
    ListTools {
        server: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The transport type is not supported.
    #[error("unsupported transport type for MCP server '{server}': {transport}")]
    UnsupportedTransport {
        server: String,
        transport: String,
    },

    /// MCP server '{server}' not found.
    #[error("MCP server '{0}' not found in config")]
    ServerNotFound(String),

    /// Generic MCP error.
    #[error("MCP error: {0}")]
    Other(String),
}

impl McpError {
    pub fn connect(
        server: impl Into<String>,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Connect {
            server: server.into(),
            source: Box::new(error),
        }
    }
}
