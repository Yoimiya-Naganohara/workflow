//! DecompositionEngine — the **single authority** for task splitting.
//!
//! # Why this exists
//!
//! Phase 2C established that the system is an "execution kernel", not a
//! "task generator".  Phase 3's job is to add the generation capability
//! without breaking the kernel's contract:
//!
//! - **Scheduler** freezed — no pipeline, no policy
//! - **DispatchDecider** — approve/reject/retry/escalate
//! - **DecompositionEngine** — decide whether a task should be split
//!
//! The engine is called FROM the scheduler (between `mark_dispatching`
//! and `decider.decide`), but it does NOT leak into the dispatch path.
//! The scheduler only sees `should_decompose → true/false`.
//!
//! # Structural Tension (not "score")
//!
//! Complexity is NOT a numeric score with an opaque threshold.  It is a
//! **transparent vector** of named dimensions that can be inspected,
//! logged, and overridden independently:
//!
//! ```text
//! StructuralTension {
//!     domain_count:     how many distinct domains are visible in the goal
//!     dependency_depth: how deep in the decomposition tree
//!     ambiguity:        lexical signals of under-specification
//!     role_diversity:   how many different roles the task might need
//! }
//! ```
//!
//! Each dimension is independent.  A task with `domain_count=3` but
//! `ambiguity=0` has high pressure but is well-specified — it can be
//! split by domain.  A task with `domain_count=1` but `ambiguity=0.8`
//! needs clarification, not splitting.
//!
//! # Graph Mutation Authority
//!
//! The `DecompositionEngine` is the **only component** that can create
//! subtasks in the graph (besides direct `spawn_agent` tool calls).
//! It calls `TaskGraph::spawn_child()` directly because graph mutation
//! is its sole responsibility.

use crate::core::types::TaskId;
use crate::runtime::task_graph::{TaskGraph, TaskNode};
use std::collections::HashMap;

// ============================================================================
//  StructuralTension — the "why" behind decomposition
// ============================================================================

/// A transparent vector of structural pressure dimensions.
///
/// Each field is independent and inspectable.  The `should_decompose`
/// method compares against a threshold, but that is a convenience —
/// the real value is in the dimensions themselves.
#[derive(Debug, Clone)]
pub struct StructuralTension {
    /// Number of distinct domains visible in the task goal.
    /// Derived from `@role` mentions and domain keywords.
    pub domain_count: u32,
    /// Depth of this task in the decomposition tree.
    /// Root = 0, first child = 1, etc.
    pub dependency_depth: u32,
    /// Lexical ambiguity: ratio of underspecified terms
    /// (question marks, "something", "etc", placeholders).
    pub ambiguity: f32,
    /// How many different roles might be needed.
    /// Roughly: number of `@role` patterns in the goal + 1.
    pub role_diversity: u32,
}

impl StructuralTension {
    /// Compute tension for a node in the context of its graph.
    pub fn compute(node: &TaskNode, graph: &TaskGraph) -> Self {
        let domain_count = Self::estimate_domain_count(node);
        let dependency_depth = graph.ancestor_chain(node.id).len().saturating_sub(1) as u32;
        let ambiguity = Self::estimate_ambiguity(&node.goal);
        let role_diversity = Self::estimate_role_diversity(&node.goal);
        Self {
            domain_count,
            dependency_depth,
            ambiguity,
            role_diversity,
        }
    }

    /// True if any dimension exceeds the threshold.
    pub fn should_decompose(&self, threshold: &TensionThreshold) -> bool {
        self.domain_count > threshold.max_domain_count
            || self.dependency_depth > threshold.max_dependency_depth
            || self.ambiguity > threshold.max_ambiguity
            || self.role_diversity > threshold.max_role_diversity
    }

    // ── Heuristic estimators (Phase 3 Step 0) ──
    // These are intentionally simple.  Future versions can use
    // embedding-based similarity or LLM calls.

    /// Count `@role` mentions + topic shift words.
    fn estimate_domain_count(node: &TaskNode) -> u32 {
        let goal = &node.goal.to_lowercase();
        let mut count = 1u32;
        // Count @role mentions
        for word in goal.split_whitespace() {
            if word.starts_with('@') {
                count += 1;
            }
        }
        // Count topic shift indicators
        for marker in &[
            "and also",
            "separately",
            "meanwhile",
            "additionally",
            "moreover",
            "furthermore",
        ] {
            if goal.contains(marker) {
                count += 1;
            }
        }
        count.min(10)
    }

