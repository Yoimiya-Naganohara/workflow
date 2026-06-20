//! Task Graph — a DAG-based task system for agent delegation.
//!
//! This is Phase 1 of the delegation primitive upgrade.  Previously
//! tasks were a flat `Vec<Task>` in `PlanEntity` with no parent,
//! child, or dependency semantics.  This module replaces that with
//! a proper directed acyclic graph where:
//!
//! - `parent/children = decomposition hierarchy` (who spawned whom)
//! - `dependencies = execution ordering` (what must finish first)
//! - `assigned_agent = who owns the work`
//!
//! # Lock strategy
//!
//! Following the same lock-free philosophy as L0 (`src/l0.rs`),
//! `TaskGraph` wraps an inner `HashMap` behind a single `Mutex`.
//! This is intentional — graph mutations are short (pointer swaps
//! in the map), never held across `.await` points, and contention
//! is bounded by the number of concurrent agents (default 10).
//! For the current scale this is correct; a lock-free DAG would
//! be premature optimization.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::core::types::{AgentId, TaskId};

// ============================================================================
//  FailurePolicy — how to propagate failures in decomposed tasks
// ============================================================================

/// Strategy for propagating task failures upward in the DAG.
///
/// In Phase 1.1 only `FailFast` exists.  Future expansions:
/// - `WaitAll`: wait for all siblings, then aggregate results
/// - `ContinueOnFail`: ignore failed siblings, only care about successful ones
///
/// This enum lives close to the graph layer, not in the Delegation Engine,
/// because failure propagation is a **graph semantics invariant** — it must
/// be enforced at the data structure level, not the orchestration level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FailurePolicy {
    /// A child failure immediately marks the parent as `Failed` and
    /// propagates recursively upward.  Siblings continue running but
    /// their results are ultimately discarded.
    FailFast,
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self::FailFast
    }
}

// ============================================================================
//  TaskStatus — richer than the old flat enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Created but not yet ready to execute (dependencies pending).
    Created,
    /// All dependencies satisfied, waiting for an agent assignment.
    Ready,
    /// Assigned to an agent, currently running.
    Running,
    /// Sub-tasks have been spawned under this node (composite).
    /// This node becomes `Completed` only when all its children finish.
    Decomposed,
    /// Successfully completed.
    Completed,
    /// Failed — either execution error or a child failed.
    Failed,
    /// The pipeline is currently processing this task (anti-double-dispatch lock).
    /// While `Dispatching`, no other scheduler can pick it up.
    Dispatching,

    /// Rejected by the pipeline before execution ever started.
    /// Unlike `Failed`, this means the system declined to run it.
    /// Used for L1/L2 rejections, budget exhaustion, etc.
    Rejected,
    /// Explicitly blocked / escalated.
    Blocked,
    /// Skipped (e.g. a conditional task that was unnecessary).
    Skipped,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed
                | TaskStatus::Failed
                | TaskStatus::Rejected
                | TaskStatus::Skipped
                | TaskStatus::Blocked
        )
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self,
            TaskStatus::Ready | TaskStatus::Running | TaskStatus::Decomposed
        )
    }

    pub fn can_transition_to(&self, target: TaskStatus) -> bool {
        use TaskStatus::*;
        match (self, target) {
            (Created, Ready) => true,
            (Created, Decomposed) => true,
            (Created, Dispatching) => true, // scheduler takes the lock
            (Created, Rejected) => true,    // pipeline rejected before execution
            (Ready, Running) => true,
            (Ready, Dispatching) => true,
            (Ready, Decomposed) => true,
            (Ready, Completed) => true,
            (Ready, Failed) => true,   // scheduling failure / cancellation
            (Ready, Rejected) => true, // pipeline rejected at schedule time
            (Dispatching, Running) => true, // pipeline approved
            (Dispatching, Rejected) => true, // pipeline rejected/errored
            (Dispatching, Created) => true, // pipeline error → retryable
            (Running, Completed) => true,
            (Running, Failed) => true,
            (Decomposed, Completed) => true,
            (Decomposed, Failed) => true, // FailFast propagation
            (_, Blocked) => true,         // always allow block
            (_, Skipped) => true,         // always allow skip
            _ => false,
        }
    }
}

// ============================================================================
//  TaskNode — one node in the DAG
// ============================================================================

/// A single node in the task dependency graph.
///
/// # DAG invariants
///
/// - `parent` and `children` form the decomposition tree.
/// - `dependencies` are *sibling* tasks that must complete first.
/// - Decomposition and dependency are orthogonal: a `parent` can
///   have an explicit `dependency` on an unrelated task.
/// - Cycles are detected at insert time (see [`TaskGraph::add_dependency`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: TaskId,
    pub parent: Option<TaskId>,
    pub children: Vec<TaskId>,
    pub dependencies: Vec<TaskId>,
    pub status: TaskStatus,
    pub assigned_agent: Option<AgentId>,
    pub goal: String,
    pub role: Option<String>,
    pub result: Option<String>,
    /// IDs of child tasks that have completed (terminal).
    /// This is a lightweight set — actual results are read from each
    /// child node via `collect_child_results()`.  Avoids O(N) data
    /// duplication that `HashMap<TaskId, String>` would cause.
    pub completed_children: Vec<TaskId>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub metadata: HashMap<String, String>,
}

