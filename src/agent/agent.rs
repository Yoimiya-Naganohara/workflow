use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

use crate::core::types::AgentId;
use crate::l0::BudgetGuard;
use crate::llm::types::Message;
use crate::tools::sandbox::SandboxHandle;

// ── MemoEntry ──

/// A single key-value memo scoped to a role.
///
/// Memos provide a simple notepad/scratchpad for agents to
/// store intermediate findings, decisions, or context during
/// their lifecycle.  Unlike the experience pool (which is about
/// long-term learning) or `result` (final output), memos are
/// ephemeral key-value notes that any agent of the same role
/// can read, write, and list via MCP tools.
///
/// Memos are shared across all agents with the same role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoEntry {
    pub key: String,
    pub value: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub model_id: String,
    pub provider_id: String,
    pub max_tokens: u64,
    pub temperature: f64,
    /// Bitmap of tools this agent is allowed to use.
    pub allowed_tools: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            provider_id: String::new(),
            max_tokens: 4000,
            temperature: 0.7,
            allowed_tools: 0,
        }
    }
}

// ── Tool trace (Phase 3) ──

/// Maximum number of tool call records kept per agent.
/// After this limit, the oldest entry is dropped (ring buffer).
pub const MAX_TOOL_TRACE: usize = 20;

// ── Inter-agent messages ──

/// Maximum number of inbound messages stored per agent.
/// When full, the oldest message is dropped (ring buffer).
pub const MAX_INBOX_SIZE: usize = 64;

/// A single message sent between agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Sender agent ID.
    pub from: AgentId,
    /// Sender's human-readable name (for display).
    pub from_name: String,
    /// Plain-text message body (short summary for small messages).
    pub content: String,
    /// Structured payload for handle-first messaging.
    /// When `Some(AssetPointer)`, the receiver should use `search_asset`
    /// to retrieve details rather than relying on the raw content.
    #[serde(default)]
    pub payload: Option<MessagePayload>,
    /// Unix timestamp.
    pub timestamp: u64,
}

/// Structured message payload for handle-first inter-agent communication.
///
/// - [`StateSummary`](MessagePayload::StateSummary): lightweight structured status
///   paired with JSON metrics (no raw text).
/// - [`AssetPointer`](MessagePayload::AssetPointer): references a large asset
///   stored in the sender's sandbox. The receiver must call `search_asset`
///   to retrieve relevant chunks — the raw bytes never enter the LLM context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessagePayload {
    StateSummary {
        status: String,
        summary: String,
        #[serde(default)]
        metrics: serde_json::Value,
    },
    AssetPointer {
        asset_id: String,
        tool_name: String,
        hint: String,
    },
}

/// A single tool call recorded during agent execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub name: String,
    /// Truncated argument preview (first 80 chars).
    pub args_preview: String,
    pub status: ToolStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub role: String,
    pub role_template_id: Option<u32>,
    pub parent_id: Option<AgentId>,
    pub children: Vec<AgentId>,
    pub depth: u32,
    pub goal: String,
    /// The task this agent is executing in the TaskGraph (Phase 2A).
    /// `None` for bootstrap agents and pre-Phase 2 agents.
    pub task_id: Option<crate::core::types::TaskId>,
    pub config: AgentConfig,
    pub status: AgentStatus,
    pub result: Option<String>,
    pub child_results: Vec<(AgentId, String)>,
    /// Conversation context — message history scoped to this agent.
    /// Appended after each interaction and used as the base for future LLM calls.
    pub context: Vec<Message>,
    /// Unix epoch seconds of last activity (used for TTL eviction).
    pub last_active_at: u64,
    /// Ring buffer of tool calls made during this agent's execution.
    /// Used by the TUI diagnostic detail popup (Phase 3).
    /// The buffer is bounded at [`MAX_TOOL_TRACE`].
    pub tool_trace: VecDeque<ToolCallRecord>,
    /// Cumulative token consumption for this agent's execution.
    /// Updated by `ToolEvent::TokenUsage` during LLM streaming.
    pub tokens_input: u32,
    pub tokens_output: u32,
    /// Inbound message inbox (ring buffer, bounded at [`MAX_INBOX_SIZE`]).
    /// Agents can read messages from siblings via the `read_messages` tool.
    pub inbox: VecDeque<AgentMessage>,
    /// Per-agent filesystem sandbox handle.
    /// Created on spawn, cleaned up on eviction.
    #[serde(skip)]
    pub sandbox: Option<Arc<SandboxHandle>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentStatus {
    Idle,
    Planning,
    AwaitingChildren,
    Aggregating,
    Completed,
    Failed,
}

