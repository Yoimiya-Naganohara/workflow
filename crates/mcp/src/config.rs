//! MCP server configuration — read from `~/.workflow/mcp_servers.json`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::McpError;

/// Describes how to connect to a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Human-readable name (e.g. "filesystem", "playwright").
    pub name: String,
    /// Transport details.
    pub transport: McpTransport,
}

/// Supported MCP transport protocols.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    /// Launch a subprocess and communicate over stdio.
    #[serde(rename = "stdio")]
    Stdio {
        /// The command to run (e.g. "npx", "node", "uvx").
        command: String,
        /// Arguments to pass.
        #[serde(default)]
        args: Vec<String>,
        /// Extra environment variables to set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<HashMap<String, String>>,
    },
    /// Server-Sent Events transport.
    #[serde(rename = "sse")]
    Sse {
        /// SSE endpoint URL.
        url: String,
    },
    /// Streamable HTTP transport.
    #[serde(rename = "streamable-http")]
    StreamableHttp {
        /// HTTP endpoint URL.
        url: String,
    },
}

/// Collection of MCP server configs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfigFile {
    /// List of configured MCP servers.
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Source for MCP server configurations.
pub struct McpConfigSource {
    path: PathBuf,
}

impl McpConfigSource {
    /// Create a new source that reads from `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Return the default config path: `~/.workflow/mcp_servers.json`.
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".workflow")
            .join("mcp_servers.json")
    }

    /// Read and parse the config file.
    ///
    /// Returns `Ok(Vec::new())` if the file does not exist.
    pub fn load(&self) -> Result<Vec<McpServerConfig>, McpError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&self.path).map_err(|source| {
            McpError::ConfigRead {
                path: self.path.clone(),
                source,
            }
        })?;

        // Accept either a top-level array or an object with a "servers" key.
        if content.trim().starts_with('[') {
            serde_json::from_str::<Vec<McpServerConfig>>(&content).map_err(|source| {
                McpError::ConfigParse {
                    path: self.path.clone(),
                    source,
                }
            })
        } else {
            let file: McpConfigFile =
                serde_json::from_str(&content).map_err(|source| McpError::ConfigParse {
                    path: self.path.clone(),
                    source,
                })?;
            Ok(file.servers)
        }
    }

    /// The config file path.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Add (or replace) a server config and persist to disk.
    ///
    /// If a server with the same name already exists, it is replaced.
    /// Returns the full list of servers after the change.
    pub fn add_server(&self, config: McpServerConfig) -> Result<Vec<McpServerConfig>, McpError> {
        let mut servers = self.load()?;

        if let Some(pos) = servers.iter().position(|s| s.name == config.name) {
            servers[pos] = config;
        } else {
            servers.push(config);
        }

        self.write(&servers)?;
        Ok(servers)
    }

    /// Remove a server by name and persist to disk.
    /// Returns `Ok(true)` if removed, `Ok(false)` if not found.
    pub fn remove_server(&self, name: &str) -> Result<bool, McpError> {
        let mut servers = self.load()?;
        let len_before = servers.len();
        servers.retain(|s| s.name != name);
        if servers.len() == len_before {
            return Ok(false);
        }
        self.write(&servers)?;
        Ok(true)
    }

    fn write(&self, servers: &[McpServerConfig]) -> Result<(), McpError> {
        let file = McpConfigFile {
            servers: servers.to_vec(),
        };
        let content = serde_json::to_string_pretty(&file).map_err(|source| {
            McpError::Other(format!("failed to serialize config: {source}"))
        })?;

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| McpError::ConfigRead {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        std::fs::write(&self.path, content).map_err(|source| McpError::ConfigRead {
            path: self.path.clone(),
            source,
        })?;
        Ok(())
    }
}