    /// Ratio of ambiguity signals in the goal text.
    fn estimate_ambiguity(goal: &str) -> f32 {
        if goal.is_empty() {
            return 1.0;
        }
        let signals = ["?", "？", "TODO", "todo", "etc", "etc.", "something"];
        let signal_count: usize = signals.iter().map(|s| goal.matches(s).count()).sum();
        let words = goal.split_whitespace().count().max(1);
        (signal_count as f32 / words as f32).clamp(0.0, 1.0)
    }

    /// Count `@role` mentions as role diversity.
    fn estimate_role_diversity(goal: &str) -> u32 {
        let mut roles = std::collections::HashSet::new();
        for word in goal.split_whitespace() {
            if let Some(role) = word.strip_prefix('@') {
                roles.insert(role.to_string());
            }
        }
        // Also detect explicit role keywords
        for keyword in &[
            "backend", "frontend", "api", "database", "test", "deploy", "security", "docs",
        ] {
            if goal.to_lowercase().contains(keyword) {
                roles.insert(keyword.to_string());
            }
        }
        roles.len().max(1) as u32
    }
}

/// Threshold for decomposition decisions.
///
/// Each dimension is compared independently.  The default values are
/// conservative — they only trigger for clearly multi-domain tasks.
#[derive(Debug, Clone)]
pub struct TensionThreshold {
    pub max_domain_count: u32,
    pub max_dependency_depth: u32,
    pub max_ambiguity: f32,
    pub max_role_diversity: u32,
}

impl Default for TensionThreshold {
    fn default() -> Self {
        Self {
            max_domain_count: 3,
            max_dependency_depth: 1,
            max_ambiguity: 0.4,
            max_role_diversity: 2,
        }
    }
}

// ============================================================================
//  DecompositionEngine
// ============================================================================

/// The single authority for deciding whether and how to split a task.
///
/// # Contract
///
/// - `should_decompose` must be cheap (no LLM calls).
/// - `decompose` mutates the graph via `spawn_child`.
/// - After `decompose`, the original task is marked `Decomposed`.
/// - The caller (scheduler) skips the decider for this task on this tick.
pub trait DecompositionEngine: Send + Sync {
    /// Quick check — should this task be decomposed?
    /// Called with the graph in a read-locked state.
    fn should_decompose(&self, task_id: TaskId, graph: &TaskGraph) -> bool;

    /// Split the task into subtasks, mutating the graph.
    /// Returns the IDs of the newly created subtasks.
    fn decompose(&self, task_id: TaskId, graph: &mut TaskGraph) -> Vec<TaskId>;
}

// ============================================================================
//  DefaultDecompositionEngine — transparency-first implementation
// ============================================================================

/// The default implementation uses [`StructuralTension`] and a
/// [`TensionThreshold`] to decide decomposition.
///
/// # Transparency
///
/// Every decomposition decision can be explained by reading the tension
/// dimensions.  This is not a black box — it is a transparent rule with
/// independently tunable knobs.
pub struct DefaultDecompositionEngine {
    threshold: TensionThreshold,
}

impl DefaultDecompositionEngine {
    pub fn new(threshold: TensionThreshold) -> Self {
        Self { threshold }
    }

    /// Log the tension vector for debugging / observability.
    fn log_tension(task_id: TaskId, tension: &StructuralTension, decision: bool) {
        tracing::debug!(
            "decomposition: task {:02x}.. tension(domains={}, depth={}, ambiguity={:.2}, roles={}) → {}",
            task_id[0],
            tension.domain_count,
            tension.dependency_depth,
            tension.ambiguity,
            tension.role_diversity,
            if decision { "DECOMPOSE" } else { "execute" }
        );
    }
}

impl DecompositionEngine for DefaultDecompositionEngine {
    fn should_decompose(&self, task_id: TaskId, graph: &TaskGraph) -> bool {
        let Some(node) = graph.get(&task_id) else {
            return false;
        };
        // Never decompose a leaf that already has children.
        if !node.children.is_empty() {
            return false;
        }
        let tension = StructuralTension::compute(node, graph);
        let decision = tension.should_decompose(&self.threshold);
        Self::log_tension(task_id, &tension, decision);
        decision
    }

