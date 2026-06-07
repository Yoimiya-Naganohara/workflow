//! Built-in MCP tools implementing [`rig::tool::Tool`].
//!
//! Each tool has a named struct, an `Args` deserialization type,
//! and implements the `Tool` trait for registration on a
//! [`ToolServer`](rig::tool::server::ToolServer).

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

/// Register all built-in tools on a `ToolServer`.
pub fn register_tools(server: crate::tools::ToolServer) -> crate::tools::ToolServer {
    server.tool(ReadFile).tool(WriteFile).tool(Shell).tool(ListDir)
}

// ── ReadFile ──

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
}

pub struct ReadFile;

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";

    type Error = ToolCallError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Read the contents of a file from the filesystem.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = std::fs::read_to_string(&args.path).map_err(|e| ToolCallError(e.to_string()))?;
        Ok(content)
    }
}

// ── WriteFile ──

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
}

pub struct WriteFile;

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";

    type Error = ToolCallError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Write content to a file. Creates or overwrites.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to write to"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let len = args.content.len();
        std::fs::write(&args.path, &args.content).map_err(|e| ToolCallError(e.to_string()))?;
        Ok(format!("Written {} bytes to {}", len, args.path))
    }
}

// ── Shell ──

#[derive(Deserialize)]
pub struct ShellArgs {
    pub command: String,
}

pub struct Shell;

impl Tool for Shell {
    const NAME: &'static str = "sh";

    type Error = ToolCallError;
    type Args = ShellArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Execute a shell command and return stdout/stderr.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "Shell command to execute"
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .output()
            .map_err(|e| ToolCallError(e.to_string()))?;

        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        if result.is_empty() {
            result = format!("(exit code: {})", output.status.code().unwrap_or(-1));
        }
        Ok(result)
    }
}

// ── ListDir ──

#[derive(Deserialize)]
pub struct ListDirArgs {
    pub path: String,
}

pub struct ListDir;

impl Tool for ListDir {
    const NAME: &'static str = "list_dir";

    type Error = ToolCallError;
    type Args = ListDirArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "List files and directories in a given path.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let entries = std::fs::read_dir(&args.path).map_err(|e| ToolCallError(e.to_string()))?;
        let mut items: Vec<String> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| ToolCallError(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = if entry.file_type().map_err(|e| ToolCallError(e.to_string()))?.is_dir() {
                "dir"
            } else {
                "file"
            };
            items.push(format!("  {} [{}]", name, kind));
        }
        items.sort();
        let mut result = format!("Contents of '{}':\n", args.path);
        result.push_str(&items.join("\n"));
        Ok(result)
    }
}

// ── Error type ──

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolCallError(pub String);
