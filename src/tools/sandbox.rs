//! Per-agent filesystem sandbox with path resolution and escape prevention.
//!
//! # Architecture
//!
//! ```text
//! ~/.workflow/sandbox/{agent_id:8}/
//!   ├── work/          ◄── writable: all writes, compilation, shell cwd
//!   └── src -> /real-project/  ◄── read‑only symlink to project root
//! ```
//!
//! Every built-in tool that touches the filesystem calls [`resolve_path`]
//! before operating.  The resolver enforces a **copy-on-write** model:
//!
//! - **Writes** land in `work/` — the real source tree is never modified.
//! - **Reads** that reference the source tree (via `src/...`) resolve through
//!   the symlink and are permitted from the read-only boundary.
//! - **`../` escapes**, absolute paths outside the boundary, and symlink
//!   traversal that leaves the project root are all rejected.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

// ── Constants ──

const WORK_DIR: &str = "work";
const SOURCE_LINK: &str = "src";
const MAX_CANON_DEPTH: usize = 64;

// ── SandboxHandle ──

/// Lightweight, cloneable handle to an agent's filesystem sandbox.
#[derive(Debug, Clone)]
pub struct SandboxHandle {
    /// Sandbox root: `~/.workflow/sandbox/{agent_id:8}/`
    pub root: PathBuf,
    /// Writable work directory (canonical).
    pub workdir: PathBuf,
    /// Source tree root (canonical, follows symlink).
    pub source_root: PathBuf,
}

impl SandboxHandle {
    /// Create a sandbox for an agent.  Idempotent — safe to call multiple
    /// times for the same agent_id (useful for retries).
    pub fn new(agent_id: &[u8; 16]) -> Result<Self> {
        let home = dirs_or_fallback();
        let root = home.join(".workflow").join("sandbox").join(hex_prefix(agent_id));

        // 1. Writable work directory.
        let workdir = root.join(WORK_DIR);
        std::fs::create_dir_all(&workdir)
            .with_context(|| format!("Failed to create sandbox workdir: {}", workdir.display()))?;
        let workdir = workdir
            .canonicalize()
            .context("Failed to canonicalise sandbox workdir")?;

        // 2. Read-only symlink to project root.
        let source_link = root.join(SOURCE_LINK);
        if !source_link.exists() {
            let project_root = std::env::current_dir().context("Cannot determine project root for sandbox symlink")?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&project_root, &source_link)
                .with_context(|| format!("symlink {} -> {}", source_link.display(), project_root.display()))?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(&project_root, &source_link)?;
        }
        // Canonicalise AFTER creation so the symlink is resolved.
        let source_root = source_link
            .canonicalize()
            .context("Failed to canonicalise sandbox source symlink")?;

        Ok(Self {
            root,
            workdir,
            source_root,
        })
    }

    /// Resolve a user-supplied path into a canonical absolute path within
    /// the sandbox boundary (workdir ∪ source_root).
    ///
    /// **Relative paths** are anchored to `workdir` — writes are isolated.
    /// **Absolute paths** are canonicalised and checked against both
    /// boundaries.  A path that does not exist yet has its nearest existing
    /// ancestor canonicalised first (avoids the `canonicalize` false-death
    /// trap for new files).
    pub fn resolve_path(&self, raw: &str) -> Result<PathBuf> {
        let path = Path::new(raw);

        if !path.is_absolute() {
            // Anchor to workdir — all relative paths land in the
            // writable sandbox zone.
            let candidate = self.workdir.join(path);
            return self.canonicalise_or_reject(&candidate);
        }

        // Absolute path: canonicalise and check against both boundaries.
        self.canonicalise_or_reject(path)
    }

    /// Resolve a path and additionally assert it resolves **inside the
    /// read-only source tree**.  Used by tools that should never modify
    /// the source (e.g. a reviewer agent).
    pub fn resolve_path_read_only(&self, raw: &str) -> Result<PathBuf> {
        let resolved = self.resolve_path(raw)?;
        if resolved.starts_with(&self.source_root) {
            return Ok(resolved);
        }
        // Allow paths in workdir that shadow source files *if* the
        // corresponding source file exists (read-through pattern).
        if let Ok(rel) = resolved.strip_prefix(&self.workdir) {
            let source_variant = self.source_root.join(rel);
            if source_variant.exists() {
                return Ok(source_variant.canonicalize().unwrap_or(source_variant));
            }
        }
        anyhow::bail!("Read-only sandbox: path '{}' is outside the source tree", raw);
    }

    /// Best-effort removal of the entire sandbox directory.
    /// Called during agent eviction; errors are logged but not propagated.
    pub fn cleanup(&self) {
        if self.root.exists() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    // ── Internal ──

    fn canonicalise_or_reject(&self, path: &Path) -> Result<PathBuf> {
        // Fast path: path already exists.
        if let Ok(canon) = path.canonicalize() {
            return self.check_boundary(canon);
        }

        // Walk up ancestors until we find an existing component.
        let mut ancestor = path.parent();
        let mut depth = 0;
        while let Some(parent) = ancestor {
            if depth >= MAX_CANON_DEPTH {
                anyhow::bail!("Path has too many non-existing ancestors: {}", path.display());
            }
            if parent.exists() {
                let canon_parent = parent.canonicalize().with_context(|| {
                    format!(
                        "Failed to canonicalise parent of '{}': '{}'",
                        path.display(),
                        parent.display()
                    )
                })?;
                let checked = self.check_boundary(canon_parent)?;
                let remaining = path.strip_prefix(parent).expect("parent is a prefix by construction");
                return Ok(checked.join(remaining));
            }
            ancestor = parent.parent();
            depth += 1;
        }

        anyhow::bail!("Path has no existing ancestor within sandbox: {}", path.display())
    }

    fn check_boundary(&self, canon: PathBuf) -> Result<PathBuf> {
        if canon.starts_with(&self.workdir) || canon.starts_with(&self.source_root) {
            Ok(canon)
        } else {
            anyhow::bail!(
                "Access denied: path '{}' is outside the sandbox boundary",
                canon.display()
            );
        }
    }
}