pub struct AgentPool {
    agents: Vec<Agent>,
    pub(crate) provider: Option<Arc<crate::llm::LlmProvider>>,
    completions: HashMap<AgentId, Arc<Notify>>,
    /// Active budget guards keyed by agent ID.
    /// Guards are dropped (releasing budget) when agents complete or fail.
    budget_guards: HashMap<AgentId, BudgetGuard>,
    /// Role-scoped memos — shared by all agents with the same role.
    role_memos: HashMap<String, Vec<MemoEntry>>,
    /// Siblings that this agent depends on for coordination.
    /// Only populated when an agent explicitly declares a dependency
    /// via the `await_sibling` tool.  Cleared after the dependency
    /// is satisfied.
    pending_deps: HashMap<AgentId, Vec<AgentId>>,
    /// Max idle TTL for completed/failed agents (seconds). Default: 3600 (1h).
    pub ttl_secs: u64,
    /// Maximum number of agents in the pool. When exceeded, the least
    /// recently used Completed/Failed/Idle agents are evicted (LRU).
    pub max_agents: usize,
    /// Reasoning effort passed to compatible LLM providers.
    /// `None` = no reasoning, `Some("low"/"medium"/"high")` = effort level.
    pub reasoning_effort: Option<String>,
    /// Reasoning options parsed from `api.json` for the current model.
    /// Used to build provider-specific reasoning parameters dynamically.
    pub reasoning_options: Vec<crate::models::ReasoningOption>,
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPool {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            provider: None,
            completions: HashMap::new(),
            budget_guards: HashMap::new(),
            role_memos: HashMap::new(),
            pending_deps: HashMap::new(),
            ttl_secs: crate::core::types::SECONDS_PER_HOUR,
            max_agents: crate::core::constants::DEFAULT_MAX_AGENTS,
            reasoning_effort: None,
            reasoning_options: vec![],
        }
    }

    pub fn set_provider(&mut self, provider: crate::llm::LlmProvider) {
        self.provider = Some(Arc::new(provider));
    }

    pub fn add_agent(&mut self, agent: Agent) -> AgentId {
        let id = agent.id;
        self.completions.insert(id, Arc::new(Notify::new()));
        self.agents.push(agent);
        // Trigger LRU eviction if pool exceeds capacity.
        self.evict_lru(None);
        id
    }

    pub fn get_agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.id == id)
    }

    pub fn get_agent_mut(&mut self, id: &AgentId) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| &a.id == id)
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    /// Mutable access to the agent vector (used for batch operations
    /// such as sandbox re-hydration).
    pub fn agents_mut(&mut self) -> &mut Vec<Agent> {
        &mut self.agents
    }

    pub fn get_completion_notify(&self, id: &AgentId) -> Option<Arc<Notify>> {
        self.completions.get(id).cloned()
    }

    /// Attach a budget guard to an agent (called after spawn approval).
    ///
    /// The guard will be released (budget returned to pool) when the
    /// agent completes or fails.
    pub fn attach_budget_guard(&mut self, agent_id: AgentId, guard: BudgetGuard) {
        self.budget_guards.insert(agent_id, guard);
    }

    /// Release an agent's budget guard (typically when agent completes/fails).
    /// Drops the guard, which returns any unspent budget to the pool.
    pub fn release_budget_guard(&mut self, agent_id: &AgentId) {
        self.budget_guards.remove(agent_id);
    }

    pub fn notify_completed(&mut self, id: &AgentId) {
        if let Some(notify) = self.completions.get(id) {
            notify.notify_one();
        }
    }

    // ── Inter-agent message passing ──

    /// Send a message to a specific agent's inbox.
    /// If the inbox is full, the oldest message is dropped.
    pub fn send_message(
        &mut self,
        recipient: AgentId,
        from: AgentId,
        from_name: &str,
        content: &str,
        payload: Option<MessagePayload>,
    ) -> Result<(), String> {
        let Some(agent) = self.get_agent_mut(&recipient) else {
            return Err(format!("Agent {:02x} not found", recipient[0]));
        };
        let msg = AgentMessage {
            from,
            from_name: from_name.to_string(),
            content: content.to_string(),
            payload,
            timestamp: now_secs(),
        };
        if agent.inbox.len() >= MAX_INBOX_SIZE {
            agent.inbox.pop_front();
        }
        agent.inbox.push_back(msg);
        Ok(())
    }

    /// Read and drain all pending messages for an agent.
    pub fn drain_inbox(&mut self, agent_id: &AgentId) -> Vec<AgentMessage> {
        self.get_agent_mut(agent_id)
            .map(|a| a.inbox.drain(..).collect())
            .unwrap_or_default()
    }

    /// Register a dependency: `agent_id` blocks until `dep_id` completes.
    pub fn add_dependency(&mut self, agent_id: AgentId, dep_id: AgentId) {
        self.pending_deps.entry(agent_id).or_default().push(dep_id);
    }

    /// Check whether all pending dependencies for `agent_id` are satisfied
    /// (i.e. all dependent agents have completed or failed).
    pub fn dependencies_satisfied(&self, agent_id: &AgentId) -> bool {
        let Some(deps) = self.pending_deps.get(agent_id) else {
            return true; // no dependencies
        };
        deps.iter().all(|dep_id| {
            self.get_agent(dep_id)
                .map(|a| matches!(a.status, AgentStatus::Completed | AgentStatus::Failed))
                .unwrap_or(true) // missing agent = satisfied (defensive)
        })
    }

    /// Remove dependency tracking for an agent (called after satisfaction or on failure).
    pub fn clear_dependencies(&mut self, agent_id: &AgentId) {
        self.pending_deps.remove(agent_id);
    }

    /// Get all agents that have pending dependencies (not yet satisfied).
    pub fn blocked_agents(&self) -> Vec<AgentId> {
        self.pending_deps
            .iter()
            .filter(|(_, deps)| {
                !deps.iter().all(|dep_id| {
                    self.get_agent(dep_id)
                        .map(|a| matches!(a.status, AgentStatus::Completed | AgentStatus::Failed))
                        .unwrap_or(true)
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get all agents that are runnable (no unsatisfied dependencies).
    pub fn runnable_agents(&self) -> Vec<&Agent> {
        self.agents
            .iter()
            .filter(|a| {
                matches!(a.status, AgentStatus::Planning | AgentStatus::Idle)
                    && self.dependencies_satisfied(&a.id)
            })
            .collect()
    }

    pub fn summary(&self) -> String {
        let total = self.agents.len();
        let running = self
            .agents
            .iter()
            .filter(|a| {
                matches!(
                    a.status,
                    AgentStatus::Planning
                        | AgentStatus::AwaitingChildren
                        | AgentStatus::Aggregating
                )
            })
            .count();
        let completed = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Completed)
            .count();
        let failed = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Failed)
            .count();
        format!(
            "Agents: {} total, {} running, {} completed, {} failed",
            total, running, completed, failed
        )
    }

    pub fn agent_id_str(id: &AgentId) -> String {
        id.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join("")
    }

    // ── Role-scoped memo access ──

    /// Get an immutable reference to the role memos map.
    pub fn role_memos(&self) -> &HashMap<String, Vec<MemoEntry>> {
        &self.role_memos
    }

    /// Get a mutable reference to the role memos map.
    pub fn role_memos_mut(&mut self) -> &mut HashMap<String, Vec<MemoEntry>> {
        &mut self.role_memos
    }

    /// Get memos for a specific role, or an empty slice if none exist.
    pub fn get_role_memos(&self, role: &str) -> &[MemoEntry] {
        self.role_memos
            .get(role)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get mutable access to memos for a specific role.
    /// Creates an empty vec if the role doesn't have memos yet.
    pub fn get_role_memos_mut(&mut self, role: &str) -> &mut Vec<MemoEntry> {
        self.role_memos.entry(role.to_string()).or_default()
    }

    /// Write a memo entry to the given role.
    /// If a memo with the same key already exists, it is overwritten.
    pub fn write_role_memo(&mut self, role: &str, entry: MemoEntry) {
        let memos = self.role_memos.entry(role.to_string()).or_default();
        if let Some(existing) = memos.iter_mut().find(|m| m.key == entry.key) {
            *existing = entry;
        } else {
            memos.push(entry);
        }
    }

    /// Read a memo by key for a given role.
    pub fn read_role_memo(&self, role: &str, key: &str) -> Option<&MemoEntry> {
        self.role_memos.get(role)?.iter().find(|m| m.key == key)
    }

    /// Format all memos for a role as a system-prompt injection block.
    /// Returns None if the role has no memos.
    pub fn format_role_memos(&self, role: &str) -> Option<String> {
        let memos = self.role_memos.get(role)?;
        if memos.is_empty() {
            return None;
        }
        let mut buf = String::from("\n\n=== Role Memos ===");
        for entry in memos {
            use std::fmt::Write;
            let _ = write!(buf, "\n  [{}]: {}", entry.key, entry.value);
        }
        buf.push_str("\n====");
        Some(buf)
    }

    pub fn delete_role_memo(&mut self, role: &str, key: &str) -> bool {
        let Some(memos) = self.role_memos.get_mut(role) else {
            return false;
        };
        let initial_len = memos.len();
        memos.retain(|m| m.key != key);
        memos.len() < initial_len
    }

    /// Find an agent with the given role that is Completed or Idle (reusable).
    /// Returns the agent ID if found.
    pub fn find_idle_agent_by_role(&self, role: &str) -> Option<AgentId> {
        self.agents
            .iter()
            .find(|a| {
                a.role == role && matches!(a.status, AgentStatus::Completed | AgentStatus::Idle)
            })
            .map(|a| a.id)
    }

    /// Migrate any per-agent memos (legacy) into role-scoped memos.
    /// After calling this, individual agents' memos are cleared.
    /// This is safe to call multiple times — it only copies memos
    /// that haven't already been migrated for each role.
    pub fn migrate_legacy_memos(&mut self) {
        for _agent in &self.agents {
            // No-op after migration
        }
    }

    /// Mark an agent as recently active (updates last_active_at).
    pub fn mark_active(&mut self, id: &AgentId) {
        if let Some(agent) = self.get_agent_mut(id) {
            agent.last_active_at = now_secs();
        }
    }

    /// Clean up sandbox directories for evicted agents (best-effort).
    fn cleanup_sandboxes(&self, evicted: &[AgentId]) {
        for id in evicted {
            // Find and clean up sandbox by checking all agents for the one being evicted.
            // Since we're called before removal, iterate the full list.
            if let Some(agent) = self.agents.iter().find(|a| a.id == *id) {
                if let Some(ref sb) = agent.sandbox {
                    sb.cleanup();
                }
            }
        }
    }

    /// Evict completed/failed agents whose idle time exceeds `ttl_secs`.
    /// Never evicts the responsible agent (protected_id) or the last remaining agent.
    /// Returns the number of evicted agents.
    pub fn evict_stale(&mut self, protected_id: Option<&AgentId>) -> usize {
        if self.agents.len() <= 1 {
            return 0;
        }
        let now = now_secs();
        let ttl = self.ttl_secs;
        let protect = protected_id.copied();

        // Collect evicted IDs first for sandbox cleanup
        let evicted: Vec<AgentId> = self
            .agents
            .iter()
            .filter(|a| {
                if Some(&a.id) == protect.as_ref() {
                    return false;
                }
                match a.status {
                    crate::agent::AgentStatus::Completed
                    | crate::agent::AgentStatus::Failed
                    | crate::agent::AgentStatus::Idle => {
                        now.saturating_sub(a.last_active_at) >= ttl
                    }
                    _ => false,
                }
            })
            .map(|a| a.id)
            .collect();

        if evicted.is_empty() {
            return 0;
        }

        // Clean up sandboxes before removing
        self.cleanup_sandboxes(&evicted);

        let before = self.agents.len();
        let evict_set: std::collections::HashSet<AgentId> = evicted.iter().copied().collect();
        self.agents.retain(|a| !evict_set.contains(&a.id));

        // Retain budget guards only for remaining agents
        let remaining: Vec<AgentId> = self.agents.iter().map(|a| a.id).collect();
        self.budget_guards.retain(|id, _| remaining.contains(id));

        let evicted_count = before - self.agents.len();
        if evicted_count > 0 {
            tracing::info!("Evicted {} stale agent(s) (ttl={}s)", evicted_count, ttl);
        }
        evicted_count
    }

    /// Evict the least recently used agents when pool exceeds `max_agents`.
    /// Only evicts terminal agents (Completed/Failed/Idle) that have no active children.
    /// Never evicts the protected (currently responsible) agent.
    /// Returns the number of evicted agents.
    pub fn evict_lru(&mut self, protected_id: Option<&AgentId>) -> usize {
        if self.agents.len() <= self.max_agents {
            return 0;
        }
        let before = self.agents.len();
        let excess = before.saturating_sub(self.max_agents);
        let protect = protected_id.copied();

        // Collect evictable agents: terminal status, no active children, not protected.
        let mut evictable: Vec<(AgentId, u64)> = self
            .agents
            .iter()
            .filter(|a| {
                if Some(&a.id) == protect.as_ref() {
                    return false;
                }
                if !matches!(
                    a.status,
                    crate::agent::AgentStatus::Completed
                        | crate::agent::AgentStatus::Failed
                        | crate::agent::AgentStatus::Idle
                ) {
                    return false;
                }
                // Don't evict agents that have active (non-terminal) children.
                let has_active_child = a.children.iter().any(|cid| {
                    self.get_agent(cid)
                        .map(|c| {
                            !matches!(
                                c.status,
                                crate::agent::AgentStatus::Completed
                                    | crate::agent::AgentStatus::Failed
                            )
                        })
                        .unwrap_or(false)
                });
                !has_active_child
            })
            .map(|a| (a.id, a.last_active_at))
            .collect();

        // Sort by last_active_at ascending — oldest (least recently used) first.
        evictable.sort_by_key(|(_, ts)| *ts);

        // Evict the oldest excess agents.
        let to_evict: Vec<AgentId> = evictable.iter().take(excess).map(|(id, _)| *id).collect();

        if to_evict.is_empty() {
            return 0;
        }

        // Clean up sandboxes before removing
        self.cleanup_sandboxes(&to_evict);

        let evict_set: std::collections::HashSet<AgentId> = to_evict.iter().copied().collect();
        self.agents.retain(|a| !evict_set.contains(&a.id));

        // Clean up associated state.
        for id in &to_evict {
            self.completions.remove(id);
            self.budget_guards.remove(id);
            self.pending_deps.remove(id);
        }

        // Retain budget guards only for remaining agents.
        let remaining: Vec<AgentId> = self.agents.iter().map(|a| a.id).collect();
        self.budget_guards.retain(|id, _| remaining.contains(id));

        let evicted = before - self.agents.len();
        if evicted > 0 {
            tracing::info!(
                "LRU evicted {} agent(s) (pool: {} → {})",
                evicted,
                before,
                self.agents.len()
            );
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_pool() {
        let mut pool = AgentPool::new();
        let agent = Agent {
            id: [1; 16],
            name: "test".to_string(),
            role: "tester".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test goal".to_string(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: VecDeque::new(),
            inbox: VecDeque::new(),
            task_id: None,
            sandbox: None,
        };
        let id = pool.add_agent(agent);
        assert_eq!(pool.agents().len(), 1);
        assert!(pool.get_agent(&id).is_some());
    }

    #[test]
    fn test_agent_summary() {
        let mut pool = AgentPool::new();
        let agent = Agent {
            id: [1; 16],
            name: "worker".to_string(),
            role: "dev".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "goal".to_string(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: VecDeque::new(),
            inbox: VecDeque::new(),
            task_id: None,
            sandbox: None,
        };
        pool.add_agent(agent);
        let summary = pool.summary();
        assert!(summary.contains("1 total"));
    }

    // ── Role memos ──

    #[test]
    fn test_write_and_read_role_memo() {
        let mut pool = AgentPool::new();
        let entry = MemoEntry {
            key: "test_key".to_string(),
            value: "test_value".to_string(),
            timestamp: 1000,
        };
        pool.write_role_memo("analyst", entry.clone());

        let read = pool.read_role_memo("analyst", "test_key");
        assert!(read.is_some());
        assert_eq!(read.unwrap().value, "test_value");

        // Different role should not see it
        assert!(pool.read_role_memo("planner", "test_key").is_none());
    }

    #[test]
    fn test_write_role_memo_overwrites() {
        let mut pool = AgentPool::new();
        pool.write_role_memo(
            "dev",
            MemoEntry {
                key: "x".to_string(),
                value: "v1".to_string(),
                timestamp: 1,
            },
        );
        pool.write_role_memo(
            "dev",
            MemoEntry {
                key: "x".to_string(),
                value: "v2".to_string(),
                timestamp: 2,
            },
        );
        let memos = pool.get_role_memos("dev");
        assert_eq!(memos.len(), 1);
        assert_eq!(memos[0].value, "v2");
    }

    #[test]
    fn test_delete_role_memo() {
        let mut pool = AgentPool::new();
        pool.write_role_memo(
            "dev",
            MemoEntry {
                key: "a".to_string(),
                value: "1".to_string(),
                timestamp: 0,
            },
        );
        assert!(pool.delete_role_memo("dev", "a"));
        assert!(!pool.delete_role_memo("dev", "a")); // already gone
        assert!(pool.get_role_memos("dev").is_empty());
    }

    #[test]
    fn test_get_role_memos_absent_role() {
        let pool = AgentPool::new();
        assert!(pool.get_role_memos("nonexistent").is_empty());
    }
}