impl TaskNode {
    pub fn new(id: TaskId, goal: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            id,
            parent: None,
            children: Vec::new(),
            dependencies: Vec::new(),
            status: TaskStatus::Created,
            assigned_agent: None,
            goal: goal.to_string(),
            role: None,
            result: None,
            completed_children: Vec::new(),
            created_at: now,
            completed_at: None,
            metadata: HashMap::new(),
        }
    }

    /// Short human-readable label for display/TUI.
    pub fn label(&self) -> String {
        let id_short = self
            .id
            .iter()
            .take(4)
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join("");
        let goal_short = if self.goal.len() > 40 {
            format!("{}…", &self.goal[..40])
        } else {
            self.goal.clone()
        };
        format!("{} [{}] {}", id_short, self.status_label(), goal_short)
    }

    pub fn status_label(&self) -> &'static str {
        match self.status {
            TaskStatus::Created => "created",
            TaskStatus::Dispatching => "dispatching",
            TaskStatus::Rejected => "rejected",
            TaskStatus::Ready => "ready",
            TaskStatus::Running => "running",
            TaskStatus::Decomposed => "decomposed",
            TaskStatus::Completed => "done",
            TaskStatus::Failed => "failed",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Skipped => "skipped",
        }
    }

    pub fn agent_label(&self) -> String {
        self.assigned_agent
            .map(|id| id.iter().take(4).map(|b| format!("{:02x}", b)).collect())
            .unwrap_or_else(|| "-".to_string())
    }
}

// ============================================================================
//  TaskGraph — The DAG itself
// ============================================================================

