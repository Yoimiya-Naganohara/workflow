//! Agent memo tools — read/write/list/delete key-value memos.
//!
//! Agents use memos as a scratchpad/notepad to store intermediate
//! findings, decisions, or context during their lifecycle.  Memos
//! are per-role key-value pairs with timestamps, shared across all
//! agents with the same role.
//!
//! These tools are registered on the memos-enabled tool server so
//! agents can manage their own memos via MCP tool calls.

use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::agent::agent::{AgentPool, MemoEntry};
use crate::core::types::AgentId;

use super::builtin::ToolCallError;

/// Dependencies needed by the memo tools.
///
/// Wrapped in `Arc` so a single set of deps can be cloned into
/// every tool without inner locking.
pub struct MemoToolDeps {
    /// The shared agent pool.
    pub agent_pool: Arc<RwLock<AgentPool>>,
    /// Handle to the full AppState to read `responsible_agent_id`.
    state: Arc<RwLock<crate::tui::state::AppState>>,
}

impl MemoToolDeps {
    /// Extract memo tool dependencies from an AppState handle.
    ///
    /// If the lock is contended (e.g. a write lock is held elsewhere),
    /// this creates a stub with an empty agent pool.  The tools will
    /// return "Agent pool locked" errors at call time rather than
    /// panicking at startup.
    pub fn from_state(state: &Arc<RwLock<crate::tui::state::AppState>>) -> Self {
        let agent_pool = match state.try_read() {
            Ok(s) => s.core.agent_pool.clone(),
            Err(_) => {
                // Lock contended — create a fresh empty pool as fallback.
                // Tools will return errors until a real pool is available.
                tracing::warn!("MemoToolDeps: AppState lock contended, using fallback pool");
                Arc::new(RwLock::new(crate::agent::AgentPool::new()))
            }
        };
        Self {
            agent_pool,
            state: state.clone(),
        }
    }

    /// Read the current responsible agent ID from the AppState.
    fn responsible_agent_id(&self) -> Option<AgentId> {
        self.state
            .try_read()
            .ok()
            .and_then(|s| s.core.responsible_agent_id)
    }
}

/// Register all memo tools on a `ToolServer`.
pub fn register_memo_tools(
    server: crate::tools::ToolServer,
    deps: MemoToolDeps,
) -> crate::tools::ToolServer {
    let deps = Arc::new(deps);
    server
        .tool(WriteMemo { deps: deps.clone() })
        .tool(ReadMemo { deps: deps.clone() })
        .tool(ListMemos { deps: deps.clone() })
        .tool(DeleteMemo { deps })
}

// ============================================================================
//  Helpers
// ============================================================================

use crate::agent::now_secs;

/// Find the calling agent's ID, name, and role (immutable read).
fn find_agent_info(deps: &MemoToolDeps) -> Option<(AgentId, String, String)> {
    let agent_id = deps.responsible_agent_id()?;
    let pool = deps.agent_pool.try_read().ok()?;
    let agent = pool.get_agent(&agent_id)?;
    Some((agent_id, agent.name.clone(), agent.role.clone()))
}

/// Return a cloned copy of all memos for a given role.
fn get_role_memos_cloned(deps: &MemoToolDeps, role: &str) -> Option<Vec<MemoEntry>> {
    let pool = deps.agent_pool.try_read().ok()?;
    Some(pool.get_role_memos(role).to_vec())
}

// ============================================================================
//  WriteMemo
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct WriteMemoArgs {
    /// Memo key (namespaced identifier).
    pub key: String,
    /// Memo value (free-form text content).
    pub value: String,
}

/// Write a memo entry for the calling agent's role.
///
/// If a memo with the same key already exists, it is overwritten
/// with a new timestamp.  Memos persist for the agent's role
/// lifetime and are saved to disk on pool flush.
pub struct WriteMemo {
    deps: Arc<MemoToolDeps>,
}

impl Tool for WriteMemo {
    const NAME: &'static str = "write_memo";

