//! Tool that provides information about the currently opened project.
//!
//! Agents can use this tool to learn about the project they are working on,
//! including its name, path, and directory structure.

use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use workflow_config::ProjectConfig;

#[derive(Debug, Deserialize)]
pub struct GetProjectInfoArgs {
    /// Optional depth to scan (default: 1, max: 3).
    #[serde(default)]
    pub depth: Option<usize>,
    /// Optional glob pattern to filter files (e.g. "**/*.rs", "**/Cargo.toml").
    #[serde(default)]
    pub pattern: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectInfo {
    /// Human-readable project name.
    pub name: String,
    /// Absolute path to the project root.
    pub path: String,
    /// Total number of directories.
    pub total_dirs: usize,
    /// Total number of files.
    pub total_files: usize,
    /// Top-level directory listing.
    pub structure: Vec<FileEntry>,
    /// Error message if scanning failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileEntry>>,
}

/// Errors that can occur when querying project info.
#[derive(Debug, Error)]
pub enum ProjectToolError {
    #[error("No project opened. Use --project flag when starting workflow.")]
    NoProject,
    #[error("Serialization error: {0}")]
    Serialize(String),
}

/// Tool that returns information about the currently opened project.
pub struct GetProjectInfo {
    project: Arc<Option<ProjectConfig>>,
}

impl GetProjectInfo {
    pub fn new(project: Arc<Option<ProjectConfig>>) -> Self {
        Self { project }
    }
}

impl Tool for GetProjectInfo {
    const NAME: &'static str = "get_project_info";

    type Error = ProjectToolError;
    type Args = GetProjectInfoArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "get_project_info".to_string(),
            description: "Get information about the currently opened project, \
                including its name, path, and directory structure. \
                Use this to explore the project layout before working with files. \
                Optionally specify a glob pattern to filter files or a depth for subdirectories."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "depth": {
                        "type": "integer",
                        "description": "Depth of directory scanning (default: 1, max: 3)"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Optional glob pattern to filter files (e.g. '**/*.rs', '**/Cargo.toml')"
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let project = self
            .project
            .as_ref()
            .as_ref()
            .ok_or(ProjectToolError::NoProject)?;

        let depth = args.depth.unwrap_or(1).min(3);

        let info = scan_project(project, depth, args.pattern.as_deref());

        serde_json::to_string_pretty(&info).map_err(|e| ProjectToolError::Serialize(e.to_string()))
    }
}

fn scan_project(project: &ProjectConfig, depth: usize, pattern: Option<&str>) -> ProjectInfo {
    let path = std::path::Path::new(&project.path);
    if !path.exists() {
        return ProjectInfo {
            name: project.name().to_string(),
            path: project.path.clone(),
            total_dirs: 0,
            total_files: 0,
            structure: Vec::new(),
            error: Some("Project path does not exist".to_string()),
        };
    }

    let mut total_dirs = 0usize;
    let mut total_files = 0usize;

    let structure = if depth > 0 {
        scan_directory(path, depth, pattern, &mut total_dirs, &mut total_files).unwrap_or_default()
    } else {
        // Depth 0: just top-level names.
        let mut entries = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten() {
                let ft = entry.file_type().ok();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name == "node_modules" || name == "target" {
                    continue;
                }
                if let Some(p) = pattern
                    && !glob_match(p, &name)
                {
                    continue;
                }
                let entry_type = if ft.map(|t| t.is_dir()).unwrap_or(false) {
                    total_dirs += 1;
                    "directory"
                } else {
                    total_files += 1;
                    "file"
                };
                entries.push(FileEntry {
                    name,
                    entry_type: entry_type.to_string(),
                    size: None,
                    children: None,
                });
            }
        }
        entries
    };

    ProjectInfo {
        name: project.name().to_string(),
        path: project.path.clone(),
        total_dirs,
        total_files,
        structure,
        error: None,
    }
}

fn scan_directory(
    dir: &std::path::Path,
    remaining_depth: usize,
    pattern: Option<&str>,
    total_dirs: &mut usize,
    total_files: &mut usize,
) -> Result<Vec<FileEntry>, std::io::Error> {
    let mut read_dir = std::fs::read_dir(dir)?;
    let mut dir_entries: Vec<_> = read_dir
        .by_ref()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            !name.starts_with('.') && name != "node_modules" && name != "target"
        })
        .collect();
    dir_entries.sort_by_key(|e| e.file_name());

    let mut entries = Vec::new();
    for entry in dir_entries {
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry.file_type()?;

        if ft.is_dir() {
            *total_dirs += 1;
            let children = if remaining_depth > 1 {
                scan_directory(
                    &entry.path(),
                    remaining_depth - 1,
                    pattern,
                    total_dirs,
                    total_files,
                )
                .unwrap_or_default()
            } else {
                Vec::new()
            };

            if pattern.is_none_or(|p| glob_match(p, &name)) || !children.is_empty() {
                entries.push(FileEntry {
                    name,
                    entry_type: "directory".to_string(),
                    size: None,
                    children: Some(children),
                });
            }
        } else {
            if pattern.is_none_or(|p| glob_match(p, &name)) {
                *total_files += 1;
                let size = entry.metadata().ok().map(|m| m.len());
                entries.push(FileEntry {
                    name,
                    entry_type: "file".to_string(),
                    size,
                    children: None,
                });
            }
        }
    }

    Ok(entries)
}

/// Very simple glob-like matching (supports `*`, `**`).
fn glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == "**/*" {
        return true;
    }
    if pattern == name {
        return true;
    }
    // Simple wildcard: "*.rs" matches "foo.rs"
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("Cargo*", "Cargo.toml"));
        assert!(glob_match("main.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.js"));
        assert!(glob_match("*", "anything.txt"));
    }
}
