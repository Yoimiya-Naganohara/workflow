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
        let bytes_len = content.len();
        let total_lines = content.lines().count();
        let lines: Vec<&str> = content.lines().collect();
        // start/end are 1-indexed line numbers (default: entire file).
        let start_idx = args
            .start
            .unwrap_or(1)
            .saturating_sub(1)
            .min(total_lines.saturating_sub(1));
        let end_idx = args.end.unwrap_or(total_lines).min(total_lines);
        let preview = if bytes_len > 10000 {
            let selected = lines[start_idx..end_idx].join("\n");
            format!(
                "{}\n\n... [truncated, {} total lines, showing {}--{}]",
                selected,
                total_lines,
                start_idx + 1,
                end_idx
            )
        } else {
            content.clone()
        };
        Ok(format!(
            "(file: {}, {} lines, {} bytes)\n\n{}",
            args.path, total_lines, bytes_len, preview
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
        let start = start.min(len);
        let end = end.max(start).min(len);
        std::fs::write(&args.path, &args.content[start..end]).map_err(|e| ToolCallError(e.to_string()))?;
        let preview = if len > 200 {
            let preview_end = (start + 200).min(len);
            format!("{}...", &args.content[start..preview_end])
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
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&args.command)
            .output()
            .await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_read_file_basic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "line1\nline2\nline3").unwrap();
        let result = ReadFile
            .call(ReadFileArgs {
                path: f.path().to_str().unwrap().to_string(),
                start: None,
                end: None,
            })
            .await
            .unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("3 lines"));
    }

    #[tokio::test]
    async fn test_read_file_multibyte_line_selection() {
        let mut f = NamedTempFile::new().unwrap();
        // CJK characters: each is 3 bytes in UTF-8
        // Need >10KB to trigger the truncation path where line joining happens
        let cjk_line = "你好世界测试内容这是多字节字符测试";
        for _ in 0..500 {
            writeln!(f, "{}", cjk_line).unwrap();
        }
        let path = f.path().to_str().unwrap().to_string();
        // start=2, end=4 are 1-indexed line numbers → selects lines 2-4
        let result = ReadFile
            .call(ReadFileArgs {
                path,
                start: Some(2),
                end: Some(4),
            })
            .await
            .unwrap();
        // Should contain exactly 3 lines of CJK, no panic from byte slicing
        assert!(result.contains(cjk_line));
        assert!(result.contains("truncated"));
        let count = result.matches(cjk_line).count();
        assert_eq!(count, 3, "should select exactly 3 lines, got {count}");
    }

    #[tokio::test]
    async fn test_read_file_large_truncation() {
        let mut f = NamedTempFile::new().unwrap();
        // Write >10KB to trigger truncation path
        let content = "x".repeat(15000);
        write!(f, "{}", content).unwrap();
        let result = ReadFile
            .call(ReadFileArgs {
                path: f.path().to_str().unwrap().to_string(),
                start: None,
                end: None,
            })
            .await
            .unwrap();
        assert!(result.contains("truncated"));
    }

    #[tokio::test]
    async fn test_write_file_out_of_bounds_clamped() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        // content is 5 bytes, start=10 is out of bounds — should be clamped, not panic
        let result = WriteFile
            .call(WriteFileArgs {
                path: path.clone(),
                content: "hello".to_string(),
                start: Some(10),
                end: Some(200),
            })
            .await
            .unwrap();
        // Clamped to empty slice, so 0 bytes written
        assert!(result.contains("Written 0 bytes"));
    }

    #[tokio::test]
    async fn test_write_file_preview_within_bounds() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        // content is 5 bytes, start=0, end=5 is in bounds
        let result = WriteFile
            .call(WriteFileArgs {
                path,
                content: "short".to_string(),
                start: Some(0),
                end: Some(5),
            })
            .await
            .unwrap();
        assert!(result.contains("Written 5 bytes"));
    }

    #[tokio::test]
    async fn test_shell_basic() {
        let result = Shell
            .call(ShellArgs {
                command: "echo hello".to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("hello"));
        assert!(result.contains("exit code: 0"));
    }

    #[tokio::test]
    async fn test_shell_stderr() {
        let result = Shell
            .call(ShellArgs {
                command: "echo error >&2".to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("stderr"));
        assert!(result.contains("error"));
    }

    #[tokio::test]
    async fn test_list_dir_basic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "content").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        let result = ListDir
            .call(ListDirArgs {
                path: dir.path().to_str().unwrap().to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("test.txt"));
        assert!(result.contains("subdir/"));
        assert!(result.contains("1 files"));
        assert!(result.contains("1 dirs"));
    }

    #[tokio::test]
    async fn test_list_dir_nonexistent() {
        let result = ListDir
            .call(ListDirArgs {
                path: "/nonexistent/path/12345".to_string(),
            })
            .await;
        assert!(result.is_err());
    }
}