    type Error = ToolCallError;
    type Args = WriteMemoArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Write a key-value memo for the current agent's role. Overwrites if key exists."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Memo key (namespaced identifier, e.g. 'task/findings', 'decision/approach')"
                    },
                    "value": {
                        "type": "string",
                        "description": "Memo value — free-form text content (up to ~8KB)"
                    }
                },
                "required": ["key", "value"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.key.is_empty() {
            return Err(ToolCallError("memo key cannot be empty".to_string()));
        }
        if args.value.len() > crate::core::types::MEMO_MAX_LENGTH {
            return Err(ToolCallError("memo value too large (max 8KB)".to_string()));
        }

        let agent_id = self
            .deps
            .responsible_agent_id()
            .ok_or_else(|| ToolCallError("No active agent".to_string()))?;

        let mut pool = self
            .deps
            .agent_pool
            .try_write()
            .map_err(|_| ToolCallError("Agent pool locked".to_string()))?;

        let agent = pool
            .get_agent(&agent_id)
            .ok_or_else(|| ToolCallError("Agent not found".to_string()))?;
        let role = agent.role.clone();
        // agent borrow dropped here — now we can call write_role_memo

        let entry = MemoEntry {
            key: args.key.clone(),
            value: args.value.clone(),
            timestamp: now_secs(),
        };

        pool.write_role_memo(&role, entry.clone());

        // Persist to disk
        let memos = pool.get_role_memos(&role).to_vec();
        let _ = crate::persistence::save_role_memos(&role, &memos);

        Ok(format!(
            "Memo written — key: '{}', {} bytes, role: '{}'",
            entry.key,
            entry.value.len(),
            role,
        ))
    }
}

// ============================================================================
//  ReadMemo
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ReadMemoArgs {
    /// Memo key to read.
    pub key: String,
}

/// Read a memo entry by key.
pub struct ReadMemo {
    deps: Arc<MemoToolDeps>,
}

impl Tool for ReadMemo {
    const NAME: &'static str = "read_memo";

    type Error = ToolCallError;
    type Args = ReadMemoArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Read a memo entry by key. Returns value and timestamp.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Memo key to read"
                    }
                },
                "required": ["key"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.key.is_empty() {
            return Err(ToolCallError("memo key cannot be empty".to_string()));
        }

        let (_, _, role) = find_agent_info(&self.deps)
            .ok_or_else(|| ToolCallError("No active agent".to_string()))?;

        let memos = get_role_memos_cloned(&self.deps, &role)
            .ok_or_else(|| ToolCallError("Agent pool locked".to_string()))?;

        match memos.iter().find(|m| m.key == args.key) {
            Some(entry) => {
                let age = now_secs().saturating_sub(entry.timestamp);
                let age_str = if age < 60 {
                    format!("{}s ago", age)
                } else if age < crate::core::types::SECONDS_PER_HOUR {
                    format!("{}m ago", age / 60)
                } else {
                    format!("{}h ago", age / crate::core::types::SECONDS_PER_HOUR)
                };
                Ok(format!(
                    "Memo '{}' ({}):\n---\n{}\n---\n(written {})",
                    entry.key,
                    entry.value.len(),
                    entry.value,
                    age_str,
                ))
            }
            None => Err(ToolCallError(format!("Memo key '{}' not found", args.key))),
        }
    }
}

// ============================================================================
//  ListMemos
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListMemosArgs {
    /// Optional prefix filter.
    pub prefix: Option<String>,
}

/// List all memo keys for the calling agent's role.
pub struct ListMemos {
    deps: Arc<MemoToolDeps>,
}

impl Tool for ListMemos {
    const NAME: &'static str = "list_memos";

    type Error = ToolCallError;
    type Args = ListMemosArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "List all memos for the current role. Optionally filter by prefix.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prefix": {
                        "type": "string",
                        "description": "Optional prefix filter (e.g. 'task/' lists only memos starting with 'task/')",
                        "optional": true
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let (_, _agent_name, role) = find_agent_info(&self.deps)
            .ok_or_else(|| ToolCallError("No active agent".to_string()))?;

        let memos = get_role_memos_cloned(&self.deps, &role)
            .ok_or_else(|| ToolCallError("Agent pool locked".to_string()))?;

        let now = now_secs();
        let mut entries: Vec<MemoEntry> = memos.into_iter().collect();

        if let Some(ref prefix) = args.prefix {
            entries.retain(|m| m.key.starts_with(prefix));
        }

        entries.sort_by(|a, b| a.key.cmp(&b.key));

        if entries.is_empty() {
            return Ok(format!("No memos for role '{}'", role));
        }

        let mut lines = format!("Memos for role '{}' ({} total):\n", role, entries.len());
        for entry in &entries {
            let age = now.saturating_sub(entry.timestamp);
            let age_str = if age < 60 {
                format!("{}s", age)
            } else if age < crate::core::types::SECONDS_PER_HOUR {
                format!("{}m", age / 60)
            } else {
                format!("{}h", age / crate::core::types::SECONDS_PER_HOUR)
            };
            let preview = if entry.value.len() > 60 {
                let end = entry
                    .value
                    .char_indices()
                    .nth(60)
                    .map(|(i, _)| i)
                    .unwrap_or(entry.value.len());
                format!("{}...", &entry.value[..end])
            } else {
                entry.value.clone()
            };
            lines.push_str(&format!(
                "  {}  {} bytes  {}  {:?}\n",
                entry.key,
                entry.value.len(),
                age_str,
                preview
            ));
        }
        Ok(lines)
    }
}

