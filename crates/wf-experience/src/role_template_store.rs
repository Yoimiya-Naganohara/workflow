//! Persistent role template store backed by JSON.
//!
//! Role templates are the system prompts associated with each agent role
//! (planner, tester, developer, reviewer, …).  They are stored in a
//! human-editable JSON file at `~/.workflow/role_templates.json` and
//! are separate from the mmap-backed experience pool.
//!
//! Lookup order:
//! 1. Exact match by role name → fastest path
//! 2. Semantic similarity via embedding → discovers related roles
//! 3. Fallback → generic prompt

use std::fs;
use std::path::Path;
use std::sync::RwLock;

use anyhow::{Context, Result};

use wf_core::simd::cosine_similarity_384;
use wf_core::EMBEDDING_DIM;
use wf_core::RoleTemplate;

/// Thread-safe persistent store for [`RoleTemplate`]s.
pub struct RoleTemplateStore {
    templates: RwLock<Vec<RoleTemplate>>,
    path: std::path::PathBuf,
}

impl RoleTemplateStore {
    /// Open (or create) the store at `path`.
    ///
    /// If the file does not exist, an empty store is created in memory.
    /// Call [`seed_if_empty`](Self::seed_if_empty) to populate defaults.
    pub fn open(path: &Path) -> Result<Self> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create role template store directory")?;
        }

        let templates = if path.exists() {
            let data = fs::read_to_string(path).context("Failed to read role template store")?;
            if data.trim().is_empty() {
                Vec::new()
            } else {
                serde_json::from_str(&data).context("Failed to parse role template store")?
            }
        } else {
            Vec::new()
        };

        Ok(Self {
            templates: RwLock::new(templates),
            path: path.to_path_buf(),
        })
    }

    /// Seed default templates if the store is empty.
    ///
    /// This is safe to call multiple times — it only writes when the
    /// store is empty.
    pub fn seed_if_empty(&self, defaults: Vec<RoleTemplate>) {
        let mut guard = self
            .templates
            .write()
            .expect("role template store poisoned");
        if !guard.is_empty() {
            return;
        }
        *guard = defaults;
        // Persist immediately so the file exists for manual editing.
        self.persist_impl(&guard);
    }

    /// Look up a template by exact role name.
    pub fn get_by_role(&self, role: &str) -> Option<RoleTemplate> {
        let guard = self.templates.read().expect("role template store poisoned");
        guard.iter().find(|t| t.role == role).cloned()
    }

    /// Find the closest template by embedding similarity.
    ///
    /// Returns `Some(template)` if the best match exceeds `threshold`.
    /// Templates with `embedding = None` are skipped.
    pub fn find_closest(
        &self,
        query: &[f32; EMBEDDING_DIM],
        threshold: f32,
    ) -> Option<RoleTemplate> {
        let guard = self.templates.read().expect("role template store poisoned");
        let mut best: Option<(f32, RoleTemplate)> = None;

        for t in guard.iter() {
            let emb = match &t.embedding {
                Some(e) => e,
                None => continue,
            };
            let sim = cosine_similarity_384(query, emb);
            if sim > threshold {
                let is_better = best.as_ref().is_none_or(|(best_sim, _)| sim > *best_sim);
                if is_better {
                    best = Some((sim, t.clone()));
                }
            }
        }

        best.map(|(_, t)| t)
    }

    /// Upsert a template by `template_id`.
    ///
    /// Returns `true` if an existing template was updated, `false` if a new
    /// one was inserted.
    pub fn upsert(&self, template: RoleTemplate) -> bool {
        let mut guard = self
            .templates
            .write()
            .expect("role template store poisoned");
        let id = template.template_id;
        if let Some(pos) = guard.iter().position(|t| t.template_id == id) {
            guard[pos] = template;
            self.persist_impl(&guard);
            true
        } else {
            guard.push(template);
            self.persist_impl(&guard);
            false
        }
    }

    /// Return all stored templates.
    pub fn all(&self) -> Vec<RoleTemplate> {
        let guard = self.templates.read().expect("role template store poisoned");
        guard.clone()
    }

    /// Persist current templates to disk.
    pub fn persist(&self) -> Result<()> {
        let guard = self.templates.read().expect("role template store poisoned");
        self.persist_impl(&guard);
        Ok(())
    }

    /// Delete a template by its ID.
    /// Silently succeeds if the ID does not exist.
    pub fn delete_by_id(&self, template_id: u32) {
        let mut guard = self
            .templates
            .write()
            .expect("role template store poisoned");
        guard.retain(|t| t.template_id != template_id);
        self.persist_impl(&guard);
    }

    // ── helpers ──

    fn persist_impl(&self, guard: &[RoleTemplate]) {
        let data = serde_json::to_string_pretty(guard).expect("Failed to serialize role templates");
        let _ = fs::write(&self.path, &data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn dummy_templates() -> Vec<RoleTemplate> {
        vec![
            RoleTemplate {
                role: "planner".into(),
                label: "Senior Architect".into(),
                system_prompt: "You are a senior architect.".into(),
                template_id: 0,
                embedding: None,
                ..Default::default()
            },
            RoleTemplate {
                role: "tester".into(),
                label: "QA Engineer".into(),
                system_prompt: "You are a QA engineer.".into(),
                template_id: 1,
                embedding: None,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn test_open_empty_creates_no_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();
        assert!(store.all().is_empty());
        // File should not exist yet (only written on seed/persist).
        assert!(!path.exists());
    }

    #[test]
    fn test_seed_if_empty_writes_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();

        store.seed_if_empty(dummy_templates());
        assert_eq!(store.all().len(), 2);

        // File now exists.
        assert!(path.exists());

        // Second call does nothing.
        store.seed_if_empty(vec![RoleTemplate {
            role: "extra".into(),
            label: "Extra".into(),
            system_prompt: "extra".into(),
            template_id: 99,
            embedding: None,
            ..Default::default()
        }]);
        assert_eq!(store.all().len(), 2);
    }

    #[test]
    fn test_get_by_role_exact() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();
        store.seed_if_empty(dummy_templates());

        let t = store.get_by_role("planner");
        assert!(t.is_some());
        assert_eq!(t.unwrap().template_id, 0);

        assert!(store.get_by_role("nonexistent").is_none());
    }

    #[test]
    fn test_find_closest_skips_none_embeddings() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();
        store.seed_if_empty(dummy_templates());

        // All embeddings are None → find_closest returns None.
        let query = [0.5f32; EMBEDDING_DIM];
        assert!(store.find_closest(&query, 0.5).is_none());
    }

    #[test]
    fn test_find_closest_with_embeddings() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();

        let mut emb_a = [0.0f32; EMBEDDING_DIM];
        emb_a[0] = 1.0;
        let mut emb_b = [0.0f32; EMBEDDING_DIM];
        emb_b[0] = 0.5;

        store.seed_if_empty(vec![
            RoleTemplate {
                role: "alpha".into(),
                label: "Alpha".into(),
                system_prompt: "alpha prompt".into(),
                template_id: 10,
                embedding: Some(emb_a),
                ..Default::default()
            },
            RoleTemplate {
                role: "beta".into(),
                label: "Beta".into(),
                system_prompt: "beta prompt".into(),
                template_id: 11,
                embedding: Some(emb_b),
                ..Default::default()
            },
        ]);

        // Query close to alpha.
        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 0.95;
        let result = store.find_closest(&query, 0.8);
        assert!(result.is_some());
        assert_eq!(result.unwrap().role, "alpha");

        // Query below threshold — opposite direction from both embeddings.
        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = -1.0;
        let result = store.find_closest(&query, 0.8);
        assert!(result.is_none());
    }

    #[test]
    fn test_upsert_new_and_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");
        let store = RoleTemplateStore::open(&path).unwrap();

        let t = RoleTemplate {
            role: "alpha".into(),
            label: "Alpha".into(),
            system_prompt: "original".into(),
            template_id: 10,
            embedding: None,
            ..Default::default()
        };

        // Insert new.
        assert!(!store.upsert(t.clone()));
        assert_eq!(store.all().len(), 1);

        // Update existing.
        let updated = RoleTemplate {
            system_prompt: "updated".into(),
            ..t
        };
        assert!(store.upsert(updated.clone()));
        assert_eq!(store.all().len(), 1);
        assert_eq!(store.get_by_role("alpha").unwrap().system_prompt, "updated");
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("templates.json");

        // Create and seed.
        {
            let store = RoleTemplateStore::open(&path).unwrap();
            store.seed_if_empty(dummy_templates());
        }

        // Re-open and verify.
        {
            let store = RoleTemplateStore::open(&path).unwrap();
            let all = store.all();
            assert_eq!(all.len(), 2);
            assert_eq!(all[0].role, "planner");
            assert_eq!(all[1].role, "tester");
        }
    }
}