    fn decompose(&self, task_id: TaskId, graph: &mut TaskGraph) -> Vec<TaskId> {
        let Some(node) = graph.get(&task_id) else {
            return Vec::new();
        };
        let goal = node.goal.clone();

        // Phase 3 Step 0: simple split by @role markers.
        // Future: LLM-based decomposition plan, embedding similarity, etc.
        let mut subtask_goals: Vec<String> = Vec::new();
        let mut current = String::new();

        for line in goal.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('@') || trimmed.starts_with('-') {
                if !current.is_empty() && current != goal {
                    subtask_goals.push(current.trim().to_string());
                }
                current = trimmed.to_string();
            } else if !trimmed.is_empty() && !current.is_empty() {
                current.push(' ');
                current.push_str(trimmed);
            }
        }
        if !current.is_empty() && current != goal {
            subtask_goals.push(current.trim().to_string());
        }

        // If no clear split found, split by sequential paragraphs.
        if subtask_goals.len() < 2 {
            subtask_goals.clear();
            for paragraph in goal.split("\n\n") {
                let p = paragraph.trim();
                if !p.is_empty() && p != goal {
                    subtask_goals.push(p.to_string());
                }
            }
        }

        // Create subtasks.
        let mut children = Vec::new();
        for sg in &subtask_goals {
            if let Some(cid) = graph.spawn_child(task_id, sg) {
                children.push(cid);
            }
        }

        // Mark parent as Decomposed — it will auto-complete when all
        // children reach `Completed` (AllCompleted policy).
        graph.mark_decomposed(task_id).ok();

        if !children.is_empty() {
            tracing::info!(
                "decomposition: task {:02x}.. → {} subtask(s)",
                task_id[0],
                children.len()
            );
        }

        children
    }
}

// ============================================================================
//  NoopDecompositionEngine — for Phase 2 backward compatibility
// ============================================================================

/// A decomposition engine that never splits.  Used when decomposition
/// is not yet enabled (or for tasks that must remain atomic).
pub struct NoopDecompositionEngine;

impl DecompositionEngine for NoopDecompositionEngine {
    fn should_decompose(&self, _task_id: TaskId, _graph: &TaskGraph) -> bool {
        false
    }

    fn decompose(&self, _task_id: TaskId, _graph: &mut TaskGraph) -> Vec<TaskId> {
        Vec::new()
    }
}

// ============================================================================
//  Phase 3 — TaskOutcome / Capability / Role / Escalation
// ============================================================================

use crate::core::types::AgentId;

#[derive(Debug, Clone)]
pub struct TaskOutcome {
    pub task_id: TaskId,
    pub agent_id: Option<AgentId>,
    pub role: String,
    pub success: bool,
    pub latency_ms: u64,
    pub tokens_input: u32,
    pub tokens_output: u32,
}

pub struct TaskOutcomeStore {
    outcomes: Vec<TaskOutcome>,
    by_role: HashMap<String, Vec<usize>>,
}

