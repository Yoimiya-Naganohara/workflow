//! Built-in MCP tools implementing [`rig::tool::Tool`].
//!
//! Each tool has a named struct, an `Args` deserialization type,
//! and implements the `Tool` trait for registration on a
//! [`ToolServer`](rig::tool::server::ToolServer).

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::spawn_blocking;

use crate::tools::sandbox::SandboxHandle;

/// Execute a blocking I/O operation on the blocking thread pool.
/// Prevents `std::fs` calls from starving the Tokio async runtime.
async fn spawn_blocking_fs<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, ToolCallError> {
    spawn_blocking(f)
        .await
        .map_err(|e| ToolCallError(format!("Blocking pool join failed: {}", e)))?
        .map_err(ToolCallError)
}

/// Register all built-in tools on a `ToolServer` (without sandbox).
pub fn register_tools(server: crate::tools::ToolServer) -> crate::tools::ToolServer {
    register_sandboxed_tools(server, None, false)
}

/// Register all built-in tools, optionally with a shared sandbox handle.
///
/// When `sandbox` is `Some` and `with_search_asset` is true,
/// the `search_asset` (semantic retrieval) tool is also registered.
/// `search_asset` requires both a sandbox and an embedding engine,
/// so it is **excluded** from plain tool servers to prevent
/// registration of a tool that always fails at runtime.
///
/// The three filesystem-critical tools (ReadFile, WriteFile, Shell)
/// resolve paths through the sandbox, isolating writes and preventing
/// path escape even when `search_asset` is excluded.
pub fn register_sandboxed_tools(
    server: crate::tools::ToolServer,
    sandbox: Option<std::sync::Arc<SandboxHandle>>,
    with_search_asset: bool,
) -> crate::tools::ToolServer {
    let server = server
        .tool(ReadFile {
            sandbox: sandbox.clone(),
        })
        .tool(WriteFile {
            sandbox: sandbox.clone(),
        })
        .tool(Shell {
            sandbox: sandbox.clone(),
        })
        .tool(crate::tools::diff_edit::DiffEdit);
    if with_search_asset {
        server.tool(SearchAsset { sandbox })
    } else {
        server
    }
}

// ── ReadFile ──

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
    pub start: Option<usize>,
    pub end: Option<usize>,
}

pub struct ReadFile {
    pub sandbox: Option<Arc<SandboxHandle>>,
}

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";

    type Error = ToolCallError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Read a file from the filesystem. Returns contents + metadata. Truncated at 10KB."
                    .into(),
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
        // Resolve path through sandbox (read-only, allows source symlink).
        let path = match self.sandbox.as_ref() {
            Some(sb) => sb
                .resolve_path_read_only(&args.path)
                .map_err(|e| ToolCallError(format!("Sandbox: {}", e)))?,
            None => PathBuf::from(&args.path),
        };
        let content =
            spawn_blocking_fs(move || std::fs::read_to_string(&path).map_err(|e| e.to_string()))
                .await?;
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

pub struct WriteFile {
    pub sandbox: Option<Arc<SandboxHandle>>,
}

impl Tool for WriteFile {
    const NAME: &'static str = "write_file";

    type Error = ToolCallError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Write content to a file (creates or overwrites). Returns confirmation with preview.".into(),
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
        // Resolve path through sandbox — writes land in workdir only.
        // Uses resolve_write_path which rejects paths that would escape
        // the writable workdir into the read-only source tree (P0 safety).
        let path = match self.sandbox.as_ref() {
            Some(sb) => sb
                .resolve_write_path(&args.path)
                .map_err(|e| ToolCallError(format!("Sandbox: {}", e)))?,
            None => PathBuf::from(&args.path),
        };

        // Create parent directories (safe — resolve_write_path already asserted boundary).
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let len = args.content.len();
        let start = args.start.unwrap_or(0);
        let end = args.end.unwrap_or(len);
        let start = start.min(len);
        let end = end.max(start).min(len);

        // Safe slicing: .get() returns None instead of panicking on
        // non-UTF-8 boundaries (LLM may send byte indices that split chars).
        let write_slice = args.content.get(start..end).unwrap_or("");
        std::fs::write(&path, write_slice).map_err(|e| ToolCallError(e.to_string()))?;

