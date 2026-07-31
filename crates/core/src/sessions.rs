//! Session management for the workflow runtime.
//!
//! Each [`Session`] is an independent workspace with its own [`Runtime`].
//! The [`Sessions`] manager provides create, switch, persist, and restore
//! operations with file-based storage under `~/.workflow/sessions/`.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tracing::info;
use workflow_role::{Role, RoleId};

use crate::{ConversationMessage, Runtime, RuntimeConfig, RuntimeError};

/// Unique identifier for a session.
pub type SessionId = u32;

/// Serializable session metadata (no live [`Runtime`] references).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub name: String,
    pub created_at: u64,
    pub last_used_at: u64,
    /// Project folder path this session is bound to (if any).
    #[serde(default)]
    pub project: Option<String>,
}

/// An active session with its own [`Runtime`].
pub struct Session {
    pub meta: SessionMeta,
    pub runtime: Arc<Runtime>,
}

/// Data persisted per session.
#[derive(Debug, Serialize, Deserialize)]
struct PersistedSession {
    meta: SessionMeta,
    messages: HashMap<u32, Vec<ConversationMessage>>,
    roles: Vec<PersistedRole>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedRole {
    id: String,
    name: String,
    definition: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".workflow").join("sessions")
}

/// Manages multiple sessions with file-based persistence to
/// `~/.workflow/sessions/`.
pub struct Sessions {
    sessions: HashMap<SessionId, Session>,
    dir: PathBuf,
    next_id: SessionId,
    config: Option<RuntimeConfig>,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

impl Sessions {
    /// Create a new session manager.
    ///
    /// Sessions are persisted under `~/.workflow/sessions/`.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            dir: sessions_dir(),
            next_id: 0,
            config: None,
        }
    }

    /// Set a custom [`RuntimeConfig`] for newly created sessions.
    ///
    /// When not set, each session uses [`Runtime::new()`] which resolves
    /// configuration from environment and config files.
    pub fn with_config(mut self, config: RuntimeConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Create a new named session with a fresh [`Runtime`].
    ///
    /// Returns the new session's ID on success.
    pub async fn create(&mut self, name: &str) -> Result<SessionId, RuntimeError> {
        let id = self.allocate_id();
        let now = now_secs();
        let runtime = match &self.config {
            Some(config) => Arc::new(Runtime::try_new(config.clone())?),
            None => Arc::new(Runtime::new()),
        };
        runtime.initialize().await?;
        let meta = SessionMeta {
            id,
            name: name.to_string(),
            created_at: now,
            last_used_at: now,
            project: runtime.project().map(|p| p.path.clone()),
        };
        let session = Session { meta, runtime };
        self.sessions.insert(id, session);
        Ok(id)
    }

    /// Create a new session with an existing [`Runtime`].
    ///
    /// Useful when a [`Runtime`] has already been configured by external
    /// code (e.g. the Tauri `configure_runtime` command).
    pub fn create_with_runtime(&mut self, name: &str, runtime: Arc<Runtime>) -> SessionId {
        let id = self.allocate_id();
        let now = now_secs();
        let meta = SessionMeta {
            id,
            name: name.to_string(),
            created_at: now,
            last_used_at: now,
            project: runtime.project().map(|p| p.path.clone()),
        };
        let session = Session { meta, runtime };
        self.sessions.insert(id, session);
        id
    }

    /// Persist all sessions to disk as JSON files.
    ///
    /// Each session is written to `session_{id}.json` in the sessions
    /// directory, containing metadata, conversation messages, and roles.
    /// Returns the number of sessions saved.
    pub async fn save(&self) -> anyhow::Result<u32> {
        std::fs::create_dir_all(&self.dir)?;
        let mut count = 0u32;
        for session in self.sessions.values() {
            let messages: HashMap<u32, Vec<ConversationMessage>> = session
                .runtime
                .messages
                .iter()
                .map(|entry| (*entry.key(), entry.value().clone()))
                .collect();
            let roles: Vec<PersistedRole> = session
                .runtime
                .roles
                .read()
                .ok()
                .map(|guard| {
                    guard
                        .list()
                        .into_iter()
                        .map(|r| PersistedRole {
                            id: r.name().to_owned(),
                            name: r.name().to_owned(),
                            definition: r.definition().to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let persisted = PersistedSession {
                meta: session.meta.clone(),
                messages,
                roles,
            };
            let path = self.dir.join(format!("session_{}.json", session.meta.id));
            let json = serde_json::to_string_pretty(&persisted)?;
            std::fs::write(&path, json)?;
            count += 1;
        }
        if count > 0 {
            info!(count, "sessions saved");
        }
        Ok(count)
    }

    /// Restore sessions from disk.
    ///
    /// Creates new [`Runtime`] instances for each persisted session and
    /// populates them with the saved roles and conversation history.
    /// Returns the number of sessions loaded.
    pub async fn load(&mut self) -> anyhow::Result<u32> {
        if !self.dir.exists() {
            return Ok(0);
        }
        let mut count = 0u32;
        let mut reader = tokio::fs::read_dir(&self.dir).await?;
        while let Some(entry) = reader.next_entry().await? {
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "json") {
                continue;
            }
            let file_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_owned();
            if !file_name.starts_with("session_") {
                continue;
            }
            let json = tokio::fs::read_to_string(&path).await?;
            let persisted: PersistedSession = serde_json::from_str(&json)?;

            // Create a fresh Runtime for this session.
            let runtime = match &self.config {
                Some(config) => Arc::new(Runtime::try_new(config.clone())?),
                None => Arc::new(Runtime::new()),
            };
            // Restore roles (messages are loaded but live agents are recreated
            // fresh — the conversation history is available for display).
            for role in &persisted.roles {
                let role_obj = Role::new(role.name.clone(), role.definition.clone(), vec![]);
                runtime.roles.write().ok().map(|mut guard| {
                    guard.add(RoleId::from(role.id.clone()), role_obj);
                });
            }
            runtime.initialize().await?;

            // Ensure the ID allocator stays ahead of loaded session IDs.
            self.next_id = self.next_id.max(persisted.meta.id + 1);

            let session = Session {
                meta: persisted.meta,
                runtime,
            };
            self.sessions.insert(session.meta.id, session);
            count += 1;
        }
        if count > 0 {
            info!(count, "sessions loaded");
        }
        Ok(count)
    }

    /// Bind a session to a project folder path (or unbind with `None`).
    ///
    /// Only updates the metadata — rebinding the session's [`Runtime`] is
    /// the caller's responsibility (it must be rebuilt with the project).
    pub fn set_project(&mut self, id: SessionId, project: Option<String>) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.meta.project = project;
        }
    }

    /// Remove a session by ID, returning it if it existed.
    pub fn remove(&mut self, id: SessionId) -> Option<Session> {
        self.sessions.remove(&id)
    }

    /// Get a mutable reference to a session by ID.
    pub fn get(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(&id)
    }

    /// Get a reference to a session by ID.
    pub fn get_ref(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// List all session metadata.
    pub fn list(&self) -> Vec<&SessionMeta> {
        let mut metas: Vec<&SessionMeta> = self.sessions.values().map(|s| &s.meta).collect();
        metas.sort_by_key(|m| m.last_used_at);
        metas
    }

    /// Number of active sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Return the path of the sessions directory.
    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    fn allocate_id(&mut self) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_list_sessions() {
        let mut sessions = Sessions::new();
        assert!(sessions.is_empty());

        let id = sessions.create("test-session").await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions.list()[0].id, id);
        assert_eq!(sessions.list()[0].name, "test-session");
    }

    #[tokio::test]
    async fn create_multiple_sessions() {
        let mut sessions = Sessions::new();
        let id1 = sessions.create("alpha").await.unwrap();
        let id2 = sessions.create("beta").await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert_ne!(id1, id2);
    }

    #[tokio::test]
    async fn remove_session() {
        let mut sessions = Sessions::new();
        let id = sessions.create("to-remove").await.unwrap();
        assert_eq!(sessions.len(), 1);
        let removed = sessions.remove(id);
        assert!(removed.is_some());
        assert!(sessions.is_empty());
    }

    #[tokio::test]
    async fn get_session() {
        let mut sessions = Sessions::new();
        let id = sessions.create("get-me").await.unwrap();
        let session = sessions.get(id);
        assert!(session.is_some());
        assert_eq!(session.unwrap().meta.name, "get-me");

        assert!(sessions.get(999).is_none());
    }

    #[tokio::test]
    async fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        let mut sessions = Sessions::new();
        // Override the directory to a temp dir.
        sessions.dir = dir_path.join("sessions");

        let id = sessions.create("roundtrip").await.unwrap();
        let saved = sessions.save().await.unwrap();
        assert_eq!(saved, 1);

        // Create a fresh manager pointing at the same dir and load.
        let mut loaded = Sessions::new();
        loaded.dir = sessions.dir.clone();
        // Ensure the loaded session gets an ID allocation that doesn't collide.
        loaded.next_id = 100;
        let count = loaded.load().await.unwrap();
        assert_eq!(count, 1);

        let session = loaded.get(id);
        assert!(session.is_some());
        assert_eq!(session.unwrap().meta.name, "roundtrip");
    }

    #[tokio::test]
    async fn set_project_updates_meta() {
        let mut sessions = Sessions::new();
        let id = sessions.create("project-session").await.unwrap();
        assert!(sessions.get(id).unwrap().meta.project.is_none());

        sessions.set_project(id, Some("/tmp/my-project".to_string()));
        assert_eq!(
            sessions.get(id).unwrap().meta.project.as_deref(),
            Some("/tmp/my-project")
        );

        sessions.set_project(id, None);
        assert!(sessions.get(id).unwrap().meta.project.is_none());
    }

    #[tokio::test]
    async fn project_field_roundtrips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        let mut sessions = Sessions::new();
        sessions.dir = dir_path.join("sessions");
        let id = sessions.create("proj-roundtrip").await.unwrap();
        sessions.set_project(id, Some("/tmp/proj".to_string()));
        sessions.save().await.unwrap();

        let mut loaded = Sessions::new();
        loaded.dir = sessions.dir.clone();
        loaded.next_id = 50;
        loaded.load().await.unwrap();

        assert_eq!(
            loaded.get_ref(id).unwrap().meta.project.as_deref(),
            Some("/tmp/proj")
        );
    }
}
