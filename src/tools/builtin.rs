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
use std::time::Duration;
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
    register_sandboxed_tools(server, None)
}

/// Register all built-in tools, optionally with a shared sandbox handle.
///
/// When `sandbox` is `Some`, the three filesystem-critical tools
/// (ReadFile, WriteFile, Shell) will resolve paths through the
/// sandbox, isolating writes and preventing path escape.
pub fn register_sandboxed_tools(
    server: crate::tools::ToolServer,
    sandbox: Option<std::sync::Arc<SandboxHandle>>,
) -> crate::tools::ToolServer {
    server
        .tool(ReadFile {
            sandbox: sandbox.clone(),
        })
        .tool(WriteFile {
            sandbox: sandbox.clone(),
        })
        .tool(Shell {
            sandbox: sandbox.clone(),
        })
        .tool(ListDir)
        .tool(Grep)
        .tool(FindFiles)
        .tool(MoveFile)
        .tool(CopyFile)
        .tool(DeleteFile)
        .tool(AppendFile)
        .tool(PatchFile)
        .tool(Glob)
        .tool(LineEdit)
        .tool(Fetch)
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
        // Resolve path through sandbox (read-only, allows source symlink).
        let path = match self.sandbox.as_ref() {
            Some(sb) => sb
                .resolve_path_read_only(&args.path)
                .map_err(|e| ToolCallError(format!("Sandbox: {}", e)))?,
            None => PathBuf::from(&args.path),
        };
        let content = spawn_blocking_fs(move || std::fs::read_to_string(&path).map_err(|e| e.to_string())).await?;
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
        // Resolve path through sandbox — writes land in workdir only.
        let path = match self.sandbox.as_ref() {
            Some(sb) => sb
                .resolve_path(&args.path)
                .map_err(|e| ToolCallError(format!("Sandbox: {}", e)))?,
            None => PathBuf::from(&args.path),
        };

        // Create parent directories (safe — resolve_path already asserted boundary).
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
                let end = s.char_indices().nth(102_400).map(|(i, _)| i).unwrap_or(s.len());
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
                let end = s.char_indices().nth(51_200).map(|(i, _)| i).unwrap_or(s.len());
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

// ── Grep ──

#[derive(Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: String,
    pub max_results: Option<usize>,
}

pub struct Grep;

impl Tool for Grep {
    const NAME: &'static str = "grep";

    type Error = ToolCallError;
    type Args = GrepArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Search files for lines matching a pattern (regex). ",
                "Returns matching lines with line numbers. ",
                "Truncated at max_results (default 50)."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Regex pattern to search for (Rust regex syntax)"
                    },
                    "path": {
                        "type": "string",
                        "description": "File path or directory path to search in. Directories are searched recursively."
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum matches to return (default: 50, max: 500)",
                        "minimum": 1,
                        "maximum": 500,
                        "optional": true
                    }
                },
                "required": ["pattern", "path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use regex;
        use std::io::{BufRead, Read};

        let max_results = args.max_results.unwrap_or(50).min(500);
        let re = regex::Regex::new(&args.pattern).map_err(|e| ToolCallError(format!("Invalid regex: {}", e)))?;

        let path = std::path::Path::new(&args.path);

        // ── Directory mode ──
        // Check path first: on macOS/Linux, File::open() on a dir succeeds
        // and the subsequent is_file() check would reject it with a misleading error.
        if path.is_dir() {
            return grep_directory(args, &re, max_results).await;
        }

        // ── Single file mode ──
        // Open first, then check metadata from the handle (avoids TOCTOU race)
        if let Ok(file) = std::fs::File::open(path) {
            let meta = file.metadata().map_err(|e| ToolCallError(e.to_string()))?;
            if !meta.is_file() {
                return Err(ToolCallError(format!("'{}' is not a file", args.path)));
            }

            // Binary detection: check first 8KB for null byte
            let mut buf = [0u8; 8192];
            let n = (&file).read(&mut buf).unwrap_or(0);
            if buf[..n].contains(&0) {
                return Ok(format!("(file: {}, binary — skipping)", args.path));
            }

            // Re-create file handle after binary check consumed position
            let file = std::fs::File::open(path).map_err(|e| ToolCallError(e.to_string()))?;
            let reader = std::io::BufReader::new(file);
            let mut matches: Vec<String> = Vec::new();
            for (i, line) in reader.lines().enumerate() {
                if matches.len() >= max_results {
                    break;
                }
                let line = line.map_err(|e| ToolCallError(e.to_string()))?;
                if re.is_match(&line) {
                    matches.push(format!("  {:>6}: {}", i + 1, line));
                }
            }
            let total = matches.len();
            let mut result = format!("Grep '{}' in '{}' — {} match(es)\n", args.pattern, args.path, total);
            if !matches.is_empty() {
                result.push_str(&matches.join("\n"));
            }
            if total >= max_results {
                result.push_str(&format!("\n... truncated at {} matches", max_results));
            }
            return Ok(result);
        }

        Err(ToolCallError(format!(
            "Path '{}' is not a file or directory",
            args.path
        )))
    }
}

/// Recursive directory grep, offloaded to the blocking thread pool.
async fn grep_directory(args: GrepArgs, re: &regex::Regex, max_results: usize) -> Result<String, ToolCallError> {
    use std::io::{BufRead, Read};
    let re = re.clone();
    spawn_blocking(move || {
        let path = std::path::Path::new(&args.path);
        let mut matches: Vec<(String, usize, String)> = Vec::new();
        let mut file_count = 0u32;
        for entry in walkdir::WalkDir::new(path).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            !name.starts_with('.') && name != "node_modules" && name != "target" && name != "dist" && name != "build"
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() || entry.path_is_symlink() {
                continue;
            }
            let fpath = entry.path().to_path_buf();
            file_count += 1;
            let Ok(file) = std::fs::File::open(&fpath) else {
                continue;
            };

            // Binary detection
            let mut buf = [0u8; 8192];
            let n = (&file).read(&mut buf).unwrap_or(0);
            if buf[..n].contains(&0) {
                continue;
            }

            // Re-open and search
            let Ok(file) = std::fs::File::open(&fpath) else {
                continue;
            };
            let reader = std::io::BufReader::new(file);
            for (i, line) in reader.lines().enumerate() {
                if matches.len() >= max_results {
                    break;
                }
                let line = match line {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                if re.is_match(&line) {
                    matches.push((fpath.to_string_lossy().to_string(), i + 1, line));
                }
            }
            if matches.len() >= max_results {
                break;
            }
        }
        let total = matches.len();
        let mut result = format!(
            "Grep '{}' in '{}' — {} match(es) across {} file(s)\n",
            args.pattern, args.path, total, file_count
        );
        for (fpath, lineno, line) in &matches {
            result.push_str(&format!("  {}:{}:{}\n", fpath, lineno, line));
        }
        if total >= max_results {
            result.push_str(&format!("... truncated at {} matches", max_results));
        }
        Ok(result)
    })
    .await
    .map_err(|e| ToolCallError(format!("Blocking pool join failed: {}", e)))?
}

