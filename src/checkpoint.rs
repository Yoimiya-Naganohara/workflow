//! Checkpoint — durable snapshot of agent runtime state.
//!
//! Saves and restores the three critical runtime components that would
//! otherwise be lost on process crash or restart:
//!
//! 1. **AgentPool** — all agents, their context, status, results
//! 2. **TaskGraph** — the DAG of pending/completed tasks
//!
//! # Design
//!
//! Checkpoints are written atomically to `~/.workflow/` using the same
//! `write_atomic()` approach as `persistence.rs`.  Each component is
//! stored in its own file so the TaskGraph can be saved independently
//! (it changes more frequently than the agent pool).
//!
//! # Recovery
//!
//! On restart, `Checkpoint::restore()` loads all components.  The caller
//! re-hydrates non-serializable fields (provider, Notify handles, budget
//! guards, sandbox handles) based on the deserialized state.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::agent::{Agent, AgentPool};
use crate::runtime::task_graph::TaskGraph;

// ── Constants ──

/// File name for the serialized agent pool.
const AGENT_POOL_FILE: &str = "agent_pool.bin";
/// File name for the serialized task graph.
const TASK_GRAPH_FILE: &str = "task_graph.bin";

// ── Checkpoint Snapshot ──

/// A full snapshot of runtime state that must survive crashes.
///
/// Only the serializable subset — runtime constructs like `Arc<Notify>`,
/// `BudgetGuard`, and `SandboxHandle` are restored separately by the
/// caller using the deserialized data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    /// All agents in the pool, including their context, status, results.
    pub agents: Vec<Agent>,
    /// Role-scoped memos shared by agents.
    pub role_memos: std::collections::HashMap<String, Vec<crate::agent::MemoEntry>>,
    /// The task dependency graph (DAG).
    pub task_graph: TaskGraph,
}

// ── Checkpoint manager ──

/// Manages durable snapshots of the agent runtime.
pub struct Checkpoint {
    pool_path: PathBuf,
    graph_path: PathBuf,
}

