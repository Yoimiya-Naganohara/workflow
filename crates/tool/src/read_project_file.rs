//! Tool that reads files from the opened project directory.
//!
//! Agents can use this tool to read source files, configuration files,
//! and other project content. Path traversal attacks are prevented by
//! ensuring all resolved paths stay within the project root.

use std::{path::PathBuf, sync::Arc};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use thiserror::Error;
use workflow_config::ProjectConfig;

#[derive(Debug, Deserialize)]
pub struct ReadProjectFileArgs {
    /// Path relative to the project root (e.g. "src/main.rs", "Cargo.toml").
    pub path: String,
    /// Maximum number of bytes to read (default: 32768, max: 524288).
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

/// Errors that can occur when reading a project file.
#[derive(Debug, Error)]
pub enum ReadFileError {
    #[error("No project opened. Use --project flag when starting workflow.")]
    NoProject,
    #[error("Failed to resolve project path: {0}")]
    PathResolution(String),
    #[error("Path '{0}' resolves outside the project root")]
    PathTraversal(String),
    #[error("File not found: '{0}'")]
    NotFound(String),
    #[error("'{0}' is not a file")]
    NotAFile(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Binary file too large")]
    BinaryTooLarge,
}

/// Tool that reads a file from the opened project.
///
/// Path traversal is prevented: the resolved path must be within the
/// project root directory.
pub struct ReadProjectFile {
    project: Arc<Option<ProjectConfig>>,
}

impl ReadProjectFile {
    pub fn new(project: Arc<Option<ProjectConfig>>) -> Self {
        Self { project }
    }
}

impl Tool for ReadProjectFile {
    const NAME: &'static str = "read_project_file";

    type Error = ReadFileError;
    type Args = ReadProjectFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "read_project_file".to_string(),
            description: "Read a file from the opened project. \
                Provide the path relative to the project root. \
                Use get_project_info first to explore the project structure. \
                Binary files will return a base64-encoded string."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path relative to project root (e.g. 'src/main.rs', 'Cargo.toml')"
                    },
                    "max_bytes": {
                        "type": "integer",
                        "description": "Maximum bytes to read (default: 32768, max: 524288)"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let project = self
            .project
            .as_ref()
            .as_ref()
            .ok_or(ReadFileError::NoProject)?;

        let project_root = std::fs::canonicalize(&project.path)
            .map_err(|e| ReadFileError::PathResolution(e.to_string()))?;

        let requested = PathBuf::from(&args.path);
        let resolved = project_root.join(&requested);

        // Canonicalize the resolved path to detect `..` traversal.
        let canonical_resolved = std::fs::canonicalize(&resolved)
            .map_err(|e| ReadFileError::Io(format!("Cannot resolve '{}': {e}", args.path)))?;

        // Path traversal protection: resolved path must be under project root.
        if !canonical_resolved.starts_with(&project_root) {
            return Err(ReadFileError::PathTraversal(args.path));
        }

        if !resolved.exists() {
            return Err(ReadFileError::NotFound(args.path));
        }

        if !resolved.is_file() {
            return Err(ReadFileError::NotAFile(args.path));
        }

        let max_bytes = args.max_bytes.unwrap_or(32_768).min(524_288);

        let metadata =
            std::fs::metadata(&canonical_resolved).map_err(|e| ReadFileError::Io(e.to_string()))?;

        // Check if file is likely binary.
        let is_binary = is_probably_binary(&resolved);

        if is_binary && metadata.len() > max_bytes as u64 {
            return Err(ReadFileError::BinaryTooLarge);
        }

        let content = if is_binary {
            let bytes = read_with_limit(&canonical_resolved, max_bytes)?;
            format!(
                "[Binary file: {}, {} bytes]\nBase64:\n{}",
                args.path,
                metadata.len(),
                base64_encode(&bytes)
            )
        } else {
            let bytes = read_with_limit(&resolved, max_bytes)?;
            let text = String::from_utf8_lossy(&bytes);
            if bytes.len() == max_bytes && metadata.len() > max_bytes as u64 {
                format!(
                    "... (truncated at {} of {} bytes)\n{}",
                    max_bytes,
                    metadata.len(),
                    text
                )
            } else {
                text.to_string()
            }
        };

        Ok(content)
    }
}

fn read_with_limit(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, ReadFileError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| ReadFileError::Io(e.to_string()))?;
    let mut buf = vec![0u8; limit];
    let n = file
        .read(&mut buf)
        .map_err(|e| ReadFileError::Io(e.to_string()))?;
    buf.truncate(n);
    Ok(buf)
}

fn is_probably_binary(path: &std::path::Path) -> bool {
    if let Some(ext) = path.extension() {
        let ext = ext.to_string_lossy().to_lowercase();
        matches!(
            ext.as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "bmp"
                | "ico"
                | "webp"
                | "svg"
                | "pdf"
                | "zip"
                | "tar"
                | "gz"
                | "bz2"
                | "xz"
                | "7z"
                | "rar"
                | "exe"
                | "dll"
                | "so"
                | "dylib"
                | "bin"
                | "wasm"
                | "mp3"
                | "mp4"
                | "avi"
                | "mov"
                | "mkv"
                | "woff"
                | "woff2"
                | "ttf"
                | "eot"
                | "o"
                | "pyc"
                | "class"
                | "dex"
                | "apk"
                | "aab"
                | "keystore"
        )
    } else {
        false
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_traversal_detected() {
        // Verify that a path with `..` traversal is NOT considered safe.
        let project_root = std::path::Path::new("/tmp/test-project");
        let bad_path = project_root.join("../etc/passwd");
        // String-wise starts_with passes, but component-wise canonicalization
        // should be used. This test verifies the raw join doesn't fool us.
        assert!(
            bad_path.starts_with(project_root),
            "without canonicalization, '../etc/passwd' still appears to start with the project root"
        );
        // After canonicalization this would resolve outside the project root.
        let canonical = std::path::Path::new("/tmp/etc/passwd");
        assert!(!canonical.starts_with(project_root));
    }

    #[test]
    fn test_is_binary() {
        assert!(is_probably_binary(std::path::Path::new("image.png")));
        assert!(is_probably_binary(std::path::Path::new("archive.zip")));
        assert!(!is_probably_binary(std::path::Path::new("main.rs")));
        assert!(!is_probably_binary(std::path::Path::new("Cargo.toml")));
    }
}