        let preview = if len > 200 {
            let preview_end = (start + 200).min(len);
            let slice = args.content.get(start..preview_end).unwrap_or("");
            format!("{}...", slice)
        } else {
            write_slice.to_string()
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

pub struct Shell {
    pub sandbox: Option<Arc<SandboxHandle>>,
}

impl Tool for Shell {
    const NAME: &'static str = "sh";

    type Error = ToolCallError;
    type Args = ShellArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Execute a shell command. Returns stdout/stderr with exit code.".into(),
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
        // 30-second timeout; cwd anchored to sandbox workdir when available.
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&args.command);
        if let Some(ref sb) = self.sandbox {
            cmd.current_dir(&sb.workdir);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| ToolCallError("Command timed out after 30 seconds".to_string()))?
            .map_err(|e| ToolCallError(e.to_string()))?;

        let code = output.status.code().unwrap_or(-1);
        let mut result = String::new();
        if !output.stdout.is_empty() {
            let s = String::from_utf8_lossy(&output.stdout);
            // Cap output at 100KB to avoid context overflow (char-boundary safe).
            if s.len() > 102_400 {
                let end = s
                    .char_indices()
                    .nth(102_400)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                result.push_str(&s[..end]);
                result.push_str("\n... [stdout truncated at 100KB]");
            } else {
                result.push_str(&s);
            }
        }
        if !output.stderr.is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str("stderr:\n");
            let s = String::from_utf8_lossy(&output.stderr);
            if s.len() > 51_200 {
                let end = s
                    .char_indices()
                    .nth(51_200)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                result.push_str(&s[..end]);
                result.push_str("\n... [stderr truncated at 50KB]");
            } else {
                result.push_str(&s);
            }
        }
        result.push_str(&format!("\n(exit code: {})", code));
        Ok(result)
    }
}

// ── SearchAsset — semantic retrieval against indexed assets ──

#[derive(Deserialize)]
pub struct SearchAssetArgs {
    pub asset_id: String,
    pub query: String,
    pub top_k: Option<usize>,
}

pub struct SearchAsset {
    pub sandbox: Option<std::sync::Arc<super::sandbox::SandboxHandle>>,
}

impl Tool for SearchAsset {
    const NAME: &'static str = "search_asset";

    type Error = ToolCallError;
    type Args = SearchAssetArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Semantic search within a previously indexed asset. ",
                "Targeted at large compilation outputs or logs. ",
                "Avoid reading the full asset via read_file. ",
                "Returns top-K semantically relevant chunks (each ~20 lines)."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "asset_id": {
                        "type": "string",
                        "description": "Asset ID returned by Shell/ReadFile after SIMD indexing"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query — describe what you're looking for (e.g. 'unresolved import error', 'panic at main')"
                    },
                    "top_k": {
                        "type": "integer",
                        "description": "Number of chunks to return (default: 3, max: 10)",
                        "minimum": 1,
                        "maximum": 10,
                        "optional": true
                    }
                },
                "required": ["asset_id", "query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let sandbox = self.sandbox.as_ref().ok_or_else(|| {
            ToolCallError("SearchAsset requires a sandboxed agent context".into())
        })?;

        let model = {
            let guard = sandbox.embedder.read().expect("builtin mutex poisoned");
            guard
                .as_ref()
                .ok_or_else(|| {
                    ToolCallError(
                        "No embedding model attached — asset was stored but not indexed".into(),
                    )
                })?
                .clone()
        };

        let query_emb = model
            .embed(&args.query)
            .await
            .map_err(|e| ToolCallError(format!("Embedding failed: {}", e)))?;

        let results = {
            let guard = sandbox
                .asset_indices
                .read()
                .expect("builtin mutex poisoned");
            let asset = guard.get(&args.asset_id).ok_or_else(|| {
                ToolCallError(format!(
                    "Asset '{}' not found or not indexed",
                    args.asset_id
                ))
            })?;
            asset.search(&query_emb, args.top_k.unwrap_or(3))
        };

        if results.is_empty() {
            return Ok(format!(
                "[search_asset] No relevant chunks in '{}' for: \"{}\"",
                args.asset_id, args.query
            ));
        }

        let mut out = format!(
            "=== Semantic search in '{}' — \"{}\" ===\n",
            args.asset_id, args.query
        );
        for (line_start, content) in &results {
            out.push_str(&format!("\n--- Line {} ---\n{}", line_start, content));
        }
        out.push_str(&format!("\n[returned {} chunk(s)]", results.len()));
        Ok(out)
    }
}

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
        let result = (ReadFile { sandbox: None })
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
        let result = (ReadFile { sandbox: None })
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
        let result = (ReadFile { sandbox: None })
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
        let result = (WriteFile { sandbox: None })
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
        let result = (WriteFile { sandbox: None })
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
        let result = (Shell { sandbox: None })
            .call(ShellArgs {
                command: "echo hello".to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("hello"));
        assert!(result.contains("exit code: 0"));
    }
}