impl Checkpoint {
    /// Create a checkpoint manager rooted at `~/.workflow/`.
    pub fn new() -> Self {
        let base = Self::base_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            pool_path: base.join(AGENT_POOL_FILE),
            graph_path: base.join(TASK_GRAPH_FILE),
        }
    }

    /// Create a checkpoint manager with a custom directory (for testing).
    pub fn with_dir(dir: PathBuf) -> Self {
        Self {
            pool_path: dir.join(AGENT_POOL_FILE),
            graph_path: dir.join(TASK_GRAPH_FILE),
        }
    }

    /// Save the agent pool to disk.
    pub fn save_pool(&self, pool: &AgentPool) -> Result<()> {
        let bytes = bincode::serialize(pool)?;
        crate::persistence::write_binary(&self.pool_path, &bytes)?;
        Ok(())
    }

    /// Load the agent pool from disk.
    pub fn load_pool(&self) -> Result<Option<AgentPool>> {
        if !self.pool_path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.pool_path)?;
        let pool: AgentPool = bincode::deserialize(&bytes)?;
        Ok(Some(pool))
    }

    /// Save the task graph to disk.
    pub fn save_graph(&self, graph: &TaskGraph) -> Result<()> {
        let bytes = bincode::serialize(graph)?;
        crate::persistence::write_binary(&self.graph_path, &bytes)?;
        Ok(())
    }

    /// Load the task graph from disk.
    pub fn load_graph(&self) -> Result<Option<TaskGraph>> {
        if !self.graph_path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.graph_path)?;
        let graph: TaskGraph = bincode::deserialize(&bytes)?;
        Ok(Some(graph))
    }

    /// Save a full snapshot (agent pool + task graph) atomically.
    ///
    /// Holds serialization locks for the duration of both serialization AND
    /// file I/O.  Prefer two-phase save via `serialize_snapshot` + `write_snapshot`
    /// when the caller holds runtime locks that should not be held across I/O.
    pub fn save_snapshot(&self, pool: &AgentPool, graph: &TaskGraph) -> Result<()> {
        self.save_pool(pool)?;
        self.save_graph(graph)?;
        Ok(())
    }

    /// Phase 1: serialize pool and graph into bytes (fast, in-memory only).
    ///
    /// Call this while holding the runtime read lock.  After dropping all
    /// locks, pass the returned bytes to `write_snapshot`.
    pub fn serialize_snapshot(
        &self,
        pool: &AgentPool,
        graph: &TaskGraph,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let pool_bytes = bincode::serialize(pool)?;
        let graph_bytes = bincode::serialize(graph)?;
        Ok((pool_bytes, graph_bytes))
    }

    /// Phase 2: write pre-serialized bytes to disk.
    ///
    /// No locks held.  Does file I/O which may block.
    pub fn write_snapshot(&self, pool_bytes: &[u8], graph_bytes: &[u8]) -> Result<()> {
        crate::persistence::write_binary(&self.pool_path, pool_bytes)?;
        crate::persistence::write_binary(&self.graph_path, graph_bytes)?;
        Ok(())
    }

    /// Restore a full snapshot.  Returns `None` if no checkpoint exists.
    pub fn restore_snapshot(&self) -> Result<Option<RuntimeSnapshot>> {
        let pool = match self.load_pool()? {
            Some(p) => p,
            None => return Ok(None),
        };
        let task_graph = self.load_graph()?.unwrap_or_default();
        let role_memos = pool.role_memos.clone();

        Ok(Some(RuntimeSnapshot {
            agents: pool.agents().to_vec(),
            role_memos,
            task_graph,
        }))
    }

    /// Delete all checkpoint files.
    pub fn clear(&self) -> Result<()> {
        if self.pool_path.exists() {
            std::fs::remove_file(&self.pool_path)?;
        }
        if self.graph_path.exists() {
            std::fs::remove_file(&self.graph_path)?;
        }
        Ok(())
    }

    /// Check whether a checkpoint exists.
    pub fn exists(&self) -> bool {
        self.pool_path.exists()
    }

    /// Re-hydrate an AgentPool from a deserialized snapshot.
    ///
    /// This restores the non-serializable fields that were skipped:
    /// - `provider`: left as `None` — caller must set it via `set_provider`
    /// - `completions`: re-created as `Arc<Notify>` for each agent
    /// - `budget_guards`: left empty — budget is reset on restart
    /// - `sandbox`: left as `None` — sandboxes are re-created on activation
    pub fn rehydrate_pool(snapshot: &RuntimeSnapshot) -> AgentPool {
        let mut pool = AgentPool::new();
        for agent in &snapshot.agents {
            let mut restored = agent.clone();
            // Sandbox handles cannot survive serialization; re-created on demand.
            restored.sandbox = None;
            // Reset non-terminal agents to Idle so they can be re-activated.
            // Running/Dispatching agents have no in-flight LLM call after crash.
            if matches!(
                restored.status,
                crate::agent::AgentStatus::Planning
                    | crate::agent::AgentStatus::AwaitingChildren
                    | crate::agent::AgentStatus::Aggregating
            ) {
                restored.status = crate::agent::AgentStatus::Idle;
            }
            // Use add_agent which also creates the Notify handle.
            pool.add_agent(restored);
        }
        pool.role_memos = snapshot.role_memos.clone();
        pool
    }

    fn base_dir() -> Option<PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        let dir = PathBuf::from(home).join(".workflow");
        let _ = std::fs::create_dir_all(&dir);
        Some(dir)
    }
}

impl Default for Checkpoint {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
//  Tests
// ============================================================================

/// Restore an agent pool and task graph from the last checkpoint.
pub async fn restore_checkpoint(
    agent_pool: &tokio::sync::RwLock<crate::agent::AgentPool>,
    task_graph: &std::sync::Mutex<crate::runtime::task_graph::TaskGraph>,
) -> bool {
    use crate::checkpoint::Checkpoint;

    let cp = Checkpoint::new();
    let snapshot = match cp.restore_snapshot() {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::info!("No checkpoint found — starting fresh");
            return false;
        }
        Err(e) => {
            tracing::warn!("Failed to load checkpoint: {} — starting fresh", e);
            return false;
        }
    };

    tracing::info!(
        "Restored {} agents and {} tasks from checkpoint",
        snapshot.agents.len(),
        snapshot.task_graph.len(),
    );