// ── FindFiles ──

#[derive(Deserialize)]
pub struct FindFilesArgs {
    pub pattern: String,
    pub root: Option<String>,
    pub max_results: Option<usize>,
}

pub struct FindFiles;

impl Tool for FindFiles {
    const NAME: &'static str = "find_files";

    type Error = ToolCallError;
    type Args = FindFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Find files by glob pattern (e.g. \"**/*.rs\", \"*.toml\", \"src/**/mod.rs\"). ",
                "Returns matching file paths with sizes. Default root is current directory."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern to match file paths against (e.g. **/*.rs, Cargo.*)"
                    },
                    "root": {
                        "type": "string",
                        "description": "Root directory to start search from (default: current working dir)",
                        "optional": true
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum files to return (default: 100, max: 1000)",
                        "minimum": 1,
                        "maximum": 1000,
                        "optional": true
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let root = args.root.unwrap_or_else(|| ".".to_string());
        let max_results = args.max_results.unwrap_or(100).min(1000);

        let glob_pattern = glob::Pattern::new(&args.pattern)
            .map_err(|e| ToolCallError(format!("Invalid glob pattern '{}': {}", args.pattern, e)))?;

        let mut results: Vec<(String, u64, bool)> = Vec::new();
        for entry in walkdir::WalkDir::new(&root).into_iter().filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            if e.depth() == 0 {
                return true;
            }
            !name.starts_with('.') && name != "node_modules" && name != "target" && name != "dist"
        }) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if results.len() >= max_results {
                break;
            }
            let rel_path = entry
                .path()
                .strip_prefix(&root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .to_string();
            if glob_pattern.matches(&rel_path) || glob_pattern.matches(entry.path().to_string_lossy().as_ref()) {
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                results.push((rel_path, meta.len(), meta.is_dir()));
            }
        }

        results.sort_by(|a, b| a.0.cmp(&b.0));

        let dirs: Vec<_> = results.iter().filter(|(_, _, is_dir)| *is_dir).collect();
        let files: Vec<_> = results.iter().filter(|(_, _, is_dir)| !*is_dir).collect();

        let mut output = format!(
            "Find '{}' in '{}' — {} file(s), {} dir(s)\n",
            args.pattern,
            root,
            files.len(),
            dirs.len()
        );
        for (path, size, _) in &results {
            if *size > 0 {
                let size_str = if *size > 1024 * 1024 {
                    format!("{:.1} MB", *size as f64 / (1024.0 * 1024.0))
                } else if *size > 1024 {
                    format!("{:.1} KB", *size as f64 / 1024.0)
                } else {
                    format!("{} B", size)
                };
                output.push_str(&format!("  {}  ({})\n", path, size_str));
            } else {
                output.push_str(&format!("  {}/\n", path));
            }
        }
        if results.len() >= max_results {
            output.push_str(&format!("... truncated at {} results", max_results));
        }
        Ok(output)
    }
}

// ── MoveFile / rename ──

#[derive(Deserialize)]
pub struct MoveFileArgs {
    pub source: String,
    pub destination: String,
}

pub struct MoveFile;

impl Tool for MoveFile {
    const NAME: &'static str = "move_file";

    type Error = ToolCallError;
    type Args = MoveFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Move or rename a file or directory. Creates parent directories if needed.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source path to move from"
                    },
                    "destination": {
                        "type": "string",
                        "description": "Destination path to move to"
                    }
                },
                "required": ["source", "destination"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Validate source exists
        let meta = std::fs::metadata(&args.source)
            .map_err(|e| ToolCallError(format!("Source '{}' not found: {}", args.source, e)))?;

        // Create parent directory of destination if needed
        if let Some(parent) = std::path::Path::new(&args.destination).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ToolCallError(format!("Failed to create parent dir '{}': {}", parent.display(), e)))?;
            }
        }

        // Attempt rename; on cross-device error, fall back to copy+delete
        if let Err(e) = std::fs::rename(&args.source, &args.destination) {
            let is_cross_device = e.raw_os_error() == Some(18)
                || e.to_string().contains("cross-device")
                || e.to_string().contains("Invalid cross-device");
            if is_cross_device {
                if meta.is_dir() {
                    copy_dir_recursive(
                        std::path::Path::new(&args.source),
                        std::path::Path::new(&args.destination),
                    )
                    .map_err(|e2| ToolCallError(format!("Move failed (cross-device copy): {}", e2)))?;
                    std::fs::remove_dir_all(&args.source).map_err(|e2| {
                        ToolCallError(format!(
                            "Moved content but failed to remove source '{}': {}",
                            args.source, e2
                        ))
                    })?;
                } else {
                    std::fs::copy(&args.source, &args.destination)
                        .map_err(|e2| ToolCallError(format!("Move failed (cross-device copy): {}", e2)))?;
                    std::fs::remove_file(&args.source).map_err(|e2| {
                        ToolCallError(format!(
                            "Moved content but failed to remove source '{}': {}",
                            args.source, e2
                        ))
                    })?;
                }
            } else {
                return Err(ToolCallError(format!(
                    "Failed to move '{}' -> '{}': {}",
                    args.source, args.destination, e
                )));
            }
        }

        let kind = if meta.is_dir() { "directory" } else { "file" };
        Ok(format!("Moved {} '{}' -> '{}'", kind, args.source, args.destination))
    }
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ── CopyFile ──

#[derive(Deserialize)]
pub struct CopyFileArgs {
    pub source: String,
    pub destination: String,
}

pub struct CopyFile;

impl Tool for CopyFile {
    const NAME: &'static str = "copy_file";

    type Error = ToolCallError;
    type Args = CopyFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Copy a file from source to destination. Creates parent directories if needed.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": {
                        "type": "string",
                        "description": "Source path to copy from"
                    },
                    "destination": {
                        "type": "string",
                        "description": "Destination path to copy to"
                    }
                },
                "required": ["source", "destination"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let meta = std::fs::metadata(&args.source)
            .map_err(|e| ToolCallError(format!("Source '{}' not found: {}", args.source, e)))?;

        // Create parent directory of destination if needed
        if let Some(parent) = std::path::Path::new(&args.destination).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ToolCallError(format!("Failed to create parent dir '{}': {}", parent.display(), e)))?;
            }
        }

        if meta.is_dir() {
            copy_dir_recursive(
                std::path::Path::new(&args.source),
                std::path::Path::new(&args.destination),
            )
            .map_err(|e| ToolCallError(format!("Failed to copy directory: {}", e)))?;
        } else {
            std::fs::copy(&args.source, &args.destination)
                .map_err(|e| ToolCallError(format!("Failed to copy file: {}", e)))?;
        }

        let kind = if meta.is_dir() { "directory" } else { "file" };
        let size = if meta.is_dir() {
            dir_size(std::path::Path::new(&args.destination))
        } else {
            std::fs::metadata(&args.destination).map(|m| m.len()).unwrap_or(0)
        };
        Ok(format!(
            "Copied {} '{}' -> '{}' ({} bytes)",
            kind, args.source, args.destination, size
        ))
    }
}

/// Calculate total size of a directory tree.
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

// ── DeleteFile ──

#[derive(Deserialize)]
pub struct DeleteFileArgs {
    pub path: String,
    pub recursive: Option<bool>,
    pub force: Option<bool>,
}

pub struct DeleteFile;

impl Tool for DeleteFile {
    const NAME: &'static str = "delete_file";

    type Error = ToolCallError;
    type Args = DeleteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Delete a file or directory. Directories require recursive=true. Safety: refuses to delete paths containing '..' or known system roots ('/', '/etc', '/home', etc.) unless force=true."
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to delete"
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Required true to delete directories",
                        "optional": true
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Bypass safety checks for known system paths",
                        "optional": true
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Safety check: refuse to delete obviously dangerous paths
        if !args.force.unwrap_or(false) {
            let dangerous = [
                "/",
                "/etc",
                "/etc/passwd",
                "/etc/shadow",
                "/usr",
                "/bin",
                "/lib",
                "/lib64",
                "/sbin",
                "/opt",
                "/System",
                "/var",
                "/boot",
                "/dev",
                "/proc",
                "/sys",
                "/home",
                "/root",
                "/private",
                "/Users",
                "/var/log",
                "/var/log/system.log",
                "C:\\",
                "C:\\Windows",
                "C:\\System32",
                "C:\\Program Files",
            ];
            // Resolve symlinks and check canonical path
            let canon = match std::fs::canonicalize(&args.path) {
                Ok(p) => p,
                Err(_) => std::path::PathBuf::from(&args.path),
            };
            let canon_str = canon.to_string_lossy();
            for d in &dangerous {
                if canon_str == *d || canon_str.starts_with(&format!("{}/", d)) {
                    return Err(ToolCallError(format!(
                        "Refusing to delete system path '{}' (resolved: '{}'). Set force=true to bypass.",
                        args.path, canon_str
                    )));
                }
            }
            // Block parent traversal after symlink resolution
            if canon_str.contains("/../") || canon_str.ends_with("/..") || canon_str.starts_with("../") {
                return Err(ToolCallError(format!(
                    "Refusing to delete path traversing above root via '{}'. Set force=true to bypass.",
                    args.path
                )));
            }
        }

        let meta = std::fs::metadata(&args.path)
            .map_err(|e| ToolCallError(format!("Path '{}' not found: {}", args.path, e)))?;

        if meta.is_dir() {
            if !args.recursive.unwrap_or(false) {
                return Err(ToolCallError(format!(
                    "'{}' is a directory. Set recursive=true to delete directories.",
                    args.path
                )));
            }
            std::fs::remove_dir_all(&args.path)
                .map_err(|e| ToolCallError(format!("Failed to delete directory '{}': {}", args.path, e)))?;
            Ok(format!("Deleted directory '{}' (recursive)", args.path))
        } else {
            std::fs::remove_file(&args.path)
                .map_err(|e| ToolCallError(format!("Failed to delete file '{}': {}", args.path, e)))?;
            let size = meta.len();
            Ok(format!("Deleted file '{}' ({} bytes)", args.path, size))
        }
    }
}

// ── AppendFile ──

#[derive(Deserialize)]
pub struct AppendFileArgs {
    pub path: String,
    pub content: String,
    pub newline: Option<bool>,
}

pub struct AppendFile;

impl Tool for AppendFile {
    const NAME: &'static str = "append_file";

    type Error = ToolCallError;
    type Args = AppendFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Append content to the end of an existing file. ",
                "Creates the file if it doesn't exist. ",
                "Use newline=true (default) to add a leading newline before the content."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to append to"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to append"
                    },
                    "newline": {
                        "type": "boolean",
                        "description": "Add a leading newline before content (default: true)",
                        "optional": true
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        use std::io::Write;

        let add_newline = args.newline.unwrap_or(true);

        let prev_size = std::fs::metadata(&args.path).ok().map(|m| m.len()).unwrap_or(0);

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&args.path)
            .map_err(|e| ToolCallError(format!("Failed to open '{}' for append: {}", args.path, e)))?;

        if prev_size > 0 && add_newline {
            file.write_all(b"\n").map_err(|e| ToolCallError(e.to_string()))?;
        }

        let content_bytes = args.content.as_bytes();
        file.write_all(content_bytes)
            .map_err(|e| ToolCallError(e.to_string()))?;

        let new_size = prev_size + if prev_size > 0 && add_newline { 1 } else { 0 } + content_bytes.len() as u64;

        let preview = if content_bytes.len() > 200 {
            let end = args
                .content
                .char_indices()
                .nth(200)
                .map(|(i, _)| i)
                .unwrap_or(args.content.len());
            format!("{}...", &args.content[..end])
        } else {
            args.content.clone()
        };

        Ok(format!(
            "Appended {} bytes to '{}' (was {} bytes, now {} bytes)\nPreview:\n---\n{}
---",
            content_bytes.len(),
            args.path,
            prev_size,
            new_size,
            preview
        ))
    }
}

// ── PatchFile (search-and-replace) ──

#[derive(Deserialize)]
pub struct PatchFileArgs {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
    pub count: Option<usize>,
}

pub struct PatchFile;

impl Tool for PatchFile {
    const NAME: &'static str = "patch_file";

    type Error = ToolCallError;
    type Args = PatchFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Search-and-replace text in a file. ",
                "Replaces all occurrences of old_text with new_text. ",
                "Use count to limit replacements. ",
                "Prefer this over sh with sed for clarity and safety."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to patch"
                    },
                    "old_text": {
                        "type": "string",
                        "description": "Exact text substring to find and replace"
                    },
                    "new_text": {
                        "type": "string",
                        "description": "Replacement text"
                    },
                    "count": {
                        "type": "integer",
                        "description": "Number of replacements to make (default: all, max: 1000)",
                        "minimum": 1,
                        "maximum": 1000,
                        "optional": true
                    }
                },
                "required": ["path", "old_text", "new_text"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| ToolCallError(format!("Failed to read '{}': {}", args.path, e)))?;

        if !content.contains(&args.old_text) {
            let preview = if args.old_text.len() > 80 {
                let end = args
                    .old_text
                    .char_indices()
                    .nth(80)
                    .map(|(i, _)| i)
                    .unwrap_or(args.old_text.len());
                format!("{}...", &args.old_text[..end])
            } else {
                args.old_text.clone()
            };
            return Err(ToolCallError(format!(
                "old_text not found in '{}': {:?}",
                args.path, preview
            )));
        }

        let max_count = args.count.unwrap_or(usize::MAX);
        let total_occurrences = content.matches(&args.old_text).count();
        let effective_count = max_count.min(total_occurrences);

        // Build result manually — always safe, never over-replaces
        let new_content = if effective_count == total_occurrences {
            // Replacing all: fast path
            content.replace(&args.old_text, &args.new_text)
        } else {
            // Limited replacements via match_indices
            let mut result = String::with_capacity(content.len());
            let mut last_end = 0;
            for (replaced, (start, _)) in content.match_indices(&args.old_text).enumerate() {
                if replaced >= effective_count {
                    break;
                }
                result.push_str(&content[last_end..start]);
                result.push_str(&args.new_text);
                last_end = start + args.old_text.len();
            }
            result.push_str(&content[last_end..]);
            result
        };

        let replacement_count = effective_count;

        std::fs::write(&args.path, &new_content)
            .map_err(|e| ToolCallError(format!("Failed to write '{}': {}", args.path, e)))?;

        // Compute a diff-like preview
        let preview = if args.old_text.len() <= 120 && args.new_text.len() <= 120 {
            format!("--- {}\n+++ {}\n", args.old_text, args.new_text)
        } else {
            let old_preview = if args.old_text.len() > 60 {
                let end = args
                    .old_text
                    .char_indices()
                    .nth(60)
                    .map(|(i, _)| i)
                    .unwrap_or(args.old_text.len());
                format!("{}...", &args.old_text[..end])
            } else {
                args.old_text.clone()
            };
            let new_preview = if args.new_text.len() > 60 {
                let end = args
                    .new_text
                    .char_indices()
                    .nth(60)
                    .map(|(i, _)| i)
                    .unwrap_or(args.new_text.len());
                format!("{}...", &args.new_text[..end])
            } else {
                args.new_text.clone()
            };
            format!("--- {}...\n+++ {}...\n", old_preview, new_preview)
        };

        let skipped = total_occurrences.saturating_sub(replacement_count);

        Ok(format!(
            "Patched '{}': {} replacement(s){} | {}\nFile size: {} bytes → {} bytes",
            args.path,
            replacement_count,
            if skipped > 0 {
                format!(" ({} skipped due to count limit)", skipped)
            } else {
                String::new()
            },
            preview.trim(),
            content.len(),
            new_content.len()
        ))
    }
}

// ── Glob ──

#[derive(Deserialize)]
pub struct GlobArgs {
    pub pattern: String,
    pub root: Option<String>,
}

pub struct Glob;

impl Tool for Glob {
    const NAME: &'static str = "glob";

    type Error = ToolCallError;
    type Args = GlobArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Resolve a glob pattern and return matching file paths. Simpler than find_files — uses standard glob semantics (no recursive walk config).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob pattern, e.g. \"src/**/*.rs\", \"Cargo.*\", \"*.toml\""
                    },
                    "root": {
                        "type": "string",
                        "description": "Working directory (default: current dir)",
                        "optional": true
                    }
                },
                "required": ["pattern"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let root = args.root.unwrap_or_else(|| ".".to_string());
        let root_path = std::path::Path::new(&root);

        // Resolve the glob pattern against root WITHOUT changing process cwd
        // (set_current_dir is NOT thread-safe for concurrent tool calls).
        // Use glob::glob_with to scan relative to root.
        let full_pattern = if std::path::Path::new(&args.pattern).is_absolute() {
            args.pattern.clone()
        } else {
            // Prepend root, normalize slashes
            let root_normalized = root.trim_end_matches('/').trim_end_matches('\\');
            format!("{}/{}", root_normalized, args.pattern)
        };

        let mut results: Vec<String> = Vec::new();
        match glob::glob(&full_pattern) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(path) => {
                            // Convert to relative path from root for display
                            let rel = path
                                .strip_prefix(root_path)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .to_string();
                            results.push(rel);
                        }
                        Err(e) => {
                            tracing::warn!("Glob entry error: {}", e);
                        }
                    }
                }
            }
            Err(e) => {
                return Err(ToolCallError(format!("Invalid glob pattern '{}': {}", args.pattern, e)));
            }
        }

        results.sort();

        let total = results.len();
        let mut output = format!("Glob '{}' in '{}' — {} match(es)\n", args.pattern, root, total);
        for path in &results {
            // Show file size if it's a file
            let full_path = std::path::Path::new(&root).join(path);
            if let Ok(meta) = full_path.metadata() {
                if meta.is_file() {
                    let size_str = if meta.len() > 1024 * 1024 {
                        format!("{:.1} MB", meta.len() as f64 / (1024.0 * 1024.0))
                    } else if meta.len() > 1024 {
                        format!("{:.1} KB", meta.len() as f64 / 1024.0)
                    } else {
                        format!("{} B", meta.len())
                    };
                    output.push_str(&format!("  {}  ({})\n", path, size_str));
                } else {
                    output.push_str(&format!("  {}/\n", path));
                }
            } else {
                output.push_str(&format!("  {}\n", path));
            }
        }
        Ok(output)
    }
}

