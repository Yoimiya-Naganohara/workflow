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
//! Every built-in tool that touches the filesystem calls `resolve_path`
//! before operating.  The resolver enforces a **copy-on-write** model:
//!
//! - **Writes** land in `work/` — the real source tree is never modified.
//! - **Reads** that reference the source tree (via `src/...`) resolve through
//!   the symlink and are permitted from the read-only boundary.
//! - **`../` escapes**, absolute paths outside the boundary, and symlink
//!   traversal that leaves the project root are all rejected.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use crate::core::simd::cosine_similarity_384;
use crate::llm::EmbeddingService;

// ── Constants ──

const WORK_DIR: &str = "work";
const SOURCE_LINK: &str = "src";
const MAX_CANON_DEPTH: usize = 64;

// ── AssetIndex — in-memory semantic chunk index ──

/// A semantic slice index for one dynamically generated asset.
/// Built during `create_embedded_asset()` and destroyed with the sandbox.
#[derive(Clone, Debug)]
pub struct AssetIndex {
    pub chunks: Vec<IndexedChunk>,
}

#[derive(Clone, Debug)]
pub struct IndexedChunk {
    pub embedding: [f32; 384],
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
}

impl AssetIndex {
    /// Slice `raw_content` into `chunk_lines`-line blocks and
    /// compute a 384-d embedding for each block using the project's
    /// local fastembed (ONNX) engine.
    pub async fn build(
        model: &dyn EmbeddingService,
        raw_content: &str,
        chunk_lines: usize,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let lines: Vec<&str> = raw_content.lines().collect();
        let mut chunks = Vec::new();
        let chunk_lines = if chunk_lines == 0 { 20 } else { chunk_lines };

        for (chunk_idx, line_group) in lines.chunks(chunk_lines).enumerate() {
            let text = line_group.join("\n");
            if text.trim().is_empty() {
                continue;
            }

            let embedding = model.embed(&text).await?;

            let line_start = chunk_idx * chunk_lines + 1;
            let line_end = (line_start + line_group.len() - 1).min(lines.len());

            chunks.push(IndexedChunk {
                embedding,
                line_start,
                line_end,
                text,
            });
        }

        Ok(Self { chunks })
    }

    /// Semantic search using AVX2+FMA SIMD cosine similarity.
    /// Returns top-K (score > 0.5) chunks with their display line numbers.
    pub fn search(&self, query_emb: &[f32; 384], k: usize) -> Vec<(usize, String)> {
        let mut scored: Vec<(f32, usize)> = self
            .chunks
            .iter()
            .enumerate()
            .map(|(i, c)| (cosine_similarity_384(query_emb, &c.embedding), i))
            .collect();

        // partial_cmp is safe because SIMD output is always a valid f32
        // (never NaN unless inputs are NaN, which we ensure by construction)
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .iter()
            .take(k)
            .filter(|(score, _)| *score > 0.5)
            .map(|(_, i)| {
                let c = &self.chunks[*i];
                (
                    c.line_start,
                    format!("[{}:{}]\n{}", c.line_start, c.line_end, c.text),
                )
            })
            .collect()
    }
}

// ── SandboxHandle ──

/// Lightweight, cloneable handle to an agent's filesystem sandbox.
///
/// Now carries a local embedding engine (`embedder`) and a pool of
/// in-memory semantic asset indices for the "semantic asset retrieval"
/// pattern — large Shell / ReadFile outputs are indexed on creation
/// and queried via `search_asset()` without polluting the LLM context.
pub struct SandboxHandle {
    /// Sandbox root: `~/.workflow/sandbox/{agent_id:8}/`
    pub root: PathBuf,
    /// Writable work directory (canonical).
    pub workdir: PathBuf,
    /// Source tree root (canonical, follows symlink).
    pub source_root: PathBuf,
    /// Local embedding engine for on-demand asset indexing.
    /// Injected at runtime via `attach_embedder`; absent = no indexing.
    pub embedder: RwLock<Option<Arc<dyn EmbeddingService>>>,
    /// In-memory semantic asset indices keyed by `asset_id`.
    pub asset_indices: RwLock<HashMap<String, AssetIndex>>,
}