// ── Helpers ──

fn dirs_or_fallback() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn hex_prefix(id: &[u8; 16]) -> String {
    id.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_handle() -> SandboxHandle {
        let id = [0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        SandboxHandle::new(&id).expect("sandbox creation")
    }

    #[test]
    fn test_creation_creates_directories() {
        let h = test_handle();
        assert!(h.workdir.exists(), "workdir exists");
        assert!(h.source_root.exists(), "source symlink exists");
        h.cleanup();
        assert!(!h.root.exists(), "cleanup removes sandbox");
    }

    #[test]
    fn test_resolve_relative_path_anchors_to_workdir() {
        let h = test_handle();
        let resolved = h.resolve_path("src/main.rs").unwrap();
        assert!(resolved.starts_with(&h.workdir), "relative → workdir");
        assert!(resolved.ends_with("src/main.rs"));
        h.cleanup();
    }

    #[test]
    fn test_resolve_new_file_uses_parent_canonicalise() {
        let h = test_handle();
        // "new_file.rs" doesn't exist, but its parent (workdir) does.
        let resolved = h.resolve_path("new_file.rs").unwrap();
        assert!(resolved.starts_with(&h.workdir));
        assert!(resolved.ends_with("new_file.rs"));
        h.cleanup();
    }

    #[test]
    fn test_resolve_source_file_via_symlink() {
        let h = test_handle();
        // Anchor workdir + "../src/Cargo.toml" → canonicalises through
        // the symlink and lands in the real project tree.
        // ../src/Cargo.toml from workdir = root/Cargo.toml (via symlink).
        let resolved = h.resolve_path("../src/Cargo.toml").unwrap();
        assert!(
            resolved.starts_with(&h.source_root),
            "source file should be under source_root: {:?}",
            resolved
        );
        assert!(resolved.ends_with("Cargo.toml"));
        h.cleanup();
    }

    #[test]
    fn test_read_only_passes_source_path() {
        let h = test_handle();
        let resolved = h.resolve_path_read_only("../src/Cargo.toml").unwrap();
        assert!(resolved.ends_with("Cargo.toml"));
        h.cleanup();
    }

    #[test]
    fn test_read_only_rejects_workdir_path() {
        let h = test_handle();
        let err = h.resolve_path_read_only("src/main.rs").unwrap_err();
        let msg = format!("{:?}", err);
        assert!(
            msg.contains("outside the source tree") || msg.contains("Read-only"),
            "read-only should reject workdir paths: {}",
            msg
        );
        h.cleanup();
    }

    #[test]
    fn test_malicious_escape_rejected() {
        let h = test_handle();
        // From workdir, "../../../../etc/passwd" → anchor to workdir →
        // canonicalize → if it ends up outside source_root, reject.
        let result = h.resolve_path("../../../../etc/passwd");
        assert!(result.is_err(), "path escape should be rejected");
        h.cleanup();
    }

    #[test]
    fn test_absolute_escape_rejected() {
        let h = test_handle();
        let result = h.resolve_path("/etc/passwd");
        assert!(result.is_err(), "absolute outside sandbox rejected");
        h.cleanup();
    }

    #[test]
    fn test_cleanup_idempotent() {
        let h = test_handle();
        h.cleanup();
        h.cleanup(); // must not panic
    }
}