// ── LineEdit (structured line-level editor) ──

/// Single line-edit operation.
#[derive(Debug, Deserialize)]
#[serde(tag = "op")]
#[serde(rename_all = "snake_case")]
pub enum LineEditOp {
    InsertAfter {
        line: usize,
        text: String,
    },
    InsertBefore {
        line: usize,
        text: String,
    },
    ReplaceRange {
        start_line: usize,
        end_line: usize,
        text: String,
    },
    DeleteRange {
        start_line: usize,
        end_line: usize,
    },
}

#[derive(Deserialize)]
pub struct LineEditArgs {
    pub path: String,
    #[serde(default)]
    pub dry_run: bool,
    pub operations: Vec<LineEditOp>,
}

pub struct LineEdit;

impl Tool for LineEdit {
    const NAME: &'static str = "line_edit";

    type Error = ToolCallError;
    type Args = LineEditArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Apply a sequence of line-level edits to a file. ",
                "Operations are applied in order. All line numbers are 1-based ",
                "and reference the current file state (after previous operations). ",
                "Use dry_run=true to preview changes without writing."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["path", "operations"],
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (relative to project root)"
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, only validate and preview, don't write",
                        "default": false
                    },
                    "operations": {
                        "type": "array",
                        "description": "Operations to apply in sequence. Line numbers are relative (1-based).",
                        "minItems": 1,
                        "maxItems": 100,
                        "items": {
                            "oneOf": [
                                {
                                    "type": "object",
                                    "required": ["op", "line", "text"],
                                    "properties": {
                                        "op": { "type": "string", "enum": ["insert_after"], "description": "Insert text after the given line. line=0 means at start." },
                                        "line": { "type": "integer", "minimum": 0 },
                                        "text": { "type": "string" }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["op", "line", "text"],
                                    "properties": {
                                        "op": { "type": "string", "enum": ["insert_before"] },
                                        "line": { "type": "integer", "minimum": 1 },
                                        "text": { "type": "string" }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["op", "start_line", "end_line", "text"],
                                    "properties": {
                                        "op": { "type": "string", "enum": ["replace_range"] },
                                        "start_line": { "type": "integer", "minimum": 1 },
                                        "end_line": { "type": "integer", "minimum": 1 },
                                        "text": { "type": "string", "description": "Replacement text. Empty string deletes the range content (line becomes empty)." }
                                    }
                                },
                                {
                                    "type": "object",
                                    "required": ["op", "start_line", "end_line"],
                                    "properties": {
                                        "op": { "type": "string", "enum": ["delete_range"] },
                                        "start_line": { "type": "integer", "minimum": 1 },
                                        "end_line": { "type": "integer", "minimum": 1 }
                                    }
                                }
                            ]
                        }
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Read file
        let content = std::fs::read_to_string(&args.path)
            .map_err(|e| ToolCallError(format!("Cannot read '{}': {}", args.path, e)))?;

        // Reject binary files
        if !content.is_ascii() && content.contains('\0') {
            return Err(ToolCallError(format!(
                "'{}' appears to be a binary file (contains null bytes). Use write_file instead.",
                args.path
            )));
        }

        let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        // Preserve trailing newline semantics: track if file ends with newline
        let ends_with_newline = content.ends_with('\n');

        let mut stats = EditStats::default();
        let mut preview_lines = Vec::new();

        for (i, op) in args.operations.iter().enumerate() {
            let before_count = lines.len();
            match apply_operation(&mut lines, op, ends_with_newline) {
                Ok(desc) => {
                    stats.record(desc);
                    if args.dry_run {
                        let after_count = lines.len();
                        let change = if after_count > before_count {
                            format!("+{} lines", after_count - before_count)
                        } else if before_count > after_count {
                            format!("-{} lines", before_count - after_count)
                        } else {
                            "modified".to_string()
                        };
                        preview_lines.push(format!(
                            "  {}. {} (was {} lines → {} lines, {})",
                            i + 1,
                            desc,
                            before_count,
                            after_count,
                            change
                        ));
                    }
                }
                Err(e) => {
                    let err_msg = format!(
                        "Operation {} failed: {}. {} operations applied successfully before this error.",
                        i + 1,
                        e,
                        i
                    );
                    if args.dry_run {
                        preview_lines.push(format!("  {}. ERROR: {}", i + 1, e));
                        return Ok(format!(
                            "⚠️ Dry-run: {} operation(s) would fail\n\n{}",
                            args.operations.len() - i,
                            preview_lines.join("\n")
                        ));
                    }
                    // Partial rollback: restore original
                    let _ = std::fs::write(&args.path, &content);
                    return Err(ToolCallError(err_msg));
                }
            }
        }

        let new_content = if lines.is_empty() {
            String::new()
        } else {
            let mut result = lines.join("\n");
            if ends_with_newline {
                result.push('\n');
            }
            result
        };

        if args.dry_run {
            let preview_diff = generate_inline_diff(&content, &new_content);
            Ok(format!(
                "✅ Dry-run: {} would apply.\n\nChange preview:\n{}\n\nSummary:\n{}",
                if stats.total == 0 { "no changes" } else { "ok" },
                preview_diff,
                stats.summary()
            ))
        } else {
            // Atomic write: temp file + rename
            use std::io::Write;
            let tmp_path = format!("{}.tmp.{}", args.path, std::process::id());
            {
                let mut tmp = std::fs::File::create(&tmp_path)
                    .map_err(|e| ToolCallError(format!("Failed to create temp file: {}", e)))?;
                tmp.write_all(new_content.as_bytes())
                    .map_err(|e| ToolCallError(format!("Failed to write temp file: {}", e)))?;
                tmp.flush()
                    .map_err(|e| ToolCallError(format!("Failed to flush temp file: {}", e)))?;
            }
            std::fs::rename(&tmp_path, &args.path)
                .map_err(|e| ToolCallError(format!("Failed to rename temp file: {}", e)))?;

            Ok(format!(
                "✅ Applied {} operation(s) to '{}'.\n\nFile changed: {} lines → {} lines.\n\nPreview of changes:\n{}",
                stats.total,
                args.path,
                content.lines().count(),
                new_content.lines().count(),
                generate_inline_diff(&content, &new_content),
            ))
        }
    }
}

// ── Operation application logic ──

#[derive(Debug, Default)]
struct EditStats {
    inserts: usize,
    replaces: usize,
    deletes: usize,
    total: usize,
}

impl EditStats {
    fn record(&mut self, desc: &str) {
        self.total += 1;
        if desc.starts_with("insert") {
            self.inserts += 1;
        } else if desc.starts_with("replace") {
            self.replaces += 1;
        } else if desc.starts_with("delete") {
            self.deletes += 1;
        }
    }

    fn summary(&self) -> String {
        let parts: Vec<String> = [
            (self.inserts, "insert"),
            (self.replaces, "replace"),
            (self.deletes, "delete"),
        ]
        .iter()
        .filter_map(|(n, label)| {
            if *n > 0 {
                Some(format!("{} {}{}", n, label, if *n > 1 { "s" } else { "" }))
            } else {
                None
            }
        })
        .collect();
        format!("{} operation(s): {}", self.total, parts.join(", "))
    }
}

/// Apply a single operation to the current line buffer.
fn apply_operation(lines: &mut Vec<String>, op: &LineEditOp, _ends_with_newline: bool) -> Result<&'static str, String> {
    match op {
        LineEditOp::InsertAfter { line, text } => {
            let pos = if *line == 0 {
                0
            } else if *line > lines.len() {
                // If file is empty (0 lines) and line > 0, treat as append
                lines.len()
            } else {
                *line
            };
            let new_lines = text.lines().map(|l| l.to_string()).collect::<Vec<_>>();
            let splice_pos = if *line == 0 { 0 } else { pos.min(lines.len()) };
            let tail = lines.split_off(splice_pos);
            lines.extend(new_lines);
            lines.extend(tail);
            Ok("insert_after")
        }
        LineEditOp::InsertBefore { line, text } => {
            if *line == 0 {
                return Err(
                    "insert_before line:0 is invalid. Use insert_after line:0 or insert_before line:1.".to_string(),
                );
            }
            let pos = if *line > lines.len() {
                lines.len()
            } else {
                *line - 1 // 1-based to 0-based
            };
            let new_lines = text.lines().map(|l| l.to_string()).collect::<Vec<_>>();
            let tail = lines.split_off(pos);
            lines.extend(new_lines);
            lines.extend(tail);
            Ok("insert_before")
        }
        LineEditOp::ReplaceRange {
            start_line,
            end_line,
            text,
        } => {
            if start_line > end_line {
                return Err(format!("start_line ({}) > end_line ({})", start_line, end_line));
            }
            if *start_line == 0 || *start_line > lines.len() {
                return Err(format!(
                    "start_line {} is out of range (file has {} lines)",
                    start_line,
                    lines.len()
                ));
            }
            let start = *start_line - 1;
            let end = (*end_line).min(lines.len());
            if start >= lines.len() {
                return Err(format!(
                    "start_line {} is out of range (file has {} lines)",
                    start_line,
                    lines.len()
                ));
            }
            let new_lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
            lines.splice(start..end, new_lines);
            Ok("replace_range")
        }
        LineEditOp::DeleteRange { start_line, end_line } => {
            if start_line > end_line {
                return Err(format!("start_line ({}) > end_line ({})", start_line, end_line));
            }
            if *start_line == 0 || *start_line > lines.len() {
                return Err(format!(
                    "start_line {} is out of range (file has {} lines)",
                    start_line,
                    lines.len()
                ));
            }
            let start = *start_line - 1;
            let end = (*end_line).min(lines.len());
            if start >= lines.len() {
                return Err(format!(
                    "start_line {} is out of range (file has {} lines)",
                    start_line,
                    lines.len()
                ));
            }
            lines.splice(start..end, std::iter::empty::<String>());
            Ok("delete_range")
        }
    }
}