impl std::fmt::Debug for SandboxHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxHandle")
            .field("root", &self.root)
            .field("workdir", &self.workdir)
            .field("source_root", &self.source_root)
            .field(
                "embedder",
                &self.embedder.read().map(|g| g.is_some()).unwrap_or(false),
            )
            .field(
                "asset_indices",
                &self.asset_indices.read().map(|g| g.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl Clone for SandboxHandle {
    fn clone(&self) -> Self {
        let embedder = self
            .embedder
            .read()
            .expect("sandbox mutex poisoned")
            .clone();
        let indices = self
            .asset_indices
            .read()
            .expect("sandbox mutex poisoned")
            .clone();
        Self {
            root: self.root.clone(),
            workdir: self.workdir.clone(),
            source_root: self.source_root.clone(),
            embedder: RwLock::new(embedder),
            asset_indices: RwLock::new(indices),
        }
    }
}

impl SandboxHandle {
    /// Create a sandbox for an agent.  Idempotent — safe to call multiple
    /// times for the same agent_id (useful for retries).
    pub fn new(agent_id: &[u8; 16]) -> Result<Self> {
        let home = dirs_or_fallback();
        let root = home
            .join(".workflow")
            .join("sandbox")
            .join(hex_prefix(agent_id));

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
            let project_root = std::env::current_dir()
                .context("Cannot determine project root for sandbox symlink")?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&project_root, &source_link).with_context(|| {
                format!(
                    "symlink {} -> {}",
                    source_link.display(),
                    project_root.display()
                )
            })?;
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
            embedder: RwLock::new(None),
            asset_indices: RwLock::new(HashMap::new()),
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
        anyhow::bail!(
            "Read-only sandbox: path '{}' is outside the source tree",
            raw
        );
    }

    /// Resolve a path for **write** operations.
    ///
    /// Same as `resolve_path` but with an additional safety check:
    /// the resolved path must land in the **writable workdir**, not the
    /// read-only source tree.  This prevents a hallucinating agent from
    /// overwriting project source files via absolute paths that resolve
    /// through the `src` symlink into the real project root.
    ///
    /// Agents without a sandbox bypass this check (no isolation), but
    /// the sandbox path is the primary execution path for spawned agents.
    pub fn resolve_write_path(&self, raw: &str) -> Result<PathBuf> {
        let resolved = self.resolve_path(raw)?;
        if !resolved.starts_with(&self.workdir) {
            anyhow::bail!(
                "Write access denied: path '{}' resolves to '{}' which is outside the \
                 writable workdir '{}'. Use relative paths for write operations; \
                 absolute paths are rejected by the sandbox write guard.",
                raw,
                resolved.display(),
                self.workdir.display()
            );
        }
        Ok(resolved)
    }

    /// Best-effort removal of the entire sandbox directory.
    /// Called during agent eviction; errors are logged but not propagated.
    pub fn cleanup(&self) {
        if self.root.exists() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// Dynamically attach the locally-available embedding engine.
    /// Called once when the agent runtime wires the sandbox into the
    /// tool server — `None` means `create_embedded_asset` falls back
    /// to plain physical writes (no indexing).
    pub fn attach_embedder(&self, embedder: Arc<dyn EmbeddingService>) {
        let mut guard = self.embedder.write().expect("sandbox mutex poisoned");
        *guard = Some(embedder);
    }

    /// Path to the asset artifact dir inside the sandbox workdir.
    fn artifacts_dir(&self) -> PathBuf {
        let path = self.workdir.join(".artifacts");
        if !path.exists() {
            let _ = std::fs::create_dir_all(&path);
        }
        path
    }

    /// Core replacement for blind `std::fs::write`: persist the raw
    /// content to the sandbox AND build a semantic index in memory.
    ///
    /// # Returns
    /// A compact handle string that tells the LLM about the asset and
    /// instructs it to use `search_asset(asset_id, query)` for precise
    /// retrieval — zero large-text leakage into the conversation history.
    pub async fn create_embedded_asset(
        &self,
        tool_name: &str,
        raw_content: &str,
    ) -> std::result::Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Deterministic hash → asset_id
        let mut hasher = DefaultHasher::new();
        raw_content.hash(&mut hasher);
        let asset_id = format!("{}_{:x}", tool_name.to_lowercase(), hasher.finish());

        // 2. Persist to physical sandbox (for fallback ReadFile / PatchFile)
        let asset_path = self.artifacts_dir().join(&asset_id);
        if !asset_path.exists() {
            std::fs::write(&asset_path, raw_content)?;
        }

        // 3. Build semantic index if embedder is attached.
        //    Clone the Arc out of the lock guard so the lock is dropped
        //    before the async `AssetIndex::build` call (clippy: await_holding_lock).
        let model = self
            .embedder
            .read()
            .expect("sandbox mutex poisoned")
            .clone();
        if let Some(model) = model {
            let index = AssetIndex::build(model.as_ref(), raw_content, 20).await?;
            let mut index_guard = self.asset_indices.write().expect("sandbox mutex poisoned");
            index_guard.insert(asset_id.clone(), index);
        }

        let line_count = raw_content.lines().count();
        let size_kb = raw_content.len() as f64 / 1024.0;
        let preview: Vec<&str> = raw_content.lines().take(3).collect();

        // 4. Compact handle — the LLM sees this instead of the raw bytes
        Ok(format!(
            "[Asset indexed | ID: {}]\n- Size: {:.2} KB ({} lines)\n- Preview:\n  {}\n  ...\nTip: call search_asset(asset_id: \"{}\", query: \"...\") to search.",
            asset_id,
            size_kb,
            line_count,
            preview.join("\n  "),
            asset_id
        ))
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
                anyhow::bail!(
                    "Path has too many non-existing ancestors: {}",
                    path.display()
                );
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
                let remaining = path
                    .strip_prefix(parent)
                    .expect("parent is a prefix by construction");
                return Ok(checked.join(remaining));
            }
            ancestor = parent.parent();
            depth += 1;
        }

        anyhow::bail!(
            "Path has no existing ancestor within sandbox: {}",
            path.display()
        )
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

    /// Atomic counter for unique sandbox IDs across parallel tests.
    static TEST_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn test_handle() -> SandboxHandle {
        let n = TEST_IDX.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut id = [0u8; 16];
        id[0..8].copy_from_slice(&n.to_le_bytes());
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
        // Use a non-existent path — src/main.rs exists in the project
        // root and would resolve through the read-through fallback.
        let err = h
            .resolve_path_read_only("nonexistent_bogus_file.bin")
            .unwrap_err();
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