impl TaskOutcomeStore {
    pub fn new() -> Self { Self { outcomes: Vec::new(), by_role: HashMap::new() } }
    pub fn record(&mut self, o: TaskOutcome) {
        let idx = self.outcomes.len();
        let role = o.role.clone();
        self.outcomes.push(o);
        self.by_role.entry(role).or_default().push(idx);
    }
    pub fn failure_rate(&self, _keywords: &[&str]) -> f32 {
        if self.outcomes.is_empty() { return 0.0; }
        self.outcomes.iter().filter(|o| !o.success).count() as f32 / self.outcomes.len() as f32
    }
    pub fn failure_rate_by_role(&self, role: &str) -> f32 {
        self.by_role.get(role).map(|indices: &Vec<usize>| {
            if indices.is_empty() { return 0.0; }
            indices.iter().filter(|&&idx| !self.outcomes[idx].success).count() as f32 / indices.len() as f32
        }).unwrap_or(0.0)
    }
    pub fn recent(&self, n: usize) -> &[TaskOutcome] {
        let start = self.outcomes.len().saturating_sub(n);
        &self.outcomes[start..]
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityProfile {
    pub role: String,
    pub success_rate: f32,
    pub avg_latency_ms: u64,
    pub avg_token_cost: u32,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
}

pub struct CapabilityRegistry {
    profiles: HashMap<String, CapabilityProfile>,
}

impl CapabilityRegistry {
    pub fn new() -> Self { Self { profiles: HashMap::new() } }
    pub fn get(&self, role: &str) -> Option<&CapabilityProfile> { self.profiles.get(role) }
    pub fn all(&self) -> Vec<CapabilityProfile> { self.profiles.values().cloned().collect() }
    pub fn record_outcome(&mut self, outcome: &TaskOutcome) {
        let entry = self.profiles.entry(outcome.role.clone()).or_insert(CapabilityProfile {
            role: outcome.role.clone(), success_rate: 0.0, avg_latency_ms: 0, avg_token_cost: 0,
            completed_tasks: 0, failed_tasks: 0,
        });
        let total = entry.completed_tasks + entry.failed_tasks + 1;
        entry.avg_latency_ms = ((entry.avg_latency_ms as u64 * (total - 1).max(1) as u64) + outcome.latency_ms) / total as u64;
        entry.avg_token_cost = ((entry.avg_token_cost as u32 * (total - 1).max(1) as u32) + outcome.tokens_input + outcome.tokens_output) / total as u32;
        if outcome.success { entry.completed_tasks += 1; } else { entry.failed_tasks += 1; }
        entry.success_rate = entry.completed_tasks as f32 / (entry.completed_tasks + entry.failed_tasks).max(1) as f32;
    }
}

#[derive(Debug, Clone)]
pub struct RoleScore {
    pub role: String,
    pub total_score: f32,
    pub skill_match: f32,
    pub success_score: f32,
    pub latency_score: f32,
    pub cost_score: f32,
}

#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub role: String,
    pub confidence: f32,
    pub capability_score: f32,
    pub skill_match: f32,
}

pub trait RoleSelector: Send + Sync {
    fn score_all(&self, task: &TaskNode, candidates: &[CapabilityProfile]) -> Vec<RoleScore>;
    fn select(&self, task: &TaskNode, candidates: &[CapabilityProfile]) -> RoutingDecision {
        let mut scored = self.score_all(task, candidates);
        scored.sort_by(|a,b| b.total_score.partial_cmp(&a.total_score).unwrap_or(std::cmp::Ordering::Equal));
        match scored.into_iter().next() {
            Some(top) => RoutingDecision { role: top.role, confidence: top.total_score, capability_score: top.success_score, skill_match: top.skill_match },
            None => RoutingDecision { role: "worker".to_string(), confidence: 0.0, capability_score: 0.0, skill_match: 0.0 },
        }
    }
}

pub struct DefaultRoleSelector;
impl DefaultRoleSelector {
    fn skill_match(goal: &str, role: &str) -> f32 {
        let g = goal.to_lowercase();
        match role {
            "developer" | "backend" | "frontend" => {
                if g.contains("api")||g.contains("backend")||g.contains("frontend")||g.contains("database")||g.contains("ui") { 0.9 }
                else if g.contains("implement")||g.contains("build")||g.contains("code") { 0.8 } else { 0.3 }
            }
            "tester" => { if g.contains("test")||g.contains("verify")||g.contains("qa") { 0.95 } else { 0.2 } }
            "security_auditor" => { if g.contains("security")||g.contains("audit")||g.contains("threat") { 0.95 } else { 0.1 } }
            "reviewer" => { if g.contains("review")||g.contains("inspect") { 0.9 } else { 0.3 } }
            "planner" => { if g.contains("plan")||g.contains("design")||g.contains("architecture") { 0.9 } else { 0.3 } }
            "devops" => { if g.contains("deploy")||g.contains("infra")||g.contains("ci") { 0.95 } else { 0.2 } }
            "researcher" => { if g.contains("research")||g.contains("analyze") { 0.9 } else { 0.3 } }
            "general_business_analyst" => { if g.contains("requirement")||g.contains("spec")||g.contains("stakeholder") { 0.9 } else { 0.2 } }
            _ => 0.5,
        }
    }
}

impl RoleSelector for DefaultRoleSelector {
    fn score_all(&self, task: &TaskNode, candidates: &[CapabilityProfile]) -> Vec<RoleScore> {
        if candidates.is_empty() {
            let inferred = "developer".to_string();
            return vec![RoleScore { role: inferred, total_score: 1.0, skill_match: 1.0, success_score: 0.0, latency_score: 0.5, cost_score: 0.5 }];
        }
        candidates.iter().map(|c| {
            let skill = Self::skill_match(&task.goal, &c.role);
            let lat_norm = 1.0 - (c.avg_latency_ms as f32 / 10_000.0).clamp(0.0, 1.0);
            let cost_norm = 1.0 - (c.avg_token_cost as f32 / 10_000.0).clamp(0.0, 1.0);
            let total = 0.40 * skill + 0.30 * c.success_rate + 0.20 * lat_norm + 0.10 * cost_norm;
            RoleScore { role: c.role.clone(), total_score: total, skill_match: skill, success_score: c.success_rate, latency_score: lat_norm, cost_score: cost_norm }
        }).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EscalationReason {
    RepeatedFailure { count: u32, last_error: String },
    NoCapableRole { confidence: f32 },
    BudgetExceeded { requested: u64, remaining: i64 },
    HumanRequired { reason: String },
}

pub trait EscalationPolicy: Send + Sync {
    fn should_escalate(&self, _task: &TaskNode, recent_outcomes: &[TaskOutcome]) -> Option<EscalationReason>;
}

pub struct DefaultEscalationPolicy {
    pub max_consecutive_failures: u32,
    pub latency_threshold_ms: u64,
}

impl Default for DefaultEscalationPolicy {
    fn default() -> Self { Self { max_consecutive_failures: 3, latency_threshold_ms: 30_000 } }
}

impl EscalationPolicy for DefaultEscalationPolicy {
    fn should_escalate(&self, _task: &TaskNode, recent_outcomes: &[TaskOutcome]) -> Option<EscalationReason> {
        let fails = recent_outcomes.iter().rev().take_while(|o| !o.success).count() as u32;
        if fails >= self.max_consecutive_failures {
            let last = recent_outcomes.last().map(|o| format!("Failed after {}ms", o.latency_ms)).unwrap_or_default();
            return Some(EscalationReason::RepeatedFailure { count: fails, last_error: last });
        }
        if let Some(last) = recent_outcomes.last() {
            if last.latency_ms > self.latency_threshold_ms {
                return Some(EscalationReason::HumanRequired { reason: format!("Latency {}ms > {}ms", last.latency_ms, self.latency_threshold_ms) });
            }
        }
        None
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::TaskId;

    fn setup_task(goal: &str) -> (TaskGraph, TaskId) {
        let mut graph = TaskGraph::new();
        let id = graph.spawn_root(goal);
        (graph, id)
    }

    #[test]
    fn test_simple_goal_no_decomposition() {
        let (graph, id) = setup_task("Write a Rust function that parses CSV");
        let node = graph.get(&id).unwrap();
        let tension = StructuralTension::compute(node, &graph);
        assert_eq!(tension.domain_count, 1);
        assert!(!tension.should_decompose(&TensionThreshold::default()));
    }

    #[test]
    fn test_multi_domain_triggers_decomposition() {
        let (graph, id) = setup_task(
            "Build a full-stack app with @backend API, @frontend UI, and @database schema",
        );
        let node = graph.get(&id).unwrap();
        let tension = StructuralTension::compute(node, &graph);
        assert!(tension.domain_count >= 3);
        assert!(tension.should_decompose(&TensionThreshold::default()));
    }

    #[test]
    fn test_ambiguous_goal_high_ambiguity() {
        let (graph, id) = setup_task("Do something with the thing? Maybe add etc?");
        let node = graph.get(&id).unwrap();
        let tension = StructuralTension::compute(node, &graph);
        assert!(tension.ambiguity > 0.3);
    }

    #[test]
    fn test_decomposition_engine_creates_subtasks() {
        let engine = DefaultDecompositionEngine::new(TensionThreshold::default());
        let (mut graph, id) = setup_task(
            "Build a web app\n@backend API design\n@frontend login page\n@database schema",
        );
        assert!(engine.should_decompose(id, &graph));
        let children = engine.decompose(id, &mut graph);
        assert!(children.len() >= 2);
        // Parent should be marked Decomposed.
        assert_eq!(
            graph.get(&id).unwrap().status,
            crate::runtime::task_graph::TaskStatus::Decomposed
        );
    }

    #[test]
    fn test_noop_engine_never_decomposes() {
        let engine = NoopDecompositionEngine;
        let (graph, id) = setup_task("Complex multi-domain @backend @frontend @database task");
        assert!(!engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_tension_threshold_customization() {
        let threshold = TensionThreshold {
            max_domain_count: 5,
            max_role_diversity: 5,
            ..Default::default()
        };
        let (graph, id) = setup_task("Simple single-domain task");
        let node = graph.get(&id).unwrap();
        let tension = StructuralTension::compute(node, &graph);
        // With raised thresholds, a simple task should NOT trigger.
        assert!(!tension.should_decompose(&threshold));
    }
}