/// Generate a simple inline diff preview (shows added/removed lines).
fn generate_inline_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut output = String::new();

    // Simple line-by-line diff using similartext
    let mut i = 0;
    let mut j = 0;
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            // Unchanged
            if output.len() < 2000 {
                output.push_str(&format!("  {}\n", old_lines[i]));
            }
            i += 1;
            j += 1;
        } else if j < new_lines.len() && (i >= old_lines.len() || new_lines[j] != old_lines[i]) {
            // Added
            if output.len() < 2000 {
                output.push_str(&format!("+ {}\n", new_lines[j]));
            }
            j += 1;
        } else if i < old_lines.len() {
            // Removed
            if output.len() < 2000 {
                output.push_str(&format!("- {}\n", old_lines[i]));
            }
            i += 1;
        }
    }

    if output.len() >= 2000 {
        output.push_str("... (diff truncated at 2000 chars)\n");
    }
    output
}

// ── Fetch (web fetch) ──

#[derive(Deserialize)]
pub struct FetchArgs {
    pub url: String,
    #[serde(default)]
    pub max_size: Option<usize>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

pub struct Fetch;

impl Tool for Fetch {
    const NAME: &'static str = "fetch";

    type Error = ToolCallError;
    type Args = FetchArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Fetch a URL and return its content as plain text. ",
                "Supports HTTP(S). Max 10KB by default. ",
                "Use for reading web pages, APIs, or documentation."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "URL to fetch (http:// or https://)"
                    },
                    "max_size": {
                        "type": "integer",
                        "description": "Maximum response size in bytes (default: 10240, max: 102400)",
                        "optional": true
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "Request timeout in seconds (default: 30)",
                        "optional": true
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(args.timeout.unwrap_or(30)))
            .user_agent("Workflow-Agent/1.0")
            .build()
            .map_err(|e| ToolCallError(format!("Failed to create HTTP client: {}", e)))?;