/// A directed acyclic graph of tasks.
///
/// # Mutation model
///
/// All mutations are single-threaded behind a `Mutex`.  This is safe
/// because:
/// - Graph mutations are fast (HashMap operations)
/// - No `.await` point holds the lock
/// - Contention is bounded by `DEFAULT_MAX_AGENTS` (10)
///
/// Wrap in `Arc<Mutex<TaskGraph>>` for shared access across the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGraph {
    /// All nodes in the graph.  `pub(crate)` for testing — normal
    /// callers should use the public API methods.
    pub(crate) nodes: HashMap<TaskId, TaskNode>,
    roots: Vec<TaskId>,
    /// How failures propagate upward.  Default: `FailFast`.
    pub failure_policy: FailurePolicy,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            roots: Vec::new(),
            failure_policy: FailurePolicy::default(),
        }
    }

    // ── Query ──

    pub fn contains(&self, id: &TaskId) -> bool {
        self.nodes.contains_key(id)
    }

    pub fn get(&self, id: &TaskId) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: &TaskId) -> Option<&mut TaskNode> {
        self.nodes.get_mut(id)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn roots(&self) -> &[TaskId] {
        &self.roots
    }

    pub fn all_nodes(&self) -> impl Iterator<Item = &TaskNode> {
        self.nodes.values()
    }

    /// Collect all terminal (Completed, Failed, Skipped) nodes.
    pub fn terminal_nodes(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|n| n.status.is_terminal())
            .collect()
    }

    /// Collect all active (Ready, Running, Decomposed) nodes.
    pub fn active_nodes(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|n| n.status.is_active())
            .collect()
    }

    // ── Mutation ──

    /// Insert a new root node (no parent).
    pub fn spawn_root(&mut self, goal: &str) -> TaskId {
        let id: TaskId = rand::random();
        let node = TaskNode::new(id, goal);
        self.nodes.insert(id, node);
        self.roots.push(id);
        id
    }

    /// Insert a new node as a child of `parent_id`.
    /// The new node starts with the parent's parent relationship set,
    /// and is added to the parent's `children` list.
    pub fn spawn_child(&mut self, parent_id: TaskId, goal: &str) -> Option<TaskId> {
        let parent = self.nodes.get(&parent_id)?;
        let id: TaskId = rand::random();
        let mut node = TaskNode::new(id, goal);
        node.parent = Some(parent_id);
        let _ = parent; // release borrow before second get_mut

        if let Some(parent) = self.nodes.get_mut(&parent_id) {
            parent.children.push(id);
        }
        self.nodes.insert(id, node);
        Some(id)
    }

    /// Add a dependency edge: `task_id` depends on `depends_on_id`.
    /// Returns `Err` if adding would create a cycle or either ID is missing.
    ///
    /// # Cycle detection
    ///
    /// Uses DFS from `depends_on_id` to check whether it can reach `task_id`.
    /// This is O(n) in the worst case but n is small (task graph depth <= 5).
    pub fn add_dependency(&mut self, task_id: TaskId, depends_on_id: TaskId) -> Result<(), String> {
        if !self.nodes.contains_key(&task_id) {
            return Err(format!("Task {:02x} not found", task_id[0]));
        }
        if !self.nodes.contains_key(&depends_on_id) {
            return Err(format!(
                "Depends-on task {:02x} not found",
                depends_on_id[0]
            ));
        }
        if task_id == depends_on_id {
            return Err("Self-dependency is not allowed".to_string());
        }

        // Cycle detection: DFS from depends_on_id → check if it reaches task_id
        if self.can_reach(depends_on_id, task_id) {
            return Err("Adding this dependency would create a cycle".to_string());
        }

        if let Some(node) = self.nodes.get_mut(&task_id) {
            if !node.dependencies.contains(&depends_on_id) {
                node.dependencies.push(depends_on_id);
            }
        }
        Ok(())
    }

    /// Set the parent of an existing node.  Replaces any existing parent.
    /// Also updates the old parent's children list and the new parent's.
    ///
    /// # Cycle detection
    ///
    /// Checks whether `new_parent_id` is a descendant of `child_id` in the
    /// decomposition tree.  If so, setting parent would create a cycle:
    ///
    /// ```text
    /// A          set_parent(A, C)      A
    ///  └── B     ──────────────►        ┊
    ///       └── C                      B ── A  ← cycle!
    /// ```
    pub fn set_parent(&mut self, child_id: TaskId, new_parent_id: TaskId) -> Result<(), String> {
        if child_id == new_parent_id {
            return Err("Cannot set self as parent".to_string());
        }
        if !self.nodes.contains_key(&child_id) {
            return Err("Child task not found".to_string());
        }
        if !self.nodes.contains_key(&new_parent_id) {
            return Err("Parent task not found".to_string());
        }

        // Cycle detection: if new_parent is already a descendant of child,
        // setting it as parent would create a decomposition cycle.
        if self.is_descendant(child_id, new_parent_id) {
            return Err(format!(
                "Cannot set parent: {:02x}.. is a descendant of {:02x}.. — would create a decomposition cycle",
                new_parent_id[0], child_id[0]
            ));
        }

        // Remove from old parent's children list
        let old_parent = self.nodes.get(&child_id).and_then(|n| n.parent);
        if let Some(old_id) = old_parent {
            if let Some(old) = self.nodes.get_mut(&old_id) {
                old.children.retain(|c| *c != child_id);
            }
        }

        // Add to new parent's children list
        if let Some(new_parent) = self.nodes.get_mut(&new_parent_id) {
            if !new_parent.children.contains(&child_id) {
                new_parent.children.push(child_id);
            }
        }

        // Update child's parent pointer
        if let Some(child) = self.nodes.get_mut(&child_id) {
            child.parent = Some(new_parent_id);
        }

        // Update roots list
        if old_parent.is_some() {
            // No longer needs root tracking if it had a parent before
        } else {
            // Was a root, now has a parent — remove from roots
            self.roots.retain(|r| *r != child_id);
        }

        Ok(())
    }

    /// Mark a task as Completed.
    /// Also updates `completed_at` timestamp.
    /// Returns `Err` if the transition is invalid.
    pub fn mark_complete(&mut self, id: TaskId) -> Result<(), String> {
        // Phase 1: extract status + children under a brief mutable borrow.
        // Drop the borrow before any recursive calls to self.
        let (is_decomposed, children, parent_id): (bool, Vec<TaskId>, Option<TaskId>) = {
            let node = self
                .nodes
                .get_mut(&id)
                .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

            if !node.status.can_transition_to(TaskStatus::Completed) {
                return Err(format!(
                    "Cannot transition from {:?} to Completed",
                    node.status
                ));
            }

            let is_dec = node.status == TaskStatus::Decomposed;
            let child_ids = node.children.clone();
            let pid = node.parent;
            (is_dec, child_ids, pid)
        };

        // Phase 2: check decomposed status without holding the mutable borrow.
        if is_decomposed {
            let all_children_done = children.iter().all(|cid| {
                self.nodes
                    .get(cid)
                    .map_or(false, |c| c.status.is_terminal())
            });
            if !all_children_done {
                return Err("Cannot complete decomposed node: children still active".to_string());
            }
        }

        // Phase 3: update node status (re-acquire mutable borrow),
        // and extract result for parent propagation.
        let child_done_result: Option<(TaskId, String)> = {
            let node = self
                .nodes
                .get_mut(&id)
                .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            node.status = TaskStatus::Completed;
            node.completed_at = Some(now);
            // Extract result for parent's child_results.
            node.result.clone().map(|r| (id, r))
        };

        // Phase 3.5: record this child as completed in the parent.
        // Uses `completed_children` (a lightweight Vec<TaskId>)
        // instead of copying result strings.
        if child_done_result.is_some() {
            if let Some(pid) = parent_id {
                if let Some(parent) = self.nodes.get_mut(&pid) {
                    if !parent.completed_children.contains(&id) {
                        parent.completed_children.push(id);
                    }
                }
            }
        }

        // Phase 4: check if parent can now transition (no borrow on node).
        // Policy: AllCompleted — every child must be `Completed`.  If any
        // child is Failed/Rejected/Skipped, the parent does NOT auto-complete.
        // Custom aggregation policies belong in the DispatchDecider or
        // a higher orchestration layer, not in the graph.
        if let Some(pid) = parent_id {
            let should_complete_parent = {
                self.nodes.get(&pid).map_or(false, |parent| {
                    if parent.status == TaskStatus::Decomposed {
                        parent.children.iter().all(|cid| {
                            self.nodes
                                .get(cid)
                                .map_or(false, |c| c.status == TaskStatus::Completed)
                        })
                    } else {
                        false
                    }
                })
            };
            if should_complete_parent {
                let _ = self.mark_complete(pid);
            }
        }

        Ok(())
    }

    /// Mark a task as Decomposed (it has spawned sub-tasks).
    pub fn mark_decomposed(&mut self, id: TaskId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Decomposed) {
            return Err(format!(
                "Cannot transition from {:?} to Decomposed",
                node.status
            ));
        }
        node.status = TaskStatus::Decomposed;
        Ok(())
    }

    /// Mark a task as Dispatching — the pipeline is currently processing it.
    ///
    /// This is the anti-double-dispatch lock.  While `Dispatching`, the task
    /// is excluded from `ready_tasks()`.  After the pipeline finishes:
    /// - `mark_running()` if approved
    /// - `mark_rejected()` if rejected
    /// - `mark_created()` if retryable error
    pub fn mark_dispatching(&mut self, id: TaskId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Dispatching) {
            return Err(format!(
                "Cannot transition from {:?} to Dispatching",
                node.status
            ));
        }
        node.status = TaskStatus::Dispatching;
        Ok(())
    }

    /// Mark a task as Running.
    pub fn mark_running(&mut self, id: TaskId, agent_id: AgentId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Running) {
            return Err(format!(
                "Cannot transition from {:?} to Running",
                node.status
            ));
        }
        node.status = TaskStatus::Running;
        node.assigned_agent = Some(agent_id);
        Ok(())
    }

    /// Mark a task as Failed.
    ///
    /// # FailFast propagation
    ///
    /// If the task has a `Decomposed` parent, the failure propagates upward
    /// recursively (`FailFast` policy).  The parent is marked as `Failed`
    /// with a summary error, and the chain continues until either a root
    /// or a non-Decomposed ancestor is reached.
    ///
    /// # Agent release
    ///
    /// `assigned_agent` is cleared so the task can be retried later.
    pub fn mark_failed(&mut self, id: TaskId, error: &str) -> Result<(), String> {
        // Phase 1: extract parent info under a brief mutable borrow.
        let parent_id: Option<TaskId> = {
            let node = self
                .nodes
                .get_mut(&id)
                .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

            if !node.status.can_transition_to(TaskStatus::Failed) {
                return Err(format!(
                    "Cannot transition from {:?} to Failed",
                    node.status
                ));
            }
            node.status = TaskStatus::Failed;
            node.result = Some(error.to_string());
            node.assigned_agent = None;
            node.parent
        };

        // Phase 2: FailFast propagation — if parent is Decomposed,
        // propagate failure upward recursively.
        if let Some(pid) = parent_id {
            let parent_is_decomposed = self
                .nodes
                .get(&pid)
                .map_or(false, |p| p.status == TaskStatus::Decomposed);

            if parent_is_decomposed {
                // Collect leaf failure reasons for a summary message.
                let leaf_fail_ids: Vec<TaskId> = self.failed_leaves(Some(pid));
                let leaf_fails: Vec<String> = leaf_fail_ids
                    .iter()
                    .filter_map(|fid| self.nodes.get(fid))
                    .filter_map(|n| n.result.as_ref())
                    .cloned()
                    .collect();
                let summary = format!(
                    "FailFast: child {:02x}.. failed — propagating. Leaf failures: {}",
                    id[0],
                    if leaf_fails.is_empty() {
                        error.to_string()
                    } else {
                        leaf_fails.join("; ")
                    }
                );
                let _ = self.mark_failed(pid, &summary);
            }
        }

        Ok(())
    }

    /// Mark a task as Ready (all dependencies satisfied).
    pub fn mark_ready(&mut self, id: TaskId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Ready) {
            return Err(format!("Cannot transition from {:?} to Ready", node.status));
        }
        node.status = TaskStatus::Ready;
        Ok(())
    }

    /// Mark a task as Rejected by the pipeline.
    ///
    /// Unlike `mark_failed`, this means the task was never executed —
    /// the system declined to run it (e.g. L1/L2 rejection, budget
    /// exhaustion).  The task goes directly to a terminal state without
    /// having been `Running`.
    ///
    /// Clears `assigned_agent` since no agent was (or will be) assigned.
    pub fn mark_rejected(&mut self, id: TaskId, reason: &str) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Rejected) {
            return Err(format!(
                "Cannot transition from {:?} to Rejected",
                node.status
            ));
        }
        node.status = TaskStatus::Rejected;
        node.result = Some(reason.to_string());
        node.assigned_agent = None;
        Ok(())
    }

    /// Mark a task as Blocked.
    ///
    /// Clears `assigned_agent` so the task can be reassigned later.
    pub fn mark_blocked(&mut self, id: TaskId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Blocked) {
            return Err(format!(
                "Cannot transition from {:?} to Blocked",
                node.status
            ));
        }
        node.status = TaskStatus::Blocked;
        node.assigned_agent = None;
        Ok(())
    }

    // ── DAG scheduling API ──

    /// Return all tasks whose dependencies are satisfied and that are
    /// in `Created` status — they are ready to be assigned to agents.
    ///
    /// This is the core scheduling query: it returns the "frontier"
    /// of runnable work.
    ///
    /// # Dependency semantics
    ///
    /// Currently a dependency is satisfied when the target task reaches
    /// any terminal state (`Completed`, `Failed`, `Skipped`).  This means:
    ///
    /// ```text
    /// B depends_on A
    ///
    /// A Failed  →  B Ready  ✓  (B runs even if A failed)
    /// ```
    ///
    /// This is the "RequireCompletion" policy — the scheduler only waits
    /// for the upstream to finish, regardless of outcome.
    ///
    /// A future `DependencyPolicy` enum will add `RequireSuccess` which
    /// rejects when an upstream fails:
    ///
    /// ```rust
    /// #[derive(Default)]
    /// enum DependencyPolicy {
    ///     #[default]
    ///     RequireCompletion,  // current behavior
    ///     RequireSuccess,     // fail if dep failed
    /// }
    /// ```
    ///
    /// For now, all dependencies use `RequireCompletion`.  Add the enum
    /// once the Delegation Engine (Phase 2) needs the distinction.
    pub fn ready_tasks(&self) -> Vec<TaskId> {
        self.nodes
            .iter()
            .filter(|(_, node)| {
                if node.status != TaskStatus::Created {
                    return false;
                }
                // All dependencies must be terminal (Completed, Failed, Skipped)
                node.dependencies.iter().all(|dep_id| {
                    self.nodes
                        .get(dep_id)
                        .map_or(false, |dep| dep.status.is_terminal())
                })
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Return all tasks that are currently blocked by their dependencies.
    pub fn blocked_tasks(&self) -> Vec<(TaskId, Vec<TaskId>)> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.status == TaskStatus::Created && !node.dependencies.is_empty())
            .filter_map(|(id, node)| {
                let unsatisfied: Vec<TaskId> = node
                    .dependencies
                    .iter()
                    .filter(|&dep_id| {
                        self.nodes
                            .get(dep_id)
                            .map_or(true, |dep| !dep.status.is_terminal())
                    })
                    .copied()
                    .collect();
                if unsatisfied.is_empty() {
                    None
                } else {
                    Some((*id, unsatisfied))
                }
            })
            .collect()
    }

    /// Topological sort of all nodes (Kahn's algorithm).
    /// Returns `None` if the graph contains a cycle.
    pub fn topological_sort(&self) -> Option<Vec<TaskId>> {
        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        let mut adj: HashMap<TaskId, Vec<TaskId>> = HashMap::new();

        // Initialize in_degree for all nodes
        for id in self.nodes.keys() {
            in_degree.entry(*id).or_insert(0);
            adj.entry(*id).or_default();
        }

        // Build adjacency and in-degree
        for node in self.nodes.values() {
            for dep_id in &node.dependencies {
                adj.entry(*dep_id).or_default().push(node.id);
                *in_degree.entry(node.id).or_insert(0) += 1;
            }
        }

        // Kahn's algorithm
        let mut queue: VecDeque<TaskId> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut result = Vec::with_capacity(self.nodes.len());
        while let Some(id) = queue.pop_front() {
            result.push(id);
            if let Some(neighbors) = adj.get(&id) {
                for neighbor in neighbors {
                    if let Some(deg) = in_degree.get_mut(neighbor) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push_back(*neighbor);
                        }
                    }
                }
            }
        }

        if result.len() == self.nodes.len() {
            Some(result)
        } else {
            None // cycle detected
        }
    }

    /// Get the leaf nodes (no children) in the decomposition tree.
    /// These are the tasks that actually need agent execution.
    pub fn leaf_tasks(&self) -> Vec<TaskId> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.children.is_empty() && !node.status.is_terminal())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get the entire ancestor chain of a task (root-most first).
    pub fn ancestor_chain(&self, id: TaskId) -> Vec<TaskId> {
        let mut chain = Vec::new();
        let mut current = id;
        loop {
            if let Some(node) = self.nodes.get(&current) {
                chain.push(current);
                if let Some(parent) = node.parent {
                    current = parent;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        chain.reverse();
        chain
    }

    // ── Subgraph queries ──

    /// Get the entire subgraph rooted at `id` (the node + all descendants).
    pub fn subgraph(&self, id: TaskId) -> Vec<TaskId> {
        let mut result = Vec::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if !result.contains(&current) {
                result.push(current);
                if let Some(node) = self.nodes.get(&current) {
                    stack.extend(&node.children);
                }
            }
        }
        result
    }

    /// Check whether the subgraph rooted at `id` is fully terminal.
    pub fn subgraph_complete(&self, id: TaskId) -> bool {
        self.subgraph(id).iter().all(|sub_id| {
            self.nodes
                .get(sub_id)
                .map_or(false, |n| n.status.is_terminal())
        })
    }

    /// Count tasks by status within a subgraph.
    pub fn status_counts(&self, id: Option<TaskId>) -> HashMap<TaskStatus, usize> {
        let ids = match id {
            Some(root) => self.subgraph(root),
            None => self.nodes.keys().copied().collect(),
        };
        let mut counts = HashMap::new();
        for tid in ids {
            if let Some(node) = self.nodes.get(&tid) {
                *counts.entry(node.status).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Get all failed leaf tasks in the decomposition tree.
    /// Used by `mark_complete` to decide whether a parent can still succeed.
    /// Collect the results of all completed children of `id`.
    ///
    /// Unlike a stored `HashMap`, this dynamically reads from each child
    /// node's `result`, avoiding data duplication.  Returns `(child_id, result)`
    /// pairs in insertion order.
    ///
    /// # Complejidad
    ///
    /// O(n) where n = `completed_children.len()`.  For n ≪ 1000 this is fine
    /// without caching.  When n grows, add a snapshot field to `TaskNode`.
    pub fn collect_child_results(&self, id: TaskId) -> Vec<(TaskId, String)> {
        let Some(node) = self.nodes.get(&id) else {
            return Vec::new();
        };
        node.completed_children
            .iter()
            .filter_map(|cid| {
                let child = self.nodes.get(cid)?;
                child
                    .result
                    .clone()
                    .map(|r| (*cid, format!("[{}] {}", child.status_label(), r)))
            })
            .collect()
    }

    /// Get all failed leaf tasks in the decomposition tree.
    /// Used by `mark_complete` to decide whether a parent can still succeed.
    pub fn failed_leaves(&self, id: Option<TaskId>) -> Vec<TaskId> {
        let ids = match id {
            Some(root) => self.subgraph(root),
            None => self.nodes.keys().copied().collect(),
        };
        ids.into_iter()
            .filter(|tid| {
                self.nodes.get(tid).map_or(false, |n| {
                    n.status == TaskStatus::Failed && n.children.is_empty()
                })
            })
            .collect()
    }

    /// Mark a task back to `Created` so the scheduler can retry.
    /// Only valid from `Dispatching` (pipeline errored, not rejected).
    pub fn mark_created(&mut self, id: TaskId) -> Result<(), String> {
        let node = self
            .nodes
            .get_mut(&id)
            .ok_or_else(|| format!("Task {:02x} not found", id[0]))?;

        if !node.status.can_transition_to(TaskStatus::Created) {
            return Err(format!(
                "Cannot transition from {:?} to Created",
                node.status
            ));
        }
        node.status = TaskStatus::Created;
        node.assigned_agent = None;
        Ok(())
    }

    // ── Internal helpers ──

    /// DFS along **dependency edges** to see if `from` can reach `target`.
    fn can_reach(&self, from: TaskId, target: TaskId) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![from];
        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(node) = self.nodes.get(&current) {
                stack.extend(&node.dependencies);
            }
        }
        false
    }

    /// DFS along **parent edges** (upward) to see if `from` can reach `target`.
    /// Used by `set_parent` to detect decomposition cycles.
    ///
    /// Currently unused — `is_descendant` (downward traversal through
    /// children) handles the cycle check.  Kept for Phase 2 when parent
    /// lineage verification is needed from the runtime layer.
    #[allow(dead_code)]
    fn can_reach_via_parent(&self, from: TaskId, target: TaskId) -> bool {
        let mut visited = HashSet::new();
        let mut stack = vec![from];
        while let Some(current) = stack.pop() {
            if current == target {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(node) = self.nodes.get(&current) {
                // Traverse upward through parent pointers
                if let Some(pid) = node.parent {
                    stack.push(pid);
                }
            }
        }
        false
    }

    /// Check whether `candidate` is a descendant of `id` in the decomposition tree.
    fn is_descendant(&self, id: TaskId, candidate: TaskId) -> bool {
        // Reuse can_reach_via_parent but reversed: DFS downward through children
        let mut visited = HashSet::new();
        let mut stack = vec![id];
        while let Some(current) = stack.pop() {
            if current == candidate {
                return true;
            }
            if !visited.insert(current) {
                continue;
            }
            if let Some(node) = self.nodes.get(&current) {
                stack.extend(&node.children);
            } else {
                return false;
            }
        }
        false
    }
}

impl Default for TaskGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
//  Summary / display
// ============================================================================

impl std::fmt::Display for TaskGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let counts = self.status_counts(None);
        let total: usize = counts.values().sum();
        write!(
            f,
            "TaskGraph: {} total (created:{}, ready:{}, running:{}, decomposed:{}, done:{}, failed:{}, rejected:{}, blocked:{}, skipped:{})",
            total,
            counts.get(&TaskStatus::Created).unwrap_or(&0),
            counts.get(&TaskStatus::Ready).unwrap_or(&0),
            counts.get(&TaskStatus::Running).unwrap_or(&0),
            counts.get(&TaskStatus::Decomposed).unwrap_or(&0),
            counts.get(&TaskStatus::Completed).unwrap_or(&0),
            counts.get(&TaskStatus::Failed).unwrap_or(&0),
            counts.get(&TaskStatus::Rejected).unwrap_or(&0),
            counts.get(&TaskStatus::Blocked).unwrap_or(&0),
            counts.get(&TaskStatus::Skipped).unwrap_or(&0),
        )
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_root() {
        let mut graph = TaskGraph::new();
        let id = graph.spawn_root("main goal");
        assert!(graph.contains(&id));
        assert_eq!(graph.roots().len(), 1);
        assert_eq!(graph.get(&id).unwrap().goal, "main goal");
    }

    #[test]
    fn test_spawn_child() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let child = graph.spawn_child(root, "child").unwrap();
        assert!(graph.contains(&child));
        assert_eq!(graph.get(&child).unwrap().parent, Some(root));
        assert_eq!(graph.get(&root).unwrap().children, vec![child]);
    }

    #[test]
    fn test_add_dependency() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("task A");
        let b = graph.spawn_root("task B");

        assert!(graph.add_dependency(b, a).is_ok());
        assert!(graph.get(&b).unwrap().dependencies.contains(&a));
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");
        let c = graph.spawn_root("C");

        graph.add_dependency(b, a).unwrap();
        graph.add_dependency(c, b).unwrap();

        // Adding A → C would create A → C → B → A cycle
        assert!(graph.add_dependency(a, c).is_err());
    }

    #[test]
    fn test_self_dependency() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        assert!(graph.add_dependency(a, a).is_err());
    }

    #[test]
    fn test_ready_tasks() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");

        // B depends on A → B is not ready initially
        graph.add_dependency(b, a).unwrap();
        assert!(graph.ready_tasks().contains(&a));
        assert!(!graph.ready_tasks().contains(&b));

        // Complete A → B becomes ready
        graph.mark_ready(a).unwrap();
        graph.mark_complete(a).unwrap();
        assert!(graph.ready_tasks().contains(&b));
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");
        let c = graph.spawn_root("C");

        graph.add_dependency(c, a).unwrap();
        graph.add_dependency(c, b).unwrap();

        let sorted = graph.topological_sort().unwrap();
        let pos_a = sorted.iter().position(|&id| id == a).unwrap();
        let pos_b = sorted.iter().position(|&id| id == b).unwrap();
        let pos_c = sorted.iter().position(|&id| id == c).unwrap();

        // C must come after both A and B
        assert!(pos_c > pos_a);
        assert!(pos_c > pos_b);
    }

    #[test]
    fn test_leaf_tasks() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let _child = graph.spawn_child(root, "leaf1").unwrap();
        let _leaf2 = graph.spawn_child(root, "leaf2").unwrap();

        let leaves = graph.leaf_tasks();
        assert_eq!(leaves.len(), 2);
        assert!(!leaves.contains(&root));
    }

    #[test]
    fn test_set_parent() {
        let mut graph = TaskGraph::new();
        let root1 = graph.spawn_root("root1");
        let root2 = graph.spawn_root("root2");
        let child = graph.spawn_child(root1, "child").unwrap();

        // Move child from root1 to root2
        graph.set_parent(child, root2).unwrap();
        assert_eq!(graph.get(&child).unwrap().parent, Some(root2));
        assert!(!graph.get(&root1).unwrap().children.contains(&child));
        assert!(graph.get(&root2).unwrap().children.contains(&child));
    }

    #[test]
    fn test_subgraph() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let child = graph.spawn_child(root, "child").unwrap();
        let grandchild = graph.spawn_child(child, "grandchild").unwrap();

        let sub = graph.subgraph(root);
        assert_eq!(sub.len(), 3);
        assert!(sub.contains(&grandchild));
    }

    #[test]
    fn test_ancestor_chain() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let child = graph.spawn_child(root, "child").unwrap();
        let grandchild = graph.spawn_child(child, "grandchild").unwrap();

        let chain = graph.ancestor_chain(grandchild);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], root);
        assert_eq!(chain[1], child);
        assert_eq!(chain[2], grandchild);
    }

    #[test]
    fn test_mark_decomposed() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        graph.mark_decomposed(root).unwrap();
        assert_eq!(graph.get(&root).unwrap().status, TaskStatus::Decomposed);
    }

    #[test]
    fn test_decomposed_auto_complete() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        graph.mark_decomposed(root).unwrap();

        let c1 = graph.spawn_child(root, "c1").unwrap();
        let c2 = graph.spawn_child(root, "c2").unwrap();

        // Complete both children → root auto-completes
        graph.mark_ready(c1).unwrap();
        graph.mark_complete(c1).unwrap();
        graph.mark_ready(c2).unwrap();
        graph.mark_complete(c2).unwrap();

        assert_eq!(graph.get(&root).unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn test_status_counts() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let c1 = graph.spawn_child(root, "c1").unwrap();
        let _c2 = graph.spawn_child(root, "c2").unwrap();

        graph.mark_ready(c1).unwrap();
        graph.mark_complete(c1).unwrap();

        let counts = graph.status_counts(None);
        assert_eq!(*counts.get(&TaskStatus::Completed).unwrap_or(&0), 1);
        assert_eq!(*counts.get(&TaskStatus::Created).unwrap_or(&0), 2);
    }

    #[test]
    fn test_invalid_transition() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        // Can't mark a Created task as Completed directly (must go through Ready)
        assert!(graph.mark_complete(root).is_err());
    }

    #[test]
    fn test_ready_transition() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        // Created → Ready → Completed is valid
        assert!(graph.mark_ready(root).is_ok());
        assert!(graph.mark_complete(root).is_ok());
        assert_eq!(graph.get(&root).unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn test_blocked_tasks() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");
        graph.add_dependency(b, a).unwrap();

        let blocked = graph.blocked_tasks();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].0, b);

        // Complete A → no blocked tasks
        graph.mark_ready(a).unwrap();
        graph.mark_complete(a).unwrap();
        assert!(graph.blocked_tasks().is_empty());
    }

    #[test]
    fn test_subgraph_complete() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let c1 = graph.spawn_child(root, "c1").unwrap();
        let c2 = graph.spawn_child(root, "c2").unwrap();
        graph.mark_decomposed(root).unwrap();

        assert!(!graph.subgraph_complete(root));

        graph.mark_ready(c1).unwrap();
        graph.mark_complete(c1).unwrap();
        graph.mark_ready(c2).unwrap();
        graph.mark_complete(c2).unwrap();

        assert!(graph.subgraph_complete(root));
    }

    #[test]
    fn test_cycle_in_add_dependency() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");

        graph.add_dependency(b, a).unwrap();
        // Adding A → B would create A → B → A
        assert!(graph.add_dependency(a, b).is_err());
    }

    #[test]
    fn test_topological_sort_cycle() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");
        // Create a cycle by directly injecting dependencies (bypassing
        // add_dependency's cycle detection).  This simulates a corrupted
        // graph that topological_sort must still handle gracefully.
        {
            let node_a = graph.nodes.get_mut(&a).unwrap();
            node_a.dependencies.push(b);
        }
        {
            let node_b = graph.nodes.get_mut(&b).unwrap();
            node_b.dependencies.push(a);
        }
        assert!(graph.topological_sort().is_none());
    }

    // ── Phase 1.1: Parent cycle detection ──

    #[test]
    fn test_set_parent_rejects_cycle() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_child(a, "B").unwrap();
        let c = graph.spawn_child(b, "C").unwrap();

        // A ─► B ─► C
        //
        // set_parent(A, C) should fail because C is already a descendant of A
        assert!(graph.set_parent(a, c).is_err());
    }

    #[test]
    fn test_set_parent_rejects_self() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        assert!(graph.set_parent(a, a).is_err());
    }

    #[test]
    fn test_set_parent_works_without_cycle() {
        let mut graph = TaskGraph::new();
        let a = graph.spawn_root("A");
        let b = graph.spawn_root("B");
        let c = graph.spawn_root("C");

        // Move C under B (no cycle since C is a root)
        assert!(graph.set_parent(c, b).is_ok());
        assert_eq!(graph.get(&c).unwrap().parent, Some(b));
    }

    // ── Phase 1.1: FailFast propagation ──

    #[test]
    fn test_failfast_propagates_upward() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        graph.mark_decomposed(root).unwrap();

        let c1 = graph.spawn_child(root, "c1").unwrap();
        let c2 = graph.spawn_child(root, "c2").unwrap();

        // Fail c1 — FailFast should propagate to root
        graph.mark_ready(c1).unwrap();
        graph.mark_failed(c1, "c1 crashed").unwrap();

        // Root should be marked Failed
        assert_eq!(graph.get(&root).unwrap().status, TaskStatus::Failed);
        // c2 should still be Created (not affected by propagation)
        assert_eq!(graph.get(&c2).unwrap().status, TaskStatus::Created);
    }

    #[test]
    fn test_failfast_releases_agent() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let agent_id: AgentId = [0xAA; 16];

        graph.mark_ready(root).unwrap();
        graph.mark_running(root, agent_id).unwrap();
        assert_eq!(graph.get(&root).unwrap().assigned_agent, Some(agent_id));

        graph.mark_failed(root, "error").unwrap();
        // assigned_agent should be cleared
        assert_eq!(graph.get(&root).unwrap().assigned_agent, None);
    }

    #[test]
    fn test_blocked_releases_agent() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        let agent_id: AgentId = [0xBB; 16];

        graph.mark_ready(root).unwrap();
        graph.mark_running(root, agent_id).unwrap();
        assert_eq!(graph.get(&root).unwrap().assigned_agent, Some(agent_id));

        graph.mark_blocked(root).unwrap();
        // assigned_agent should be cleared
        assert_eq!(graph.get(&root).unwrap().assigned_agent, None);
    }

    #[test]
    fn test_failfast_multi_level() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        graph.mark_decomposed(root).unwrap();

        let mid = graph.spawn_child(root, "mid").unwrap();
        graph.mark_decomposed(mid).unwrap();

        let leaf = graph.spawn_child(mid, "leaf").unwrap();

        // Fail leaf → should propagate mid → root
        graph.mark_ready(leaf).unwrap();
        graph.mark_failed(leaf, "deep error").unwrap();

        assert_eq!(graph.get(&leaf).unwrap().status, TaskStatus::Failed);
        assert_eq!(graph.get(&mid).unwrap().status, TaskStatus::Failed);
        assert_eq!(graph.get(&root).unwrap().status, TaskStatus::Failed);
    }

    // ── Phase 1.1: Failed leaves query ──

    #[test]
    fn test_failed_leaves() {
        let mut graph = TaskGraph::new();
        let root = graph.spawn_root("root");
        graph.mark_decomposed(root).unwrap();

        let c1 = graph.spawn_child(root, "c1").unwrap();
        let c2 = graph.spawn_child(root, "c2").unwrap();
        let c3 = graph.spawn_child(root, "c3").unwrap();

        graph.mark_ready(c1).unwrap();
        graph.mark_ready(c2).unwrap();
        graph.mark_complete(c1).unwrap();
        graph.mark_failed(c2, "c2 error").unwrap();

        let failed = graph.failed_leaves(None);
        // Only c2 should be in failed leaves (c3 is Created, not terminal)
        assert!(failed.contains(&c2));
        assert!(!failed.contains(&c1));
        assert!(!failed.contains(&c3));
        assert!(!failed.contains(&root));
    }
}
