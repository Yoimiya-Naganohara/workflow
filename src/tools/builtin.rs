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
    pub start: Option<usize>,
    pub end: Option<usize>,
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
            description: "Read a file from the filesystem. Returns contents plus file metadata (lines, bytes). Truncated at 10KB.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to read"
                    },
                    "start": {
                        "type": "integer",
                        "description": "Optional start line to read from",
                        "minimum": 0,
                        "optional": true
                    },
                    "end": {
                        "type": "integer",
                        "description": "Optional end line to read to",
                        "minimum": 0,
                        "optional": true
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = std::fs::read_to_string(&args.path).map_err(|e| ToolCallError(e.to_string()))?;
        let total_bytes = content.len();
        let total_lines = content.lines().count();
        let start = args.start.unwrap_or(0);
        let end = args.end.unwrap_or(total_lines);
        let preview = if total_bytes > 10000 {
            format!(
                "{}\n\n... [truncated, {} total bytes, {} lines]",
                &content[start..end],
                total_bytes - (end - start),
                end - start
            )
        } else {
            content
        };
        Ok(format!(
            "(file: {}, {} lines, {} bytes)\n\n{}",
            args.path, total_lines, total_bytes, preview
        ))
    }
}

// ── WriteFile ──

#[derive(Deserialize)]
pub struct WriteFileArgs {
    pub path: String,
    pub content: String,
    pub start: Option<usize>,
    pub end: Option<usize>,
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
            description:
                "Write content to a file (creates or overwrites). Returns write confirmation with a content preview."
                    .into(),
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
                    },
                    "start": {
                        "type": "integer",
                        "description": "Start position in content to write",
                        "optional": true
                    },
                    "end": {
                        "type": "integer",
                        "description": "End position in content to write",
                        "optional": true
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let len = args.content.len();
        let start = args.start.unwrap_or(0);
        let end = args.end.unwrap_or(len);
        std::fs::write(&args.path, &args.content[start..end]).map_err(|e| ToolCallError(e.to_string()))?;
        let preview = if len > 200 {
            format!("{}...", &args.content[start..start + 200])
        } else {
            args.content[start..end].to_string()
        };
        Ok(format!(
            "Written {} bytes to {}\nFirst {} chars:\n---\n{}\n---",
            end - start,
            args.path,
            preview.len(),
            preview
        ))
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
            description: "Execute a shell command and return stdout/stderr with exit code. The working directory is the project root.".into(),
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

        let code = output.status.code().unwrap_or(-1);
        let mut result = String::new();
        if !output.stdout.is_empty() {
            result.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr:\n");
            result.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        result.push_str(&format!("\n(exit code: {})", code));
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
            description: "List files and directories in a path. Shows file sizes and directory counts.".into(),
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
        let mut dir_count = 0u32;
        let mut file_count = 0u32;
        for entry in entries {
            let entry = entry.map_err(|e| ToolCallError(e.to_string()))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata().map_err(|e| ToolCallError(e.to_string()))?;
            if meta.is_dir() {
                dir_count += 1;
                items.push(format!("  {}/", name));
            } else {
                file_count += 1;
                let size = meta.len();
                items.push(format!("  {}  ({} bytes)", name, size));
            }
        }
        items.sort();
        let mut result = format!(
            "Contents of '{}': {} files, {} dirs\n",
            args.path, file_count, dir_count
        );
        result.push_str(&items.join("\n"));
        Ok(result)
    }
}

// ── Error type ──

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolCallError(pub String);