        let response = client
            .get(&args.url)
            .send()
            .await
            .map_err(|e| ToolCallError(format!("Request failed: {}", e)))?;

        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let max_size = args.max_size.unwrap_or(10240).min(102400);

        let content = response
            .bytes()
            .await
            .map_err(|e| ToolCallError(format!("Failed to read response body: {}", e)))?;

        let bytes_len = content.len();
        let body_str = String::from_utf8_lossy(&content);

        let truncated = if bytes_len > max_size {
            // Truncate at char boundary to avoid panics
            let max_chars = max_size / 4; // conservative estimate (max 4 bytes per char)
            let truncated_body: String = body_str.chars().take(max_chars).collect();
            format!(
                "{}\n\n... [truncated, {} bytes, showing first {} bytes]",
                truncated_body, bytes_len, max_size
            )
        } else {
            body_str.to_string()
        };

        Ok(format!(
            "HTTP {} ({}, {} bytes, {})\n\n{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            bytes_len,
            content_type,
            truncated
        ))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolCallError(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

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

    #[tokio::test]
    async fn test_shell_stderr() {
        let result = (Shell { sandbox: None })
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

    // ═══════════════════════════════════════════════
    //  Grep
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_grep_basic() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "apple").unwrap();
        writeln!(f, "banana").unwrap();
        writeln!(f, "cherry").unwrap();
        writeln!(f, "date").unwrap();
        let result = Grep
            .call(GrepArgs {
                pattern: "^a.*".to_string(),
                path: f.path().to_str().unwrap().to_string(),
                max_results: None,
            })
            .await
            .unwrap();
        assert!(result.contains("apple"), "'apple' should match /^a.*/");
        assert!(!result.contains("cherry"), "'cherry' should not match");
        assert!(result.contains("1 match(es)"), "expected 1 match, got: {}", result);
    }