    // Restore agent pool — preserve non-serialized config from existing pool.
    {
        let config = {
            let p = agent_pool.read().await;
            (
                p.max_retries,
                p.checkpoint_interval,
                p.ttl_secs,
                p.max_agents,
                p.reasoning_effort.clone(),
                p.reasoning_options.clone(),
            )
        };
        let mut rehydrated = Checkpoint::rehydrate_pool(&snapshot);
        rehydrated.max_retries = config.0;
        rehydrated.checkpoint_interval = config.1;
        rehydrated.ttl_secs = config.2;
        rehydrated.max_agents = config.3;
        rehydrated.reasoning_effort = config.4;
        rehydrated.reasoning_options = config.5;
        let mut p = agent_pool.write().await;
        *p = rehydrated;
    }

    // Restore task graph.
    {
        let mut g = task_graph
            .lock()
            .unwrap_or_else(|e: std::sync::PoisonError<_>| e.into_inner());
        *g = snapshot.task_graph;
        let reset_ids: Vec<crate::core::types::TaskId> = g
            .all_nodes()
            .filter(|n| {
                matches!(
                    n.status,
                    crate::runtime::task_graph::TaskStatus::Running
                        | crate::runtime::task_graph::TaskStatus::Dispatching
                )
            })
            .map(|n| n.id)
            .collect();
        for tid in reset_ids {
            let prev = g.get(&tid).map(|n| n.status);
            match prev {
                Some(crate::runtime::task_graph::TaskStatus::Running) => {
                    let _ = g.mark_ready(tid);
                }
                Some(crate::runtime::task_graph::TaskStatus::Dispatching) => {
                    let _ = g.mark_created(tid);
                }
                _ => {}
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentConfig, AgentStatus};
    use std::collections::VecDeque;

    fn stub_agent(id: u8) -> Agent {
        Agent {
            id: [id; 16],
            name: format!("agent-{}", id),
            role: "tester".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test".to_string(),
            config: AgentConfig::default(),
            status: AgentStatus::Planning,
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
            retry_count: 0,
            reasoning: String::new(),
        }
    }

    #[test]
    fn test_save_and_load_pool() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());

        let mut pool = AgentPool::new();
        pool.add_agent(stub_agent(1));
        pool.add_agent(stub_agent(2));

        cp.save_pool(&pool).unwrap();
        let loaded = cp.load_pool().unwrap().unwrap();
        assert_eq!(loaded.agents().len(), 2);
        assert_eq!(loaded.agents()[0].name, "agent-1");
        assert_eq!(loaded.agents()[1].name, "agent-2");
    }

    #[test]
    fn test_save_and_load_graph() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());

        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("main");
        let child = graph.spawn_child(root, "subtask").unwrap();
        graph.mark_ready(child).unwrap();

        cp.save_graph(&graph).unwrap();
        let loaded = cp.load_graph().unwrap().unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&root));
        assert_eq!(
            loaded.get(&child).unwrap().status,
            crate::runtime::task_graph::TaskStatus::Ready
        );
    }

    #[test]
    fn test_roundtrip_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());

        let mut pool = AgentPool::new();
        pool.add_agent(stub_agent(1));

        let mut graph = TaskGraph::new();
        graph.spawn_root("root");

        cp.save_snapshot(&pool, &graph).unwrap();
        let restored = cp.restore_snapshot().unwrap().unwrap();
        assert_eq!(restored.agents.len(), 1);
        assert_eq!(restored.task_graph.len(), 1);
    }

    #[test]
    fn test_rehydrate_pool() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());

        let mut pool = AgentPool::new();
        pool.add_agent(stub_agent(42));

        cp.save_pool(&pool).unwrap();
        let snapshot = cp.restore_snapshot().unwrap().unwrap();
        let rehydrated = Checkpoint::rehydrate_pool(&snapshot);

        // Sandbox should be None after rehydration
        assert_eq!(rehydrated.agents().len(), 1);
        assert!(rehydrated.agents()[0].sandbox.is_none());
        // Notify handles should be re-created
        assert!(rehydrated.get_completion_notify(&[42; 16]).is_some());
    }

    #[test]
    fn test_clear() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());

        let pool = AgentPool::new();
        let graph = TaskGraph::new();
        cp.save_snapshot(&pool, &graph).unwrap();
        assert!(cp.exists());

        cp.clear().unwrap();
        assert!(!cp.exists());
    }

    #[test]
    fn test_no_checkpoint_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let cp = Checkpoint::with_dir(dir.path().to_path_buf());
        assert!(cp.restore_snapshot().unwrap().is_none());
    }
}