// ============================================================================
//  DeleteMemo
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DeleteMemoArgs {
    /// Memo key to delete.
    pub key: String,
}

/// Delete a memo entry by key.
pub struct DeleteMemo {
    deps: Arc<MemoToolDeps>,
}

impl Tool for DeleteMemo {
    const NAME: &'static str = "delete_memo";

    type Error = ToolCallError;
    type Args = DeleteMemoArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Delete a memo entry by key.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Memo key to delete"
                    }
                },
                "required": ["key"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.key.is_empty() {
            return Err(ToolCallError("memo key cannot be empty".to_string()));
        }

        let agent_id = self
            .deps
            .responsible_agent_id()
            .ok_or_else(|| ToolCallError("No active agent".to_string()))?;

        let mut pool = self
            .deps
            .agent_pool
            .try_write()
            .map_err(|_| ToolCallError("Agent pool locked".to_string()))?;

        let agent = pool
            .get_agent(&agent_id)
            .ok_or_else(|| ToolCallError("Agent not found".to_string()))?;
        let role = agent.role.clone();
        // agent borrow dropped here

        if pool.delete_role_memo(&role, &args.key) {
            // Persist to disk after deletion
            let memos = pool.get_role_memos(&role).to_vec();
            let _ = crate::persistence::save_role_memos(&role, &memos);
            Ok(format!("Memo '{}' deleted from role '{}'", args.key, role))
        } else {
            Err(ToolCallError(format!(
                "Memo key '{}' not found for role '{}'",
                args.key, role
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── now_secs helper ──

    #[test]
    fn test_now_secs_returns_recent_timestamp() {
        let now = now_secs();
        assert!(now > 1577836800, "timestamp should be after 2020");
        assert!(now < 4102444800, "timestamp should be before 2100");
    }

    // ── WriteMemo validation ──

    #[tokio::test]
    async fn test_write_memo_empty_key_rejected() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = WriteMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(WriteMemoArgs {
                key: "".to_string(),
                value: "test".to_string(),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_write_memo_value_too_large_rejected() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = WriteMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(WriteMemoArgs {
                key: "key".to_string(),
                value: "x".repeat(8193),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("8KB"));
    }

    #[tokio::test]
    async fn test_write_memo_value_at_max_size_accepted() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = WriteMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(WriteMemoArgs {
                key: "key".to_string(),
                value: "x".repeat(8192),
            })
            .await;
        // Should not fail with size error; fails with "No active agent"
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(!err.contains("8KB"), "should pass size validation");
    }

    // ── ReadMemo validation ──

    #[tokio::test]
    async fn test_read_memo_empty_key_rejected() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = ReadMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(ReadMemoArgs {
                key: "".to_string(),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_read_memo_no_active_agent() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = ReadMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(ReadMemoArgs {
                key: "some-key".to_string(),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No active agent"));
    }

    // ── DeleteMemo validation ──

    #[tokio::test]
    async fn test_delete_memo_empty_key_rejected() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        let tool = DeleteMemo {
            deps: Arc::new(deps),
        };
        let result = tool
            .call(DeleteMemoArgs {
                key: "".to_string(),
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    // ── MemoToolDeps ──

    #[test]
    fn test_memo_tool_deps_from_state_default() {
        let state = Arc::new(tokio::sync::RwLock::new(
            crate::tui::state::AppState::default(),
        ));
        let deps = MemoToolDeps::from_state(&state);
        assert!(deps.responsible_agent_id().is_none());
    }
}