    #[tokio::test]
    async fn test_grep_no_match() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, "apple").unwrap();
        writeln!(f, "banana").unwrap();
        let result = Grep
            .call(GrepArgs {
                pattern: "zzz".to_string(),
                path: f.path().to_str().unwrap().to_string(),
                max_results: None,
            })
            .await
            .unwrap();
        assert!(result.contains("0 match"));
    }

    #[tokio::test]
    async fn test_grep_invalid_regex() {
        let result = Grep
            .call(GrepArgs {
                pattern: "[unclosed".to_string(),
                path: "/tmp".to_string(),
                max_results: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid regex"));
    }

    #[tokio::test]
    async fn test_grep_max_results() {
        let mut f = NamedTempFile::new().unwrap();
        for i in 0..20 {
            writeln!(f, "line-{}", i).unwrap();
        }
        let result = Grep
            .call(GrepArgs {
                pattern: "line".to_string(),
                path: f.path().to_str().unwrap().to_string(),
                max_results: Some(5),
            })
            .await
            .unwrap();
        assert!(result.contains("truncated at 5"));
        assert!(result.contains("5 match"));
    }

    #[tokio::test]
    async fn test_grep_directory_mode() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world\nfoo bar\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world hello\n").unwrap();
        let result = Grep
            .call(GrepArgs {
                pattern: "world".to_string(),
                path: dir.path().to_str().unwrap().to_string(),
                max_results: None,
            })
            .await
            .unwrap();
        assert!(result.contains("world"));
        assert!(result.contains("match"));
    }

    // ═══════════════════════════════════════════════
    //  FindFiles
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_find_files_rs_pattern() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub fn foo() {}").unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Readme").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/mod.rs"), "mod x;").unwrap();
        let result = FindFiles
            .call(FindFilesArgs {
                pattern: "*.rs".to_string(),
                root: Some(dir.path().to_str().unwrap().to_string()),
                max_results: None,
            })
            .await
            .unwrap();
        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
        assert!(result.contains("file(s)"), "result: {}", result);
        // readme.md shouldn't match
        assert!(!result.contains("readme.md"));
    }

    #[tokio::test]
    async fn test_find_files_recursive() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::create_dir_all(dir.path().join("src/sub")).unwrap();
        std::fs::write(dir.path().join("src/sub/b.rs"), "").unwrap();
        let result = FindFiles
            .call(FindFilesArgs {
                pattern: "**/*.rs".to_string(),
                root: Some(dir.path().to_str().unwrap().to_string()),
                max_results: None,
            })
            .await
            .unwrap();
        assert!(result.contains("a.rs"));
        assert!(result.contains("src/sub/b.rs"));
    }

    // ═══════════════════════════════════════════════
    //  MoveFile
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_move_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        std::fs::write(&src, "content").unwrap();
        let result = MoveFile
            .call(MoveFileArgs {
                source: src.to_str().unwrap().to_string(),
                destination: dst.to_str().unwrap().to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("Moved"));
        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[tokio::test]
    async fn test_move_file_nonexistent_source() {
        let result = MoveFile
            .call(MoveFileArgs {
                source: "/nonexistent/foo".to_string(),
                destination: "/tmp/bar".to_string(),
            })
            .await;
        assert!(result.is_err());
    }

    // ═══════════════════════════════════════════════
    //  CopyFile
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_copy_file_basic() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("copy.txt");
        std::fs::write(&src, "hello world").unwrap();
        let result = CopyFile
            .call(CopyFileArgs {
                source: src.to_str().unwrap().to_string(),
                destination: dst.to_str().unwrap().to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("Copied"));
        assert!(result.contains("11 bytes"));
        assert!(src.exists());
        assert!(dst.exists());
    }

    #[tokio::test]
    async fn test_copy_file_nonexistent_source() {
        let result = CopyFile
            .call(CopyFileArgs {
                source: "/nonexistent/foo".to_string(),
                destination: "/tmp/bar".to_string(),
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_copy_file_directory() {
        let dir = TempDir::new().unwrap();
        let src_dir = dir.path().join("src");
        let dst_dir = dir.path().join("dst");
        std::fs::create_dir(&src_dir).unwrap();
        std::fs::write(src_dir.join("a.txt"), "aaa").unwrap();
        std::fs::write(src_dir.join("b.txt"), "bbb").unwrap();
        let result = CopyFile
            .call(CopyFileArgs {
                source: src_dir.to_str().unwrap().to_string(),
                destination: dst_dir.to_str().unwrap().to_string(),
            })
            .await
            .unwrap();
        assert!(result.contains("Copied directory"));
        assert!(dst_dir.join("a.txt").exists());
        assert!(dst_dir.join("b.txt").exists());
    }

    // ═══════════════════════════════════════════════
    //  DeleteFile
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_delete_file_basic() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "content").unwrap();
        let result = DeleteFile
            .call(DeleteFileArgs {
                path: file.to_str().unwrap().to_string(),
                recursive: None,
                force: None,
            })
            .await
            .unwrap();
        assert!(result.contains("Deleted"));
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn test_delete_file_nonexistent() {
        let result = DeleteFile
            .call(DeleteFileArgs {
                path: "/nonexistent/foo".to_string(),
                recursive: None,
                force: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_directory_without_recursive() {
        let dir = TempDir::new().unwrap();
        let result = DeleteFile
            .call(DeleteFileArgs {
                path: dir.path().to_str().unwrap().to_string(),
                recursive: None,
                force: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("recursive"));
    }

    #[tokio::test]
    async fn test_delete_directory_recursive() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "data").unwrap();
        let result = DeleteFile
            .call(DeleteFileArgs {
                path: dir.path().to_str().unwrap().to_string(),
                recursive: Some(true),
                force: None,
            })
            .await
            .unwrap();
        assert!(result.contains("Deleted directory"));
        assert!(!dir.path().exists());
    }

    #[tokio::test]
    async fn test_delete_file_safety_rejects_system_path() {
        let result = DeleteFile
            .call(DeleteFileArgs {
                path: "/etc".to_string(),
                recursive: None,
                force: None,
            })
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Refusing"));
    }

    // ═══════════════════════════════════════════════
    //  AppendFile
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_append_file_new() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt").to_str().unwrap().to_string();
        let result = AppendFile
            .call(AppendFileArgs {
                path: path.clone(),
                content: "hello".to_string(),
                newline: Some(false),
            })
            .await
            .unwrap();
        assert!(result.contains("Appended"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn test_append_file_existing_with_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "first").unwrap();
        let result = AppendFile
            .call(AppendFileArgs {
                path: path.to_str().unwrap().to_string(),
                content: "second".to_string(),
                newline: Some(true),
            })
            .await
            .unwrap();
        assert!(result.contains("Appended"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "first\nsecond");
    }

    #[tokio::test]
    async fn test_append_file_existing_no_newline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "base").unwrap();
        let result = AppendFile
            .call(AppendFileArgs {
                path: path.to_str().unwrap().to_string(),
                content: "append".to_string(),
                newline: Some(false),
            })
            .await
            .unwrap();
        assert!(result.contains("Appended"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "baseappend");
    }

    // ═══════════════════════════════════════════════
    //  PatchFile
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_patch_file_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn old_name() {}\nfn old_name() {}").unwrap();
        let result = PatchFile
            .call(PatchFileArgs {
                path: path.to_str().unwrap().to_string(),
                old_text: "old_name".to_string(),
                new_text: "new_name".to_string(),
                count: None,
            })
            .await
            .unwrap();
        assert!(result.contains("2 replacement"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("old_name"));
        assert_eq!(content, "fn new_name() {}\nfn new_name() {}");
    }

    #[tokio::test]
    async fn test_patch_file_limited_count() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "aaa aaa aaa").unwrap();
        let result = PatchFile
            .call(PatchFileArgs {
                path: path.to_str().unwrap().to_string(),
                old_text: "aaa".to_string(),
                new_text: "bbb".to_string(),
                count: Some(2),
            })
            .await
            .unwrap();
        assert!(result.contains("2 replacement"));
        assert!(result.contains("1 skipped"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "bbb bbb aaa");
    }

    #[tokio::test]
    async fn test_patch_file_not_found() {
        let result = PatchFile
            .call(PatchFileArgs {
                path: "/nonexistent/file.rs".to_string(),
                old_text: "foo".to_string(),
                new_text: "bar".to_string(),
                count: None,
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_patch_file_old_text_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world").unwrap();
        let result = PatchFile
            .call(PatchFileArgs {
                path: path.to_str().unwrap().to_string(),
                old_text: "nonexistent".to_string(),
                new_text: "replacement".to_string(),
                count: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // ═══════════════════════════════════════════════
    //  Glob
    // ═══════════════════════════════════════════════

    #[tokio::test]
    async fn test_glob_basic() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("readme.md"), "# title").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}").unwrap();
        let result = Glob
            .call(GlobArgs {
                pattern: "*.toml".to_string(),
                root: Some(dir.path().to_str().unwrap().to_string()),
            })
            .await
            .unwrap();
        assert!(result.contains("cargo.toml"));
        assert!(!result.contains("readme.md"));
        assert!(result.contains("1 match"));
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/file.rs"), "").unwrap();
        let result = Glob
            .call(GlobArgs {
                pattern: "**/*.rs".to_string(),
                root: Some(dir.path().to_str().unwrap().to_string()),
            })
            .await
            .unwrap();
        assert!(result.contains("file.rs"));
        assert!(result.contains("1 match"));
    }

    #[tokio::test]
    async fn test_glob_no_match() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "").unwrap();
        let result = Glob
            .call(GlobArgs {
                pattern: "*.rs".to_string(),
                root: Some(dir.path().to_str().unwrap().to_string()),
            })
            .await
            .unwrap();
        assert!(result.contains("0 match"));
    }

    #[tokio::test]
    async fn test_glob_invalid_pattern() {
        let result = Glob
            .call(GlobArgs {
                pattern: "[invalid".to_string(),
                root: None,
            })
            .await;
        assert!(result.is_err());
    }
}
