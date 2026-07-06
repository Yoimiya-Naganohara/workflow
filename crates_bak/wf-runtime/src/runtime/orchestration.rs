//! Orchestration layer — dispatch, decomposition, capability, goal analysis, escalation.
//!
//! This module was created by merging 5 thin files into one.
#![allow(clippy::empty_line_after_doc_comments)]
//!

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::runtime::AgentRuntime;
use wf_core::simd::cosine_similarity_384;
use wf_core::task_graph::{TaskGraph, TaskNode};
use wf_core::{AgentId, ChildAgentConfig, EMBEDDING_DIM, SpawnDecision, SpawnRejection, TaskId};

/// DispatchDecider — the **single authority** for task execution decisions.
///
/// # Why this exists
///
/// Before Phase 2C, the system had three overlapping decision authorities:
///
/// 1. **TaskGraph** — decided completion semantics via `aggregation_policy`
/// 2. **Scheduler** — ran pipeline, decided spawn/reject/retry
/// 3. **Pipeline/L1/L2** — decided approval via opaque `process_with_text()`
///
/// This led to **decision authority drift**: a single task spawn could be
/// rejected at the pipeline, retried by the scheduler, or silently ignored.
///
/// # What this does
///
/// `DispatchDecider` is the **single entry point** for ALL task execution
/// decisions.  Every task goes through exactly one `decide()` call, which
/// returns one of four outcomes:
///
/// ```text
/// dispatching_task
///   └── DispatchDecider::decide()
///         ├── Approved(config)  → agent created, mark_running
///         ├── Rejected(reason)  → mark_rejected, terminal
///         ├── RetryLater        → mark_created, next tick
///         └── Escalate(target)  → mark_blocked, notify
/// ```
///
/// The scheduler no longer knows about pipelines, L1, L2, or retry
/// strategies — it only dispatches.

//  DispatchDecision — the single outcome type

/// The exclusive set of outcomes from `DispatchDecider::decide()`.
///
/// No other decision path should exist in the system.  If you need a new
/// outcome, add it here, not as a special case in the scheduler.
#[derive(Debug)]
pub enum DispatchDecision {
    /// Approved: create an agent and execute.
    Approved { config: ChildAgentConfig },
    /// Rejected: the task will never run (L1/L2 rejection, budget, etc.).
    Rejected { reason: SpawnRejection },
    /// Transient error: retry on the next dispatch tick.
    RetryLater { reason: String },
    /// The task should be escalated to a different role or a human.
    Escalate { target_role: String, reason: String },
}

//  DispatchDecider trait — single decision entry point

/// The **single authority** for whether a task should run, wait, or stop.
///
/// # Contract
///
/// - Must be `Send + Sync` (called from the event loop).
/// - Must be deterministic for the same input (no hidden state machine).
/// - Must NOT call back into the `TaskGraph` or `Scheduler` (no reentrancy).
#[async_trait]
pub trait DispatchDecider: Send + Sync {
    /// Decide what to do with a task that is about to be scheduled.
    ///
    /// The task is already in `Dispatching` state (anti-double-dispatch lock).
    /// The scheduler will apply the result without further policy calls.
    async fn decide(&self, task_id: TaskId, goal: &str, role: &str) -> DispatchDecision;
}

//  PipelineDispatchDecider — wraps the existing L-1/L0/L1/L2 pipeline

/// The standard decider that runs the full decision pipeline
/// (L-1 admission → L0 circuit breaker → L1 experience → L2 audit).
///
/// This is what Phase 2A/B used as inline code in the scheduler.  Extracted
/// here so the scheduler no longer knows about pipeline internals.
///
/// # Future
///
/// - Phase 3: `ComplexityDispatchDecider` adds complexity estimation
/// - Phase 4: `LearningDispatchDecider` incorporates experience feedback
/// - Always wrapped, never modified in-place
pub struct PipelineDispatchDecider {
    runtime: Arc<RwLock<AgentRuntime>>,
}

impl PipelineDispatchDecider {
    pub fn new(runtime: Arc<RwLock<AgentRuntime>>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl DispatchDecider for PipelineDispatchDecider {
    async fn decide(&self, _: TaskId, goal: &str, role: &str) -> DispatchDecision {
        let rt = self.runtime.read().await;
        match rt
            .process_with_text(goal, role, "default", 1000, 0, None, None)
            .await
        {
            Ok(SpawnDecision::Approved(config)) => DispatchDecision::Approved { config },
            Ok(SpawnDecision::Rejected(reason)) => DispatchDecision::Rejected { reason },
            Err(e) => DispatchDecision::RetryLater {
                reason: format!("Pipeline error: {}", e),
            },
        }
    }
}

/// DecompositionEngine — the **single authority** for task splitting.
///
/// The engine delegates all heuristic analysis to a `dyn GoalAnalyzer`
/// (trait in `embedding_analyzer.rs`).  The engine never touches embedding
/// vectors, async infrastructure, or keyword lists.
///
/// ```text
/// DefaultDecompositionEngine
///   └── Arc<dyn GoalAnalyzer>
///         ├── EmbeddingGoalAnalyzer (cosine similarity, production)
///         └── MockGoalAnalyzer      (fixed values, tests)
/// ```

//  StructuralTension — the "why" behind decomposition

pub struct StructuralTension {
    pub domain_count: u32,
    pub dependency_depth: u32,
    pub ambiguity: f32,
    pub role_diversity: u32,
    pub readability: f32,
    pub uncertainty: f32,
}

impl StructuralTension {
    pub fn compute(node: &TaskNode, graph: &TaskGraph, analyzer: &dyn GoalAnalyzer) -> Self {
        let domain_count = analyzer.estimate_domain_count(&node.goal);
        let dependency_depth = graph.ancestor_chain(node.id).len() as u32;
        let ambiguity = analyzer.estimate_ambiguity(&node.goal);
        let role_diversity = Self::count_role_signals(&node.goal, analyzer);
        Self {
            domain_count,
            dependency_depth,
            ambiguity,
            role_diversity,
            readability: 0.0,
            uncertainty: 0.0,
        }
    }

    pub fn should_decompose(&self, threshold: &TensionThreshold) -> bool {
        self.domain_count > threshold.max_domain_count
            || self.dependency_depth > threshold.max_dependency_depth
            || self.ambiguity > threshold.max_ambiguity
            || self.role_diversity > threshold.max_role_diversity
    }

    fn count_role_signals(goal: &str, analyzer: &dyn GoalAnalyzer) -> u32 {
        let mut count = goal
            .split_whitespace()
            .filter(|w| w.starts_with('@'))
            .count() as u32;
        if analyzer.estimate_role(goal).is_some() {
            count += 1;
        }
        count
    }
}

//  TensionThreshold

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
            max_domain_count: 2,
            max_dependency_depth: 3,
            max_ambiguity: 0.5,
            max_role_diversity: 1,
        }
    }
}

//  DecompositionEngine trait

pub trait DecompositionEngine: Send + Sync {
    fn should_decompose(&self, task_id: TaskId, graph: &TaskGraph) -> bool;
    fn decompose(&self, task_id: TaskId, graph: &mut TaskGraph) -> Vec<TaskId>;
}

//  DefaultDecompositionEngine

pub struct DefaultDecompositionEngine {
    threshold: TensionThreshold,
    analyzer: Arc<dyn GoalAnalyzer>,
}

impl DefaultDecompositionEngine {
    pub fn new(threshold: TensionThreshold, analyzer: Arc<dyn GoalAnalyzer>) -> Self {
        Self {
            threshold,
            analyzer,
        }
    }

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
        if !node.children.is_empty() {
            return false;
        }
        let tension = StructuralTension::compute(node, graph, &*self.analyzer);
        let decision = tension.should_decompose(&self.threshold);
        Self::log_tension(task_id, &tension, decision);
        decision
    }

    fn decompose(&self, task_id: TaskId, graph: &mut TaskGraph) -> Vec<TaskId> {
        let Some(node) = graph.get(&task_id) else {
            return Vec::new();
        };
        let goal = node.goal.clone();

        // Split by @role markers or paragraphs.
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
        if subtask_goals.len() < 2 {
            subtask_goals.clear();
            for paragraph in goal.split("\n\n") {
                let p = paragraph.trim();
                if !p.is_empty() && p != goal {
                    subtask_goals.push(p.to_string());
                }
            }
        }

        // Create subtasks with role inference from the analyzer.
        let mut children = Vec::new();
        for sg in &subtask_goals {
            if let Some(cid) = graph.spawn_child(task_id, sg) {
                if let Some((role, _)) = self.analyzer.estimate_role(sg)
                    && let Some(child) = graph.get_mut(&cid)
                {
                    child.role = Some(role);
                }
                children.push(cid);
            }
        }
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

//  NoopDecompositionEngine

pub struct NoopDecompositionEngine;

impl DecompositionEngine for NoopDecompositionEngine {
    fn should_decompose(&self, _: TaskId, _: &TaskGraph) -> bool {
        false
    }
    fn decompose(&self, _: TaskId, _: &mut TaskGraph) -> Vec<TaskId> {
        Vec::new()
    }
}

//  Tests

#[cfg(test)]
mod dispatch_tests {

    use crate::runtime::orchestration::*;
    use wf_core::TaskId;
    use wf_core::task_graph::{TaskGraph, TaskStatus};

    fn setup_task(goal: &str) -> (TaskGraph, TaskId) {
        let mut graph = TaskGraph::new();
        let id = graph.spawn_root(goal);
        (graph, id)
    }

    fn mock_engine(domain: u32, ambiguity: f32, role_count: u32) -> DefaultDecompositionEngine {
        DefaultDecompositionEngine::new(
            TensionThreshold::default(),
            Arc::new(MockGoalAnalyzer {
                domain_count: domain,
                ambiguity,
                role: if role_count > 0 {
                    Some(("developer".into(), 0.9))
                } else {
                    None
                },
            }),
        )
    }

    #[test]
    fn test_simple_goal_no_decomposition() {
        let (graph, id) = setup_task("Single domain task");
        let engine = mock_engine(1, 0.0, 0);
        assert!(!engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_multi_domain_triggers_decomposition() {
        let (graph, id) = setup_task("Multi-domain task");
        let engine = mock_engine(3, 0.0, 0);
        assert!(engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_high_ambiguity_triggers_decomposition() {
        let (graph, id) = setup_task("Vague task");
        let engine = mock_engine(1, 0.8, 0);
        assert!(engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_low_ambiguity_no_decomposition() {
        let (graph, id) = setup_task("Specific task");
        let engine = mock_engine(1, 0.1, 0);
        assert!(!engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_decomposition_engine_creates_subtasks() {
        let (mut graph, id) = setup_task(
            "Build a web app\n@backend API design\n@frontend login page\n@database schema",
        );
        let engine = mock_engine(3, 0.0, 1);
        assert!(engine.should_decompose(id, &graph));
        let children = engine.decompose(id, &mut graph);
        assert!(children.len() >= 2);
        assert_eq!(graph.get(&id).unwrap().status, TaskStatus::Decomposed);
    }

    #[test]
    fn test_noop_engine_never_decomposes() {
        let engine = NoopDecompositionEngine;
        let (graph, id) = setup_task("Any task");
        assert!(!engine.should_decompose(id, &graph));
    }

    #[test]
    fn test_tension_threshold_customization() {
        let threshold = TensionThreshold {
            max_domain_count: 5,
            max_role_diversity: 5,
            ..Default::default()
        };
        let (graph, id) = setup_task("Simple task");
        let engine = DefaultDecompositionEngine::new(
            threshold,
            Arc::new(MockGoalAnalyzer {
                domain_count: 1,
                ambiguity: 0.1,
                role: Some(("developer".into(), 0.9)),
            }),
        );
        assert!(!engine.should_decompose(id, &graph));
    }
}

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
    pub fn new() -> Self {
        Self {
            outcomes: Vec::new(),
            by_role: HashMap::new(),
        }
    }
}

impl Default for TaskOutcomeStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskOutcomeStore {
    pub fn record(&mut self, o: TaskOutcome) {
        let idx = self.outcomes.len();
        let role = o.role.clone();
        self.outcomes.push(o);
        self.by_role.entry(role).or_default().push(idx);
    }
    pub fn failure_rate(&self, _: &[&str]) -> f32 {
        if self.outcomes.is_empty() {
            return 0.0;
        }
        self.outcomes.iter().filter(|o| !o.success).count() as f32 / self.outcomes.len() as f32
    }
    pub fn failure_rate_by_role(&self, role: &str) -> f32 {
        self.by_role
            .get(role)
            .map(|indices| {
                if indices.is_empty() {
                    return 0.0;
                }
                indices
                    .iter()
                    .filter(|&&idx| !self.outcomes[idx].success)
                    .count() as f32
                    / indices.len() as f32
            })
            .unwrap_or(0.0)
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
    pub embedding: Option<[f32; EMBEDDING_DIM]>,
}

pub struct CapabilityRegistry {
    profiles: HashMap<String, CapabilityProfile>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityRegistry {
    pub fn get(&self, role: &str) -> Option<&CapabilityProfile> {
        self.profiles.get(role)
    }
    pub fn get_mut(&mut self, role: &str) -> Option<&mut CapabilityProfile> {
        self.profiles.get_mut(role)
    }
    pub fn all(&self) -> Vec<CapabilityProfile> {
        self.profiles.values().cloned().collect()
    }
    pub fn role_prototypes(&self) -> HashMap<String, [f32; EMBEDDING_DIM]> {
        self.profiles
            .iter()
            .filter_map(|(role, p)| p.embedding.map(|e| (role.clone(), e)))
            .collect()
    }
    pub fn record_outcome(&mut self, outcome: &TaskOutcome) {
        let entry = self
            .profiles
            .entry(outcome.role.clone())
            .or_insert(CapabilityProfile {
                role: outcome.role.clone(),
                success_rate: 0.0,
                avg_latency_ms: 0,
                avg_token_cost: 0,
                completed_tasks: 0,
                failed_tasks: 0,
                embedding: None,
            });
        let total = entry.completed_tasks + entry.failed_tasks + 1;
        let total_u32 = total as u32;
        entry.avg_latency_ms =
            ((entry.avg_latency_ms * (total - 1).max(1)) + outcome.latency_ms) / total;
        entry.avg_token_cost = ((entry.avg_token_cost * (total_u32 - 1).max(1))
            + outcome.tokens_input
            + outcome.tokens_output)
            / total_u32;
        if outcome.success {
            entry.completed_tasks += 1;
        } else {
            entry.failed_tasks += 1;
        }
        entry.success_rate = entry.completed_tasks as f32
            / (entry.completed_tasks + entry.failed_tasks).max(1) as f32;
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
    fn score_all(
        &self,
        task: &wf_core::task_graph::TaskNode,
        candidates: &[CapabilityProfile],
    ) -> Vec<RoleScore>;
    fn select(
        &self,
        task: &wf_core::task_graph::TaskNode,
        candidates: &[CapabilityProfile],
    ) -> RoutingDecision {
        let scored = self.score_all(task, candidates);
        scored
            .into_iter()
            .max_by(|a, b| a.total_score.partial_cmp(&b.total_score).unwrap())
            .map(|top| RoutingDecision {
                role: top.role,
                confidence: top.total_score,
                capability_score: top.success_score,
                skill_match: top.skill_match,
            })
            .unwrap_or(RoutingDecision {
                role: "worker".to_string(),
                confidence: 0.0,
                capability_score: 0.0,
                skill_match: 0.0,
            })
    }
}

pub struct DefaultRoleSelector {
    analyzer: Arc<dyn GoalAnalyzer>,
}

impl DefaultRoleSelector {
    pub fn new(analyzer: Arc<dyn GoalAnalyzer>) -> Self {
        Self { analyzer }
    }
    fn skill_match(goal: &str, role: &str, analyzer: &dyn GoalAnalyzer) -> f32 {
        match analyzer.estimate_role(goal) {
            Some((best_role, conf)) if best_role == role => conf,
            Some(_) => 0.3,
            None => 0.5,
        }
    }
}

impl RoleSelector for DefaultRoleSelector {
    fn score_all(
        &self,
        task: &wf_core::task_graph::TaskNode,
        candidates: &[CapabilityProfile],
    ) -> Vec<RoleScore> {
        if candidates.is_empty() {
            let best = self
                .analyzer
                .estimate_role(&task.goal)
                .map(|(r, _)| r)
                .unwrap_or_else(|| "developer".to_string());
            return vec![RoleScore {
                role: best,
                total_score: 1.0,
                skill_match: 1.0,
                success_score: 0.0,
                latency_score: 0.5,
                cost_score: 0.5,
            }];
        }
        candidates
            .iter()
            .map(|c| {
                let skill = Self::skill_match(&task.goal, &c.role, &*self.analyzer);
                let lat_norm = 1.0 - (c.avg_latency_ms as f32 / 10_000.0).clamp(0.0, 1.0);
                let cost_norm = 1.0 - (c.avg_token_cost as f32 / 10_000.0).clamp(0.0, 1.0);
                let total =
                    0.40 * skill + 0.30 * c.success_rate + 0.20 * lat_norm + 0.10 * cost_norm;
                RoleScore {
                    role: c.role.clone(),
                    total_score: total,
                    skill_match: skill,
                    success_score: c.success_rate,
                    latency_score: lat_norm,
                    cost_score: cost_norm,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod cap_tests {

    use crate::runtime::orchestration::*;

    #[test]
    fn test_capability_registry_prototypes_empty_by_default() {
        let reg = CapabilityRegistry::new();
        assert!(reg.role_prototypes().is_empty());
    }

    #[test]
    fn test_default_role_selector_returns_developer_on_empty() {
        let task = wf_core::task_graph::TaskNode::new([0u8; 16], "Build API");
        let selector = DefaultRoleSelector::new(Arc::new(MockGoalAnalyzer {
            domain_count: 1,
            ambiguity: 0.0,
            role: Some(("developer".into(), 1.0)),
        }));
        let scores = selector.score_all(&task, &[]);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].role, "developer");
    }
}

/// GoalAnalyzer trait — the single decomposition heuristic.
///
/// The only implementation is `EmbeddingGoalAnalyzer`: cosine similarity
/// against pre-computed prototype vectors.  No keywords, no if-else chains.
/// Every decision is a dot product.
///
/// # Data flow
///
/// ```text
/// Async init (once):            Sync inference (per goal):
///   embed("developer")  ───┐    goal_embedding × role_prototypes
///   embed("tester")     ───┤→     → cosine_similarity_384
///   embed("security")   ───┤     → highest score → role + confidence
///   embed(vague_phrase) ───┤
///   embed(domain_phrases) ──┘
/// ```
///
/// No file I/O, no config loading, no polymorphic pattern registry.
/// The prototypes are computed once at startup from role names and
/// reference phrases, stored in the analyzer, and queried via SIMD.

//  GoalAnalyzer trait

pub trait GoalAnalyzer: Send + Sync {
    fn estimate_domain_count(&self, goal: &str) -> u32;
    fn estimate_ambiguity(&self, goal: &str) -> f32;
    fn estimate_role(&self, goal: &str) -> Option<(String, f32)>;
}

//  Reference data — what gets embedded at startup

pub(crate) static ROLE_NAMES: &[&str] = &[
    "developer",
    "tester",
    "security_auditor",
    "reviewer",
    "planner",
    "devops",
    "researcher",
    "general_business_analyst",
];

pub(crate) static AMBIGUITY_PHRASE: &str =
    "Make it better, improve this, fix things up, do something";

pub(crate) static DOMAIN_PHRASES: &[(&str, &str)] = &[
    (
        "backend",
        "Build server-side API, database, authentication, business logic",
    ),
    (
        "frontend",
        "Build UI, client-side, user interface, dashboard, web pages",
    ),
    (
        "database",
        "Schema, tables, migrations, data model, query optimization",
    ),
    (
        "devops",
        "Deploy, CI/CD, Docker, infrastructure, monitoring, scaling",
    ),
    (
        "security",
        "Authentication, authorization, permissions, encryption, audit",
    ),
    (
        "testing",
        "Unit tests, integration tests, QA, validation, assertions",
    ),
];

/// Pre-computed reference embeddings.  Built once at runtime init.
#[derive(Debug, Clone)]
pub struct ReferenceEmbeddings {
    pub role_prototypes: Vec<(String, [f32; EMBEDDING_DIM])>,
    pub ambiguity_reference: [f32; EMBEDDING_DIM],
    pub domain_references: Vec<(String, [f32; EMBEDDING_DIM])>,
}

impl ReferenceEmbeddings {
    pub async fn compute(embedder: &dyn wf_llm::EmbeddingService) -> Self {
        let mut role_protos = Vec::with_capacity(ROLE_NAMES.len());
        for role in ROLE_NAMES {
            if let Ok(emb) = embedder.embed(role).await {
                role_protos.push((role.to_string(), emb));
            }
        }

        let ambiguity_ref = embedder
            .embed(AMBIGUITY_PHRASE)
            .await
            .unwrap_or([0.0; EMBEDDING_DIM]);

        let mut domain_refs = Vec::with_capacity(DOMAIN_PHRASES.len());
        for (label, phrase) in DOMAIN_PHRASES {
            if let Ok(emb) = embedder.embed(phrase).await {
                domain_refs.push((label.to_string(), emb));
            }
        }

        Self {
            role_prototypes: role_protos,
            ambiguity_reference: ambiguity_ref,
            domain_references: domain_refs,
        }
    }
}

//  EmbeddingGoalAnalyzer — the only production GoalAnalyzer

pub struct EmbeddingGoalAnalyzer {
    role_prototypes: Vec<(String, [f32; EMBEDDING_DIM])>,
    ambiguity_reference: [f32; EMBEDDING_DIM],
    domain_references: Vec<(String, [f32; EMBEDDING_DIM])>,
    goal_embedding: Mutex<Option<[f32; EMBEDDING_DIM]>>,
    domain_threshold: f32,
    role_threshold: f32,
}

impl EmbeddingGoalAnalyzer {
    pub fn new(references: ReferenceEmbeddings) -> Self {
        Self {
            role_prototypes: references.role_prototypes,
            ambiguity_reference: references.ambiguity_reference,
            domain_references: references.domain_references,
            goal_embedding: Mutex::new(None),
            domain_threshold: 0.7,
            role_threshold: 0.3,
        }
    }

    pub fn with_goal(
        references: ReferenceEmbeddings,
        goal_embedding: [f32; EMBEDDING_DIM],
    ) -> Self {
        let mut s = Self::new(references);
        s.goal_embedding = Mutex::new(Some(goal_embedding));
        s
    }

    pub fn set_goal_embedding(&self, embedding: [f32; EMBEDDING_DIM]) {
        if let Ok(mut g) = self.goal_embedding.lock() {
            *g = Some(embedding);
        }
    }

    pub fn get_goal_embedding(&self) -> Option<[f32; EMBEDDING_DIM]> {
        self.goal_embedding.lock().ok().and_then(|g| *g)
    }
}

impl GoalAnalyzer for EmbeddingGoalAnalyzer {
    fn estimate_domain_count(&self, _: &str) -> u32 {
        let goal_emb = match self.goal_embedding.lock().ok().and_then(|g| *g) {
            Some(e) => e,
            None => return 0,
        };
        let c = self
            .domain_references
            .iter()
            .filter(|(_, ref_emb)| {
                cosine_similarity_384(&goal_emb, ref_emb) > self.domain_threshold
            })
            .count() as u32;
        if c > 0 { c } else { 1 }
    }

    fn estimate_ambiguity(&self, _: &str) -> f32 {
        match self.goal_embedding.lock().ok().and_then(|g| *g) {
            Some(ref goal_emb) => cosine_similarity_384(goal_emb, &self.ambiguity_reference),
            None => 0.0,
        }
    }

    fn estimate_role(&self, _: &str) -> Option<(String, f32)> {
        let goal_emb = self.goal_embedding.lock().ok().and_then(|g| *g)?;
        let mut best: Option<(String, f32)> = None;
        for (role, prot_emb) in &self.role_prototypes {
            let sim = cosine_similarity_384(&goal_emb, prot_emb);
            if sim > self.role_threshold {
                match &best {
                    Some((_, best_sim)) if sim > *best_sim => best = Some((role.clone(), sim)),
                    None => best = Some((role.clone(), sim)),
                    _ => {}
                }
            }
        }
        best.or_else(|| {
            self.role_prototypes
                .iter()
                .max_by(|a, b| {
                    cosine_similarity_384(&goal_emb, &a.1)
                        .partial_cmp(&cosine_similarity_384(&goal_emb, &b.1))
                        .unwrap()
                })
                .map(|(role, prot_emb)| (role.clone(), cosine_similarity_384(&goal_emb, prot_emb)))
        })
    }
}

//  MockGoalAnalyzer — deterministic, for tests

pub struct MockGoalAnalyzer {
    pub domain_count: u32,
    pub ambiguity: f32,
    pub role: Option<(String, f32)>,
}

impl GoalAnalyzer for MockGoalAnalyzer {
    fn estimate_domain_count(&self, _: &str) -> u32 {
        self.domain_count
    }
    fn estimate_ambiguity(&self, _: &str) -> f32 {
        self.ambiguity
    }
    fn estimate_role(&self, _: &str) -> Option<(String, f32)> {
        self.role.clone()
    }
}

//  Tests

#[cfg(test)]
mod embedding_tests {

    use crate::runtime::orchestration::*;

    #[test]
    fn test_mock_goal_analyzer() {
        let a = MockGoalAnalyzer {
            domain_count: 3,
            ambiguity: 0.8,
            role: Some(("tester".into(), 0.95)),
        };
        assert_eq!(a.estimate_domain_count("anything"), 3);
        assert!((a.estimate_ambiguity("anything") - 0.8).abs() < 1e-6);
        assert_eq!(a.estimate_role("anything"), Some(("tester".into(), 0.95)));
    }

    #[test]
    fn test_goal_analyzer_trait_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EmbeddingGoalAnalyzer>();
        assert_send_sync::<MockGoalAnalyzer>();
    }

    #[test]
    fn test_embedding_analyzer_returns_closest_role_by_cosine_similarity() {
        let mut dev_emb = [0.0f32; EMBEDDING_DIM];
        dev_emb[0] = 1.0;
        let mut test_emb = [0.0f32; EMBEDDING_DIM];
        test_emb[1] = 1.0;
        let protos = ReferenceEmbeddings {
            role_prototypes: vec![("developer".into(), dev_emb), ("tester".into(), test_emb)],
            ambiguity_reference: [0.5; EMBEDDING_DIM],
            domain_references: vec![],
        };
        let mut goal_emb = [0.0f32; EMBEDDING_DIM];
        goal_emb[0] = 1.0;
        let a = EmbeddingGoalAnalyzer::with_goal(protos, goal_emb);
        let role = a.estimate_role("build api");
        assert!(role.is_some());
        assert_eq!(role.unwrap().0, "developer");
    }

    #[test]
    fn test_domain_count_returns_1_when_no_domain_refs() {
        let protos = ReferenceEmbeddings {
            role_prototypes: vec![("dev".into(), [0.5; EMBEDDING_DIM])],
            ambiguity_reference: [0.5; EMBEDDING_DIM],
            domain_references: vec![],
        };
        let a = EmbeddingGoalAnalyzer::with_goal(protos, [0.5; EMBEDDING_DIM]);
        assert_eq!(a.estimate_domain_count("anything"), 1);
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
    fn should_escalate(
        &self,
        _: &TaskNode,
        recent_outcomes: &[TaskOutcome],
    ) -> Option<EscalationReason>;
}

pub struct DefaultEscalationPolicy {
    pub max_consecutive_failures: u32,
    pub latency_threshold_ms: u64,
}

impl Default for DefaultEscalationPolicy {
    fn default() -> Self {
        Self {
            max_consecutive_failures: 3,
            latency_threshold_ms: 30_000,
        }
    }
}

impl EscalationPolicy for DefaultEscalationPolicy {
    fn should_escalate(
        &self,
        _: &TaskNode,
        recent_outcomes: &[TaskOutcome],
    ) -> Option<EscalationReason> {
        let fails = recent_outcomes
            .iter()
            .rev()
            .take_while(|o| !o.success)
            .count() as u32;
        if fails >= self.max_consecutive_failures {
            let last = recent_outcomes
                .last()
                .map(|o| format!("Failed after {}ms", o.latency_ms))
                .unwrap_or_default();
            return Some(EscalationReason::RepeatedFailure {
                count: fails,
                last_error: last,
            });
        }
        if let Some(last) = recent_outcomes.last()
            && last.latency_ms > self.latency_threshold_ms
        {
            return Some(EscalationReason::HumanRequired {
                reason: format!(
                    "Latency {}ms > {}ms",
                    last.latency_ms, self.latency_threshold_ms
                ),
            });
        }
        None
    }
}

#[cfg(test)]
mod escalation_tests {

    use crate::runtime::orchestration::*;
    use wf_core::task_graph::TaskNode;

    fn make_outcome(success: bool, latency_ms: u64) -> TaskOutcome {
        TaskOutcome {
            task_id: [0u8; 16],
            agent_id: None,
            success,
            role: "developer".into(),
            latency_ms,
            tokens_input: 100,
            tokens_output: 50,
        }
    }

    #[test]
    fn test_no_escalation_on_success() {
        let policy = DefaultEscalationPolicy::default();
        let task = TaskNode::new([0u8; 16], "test");
        let outcomes = vec![make_outcome(true, 100)];
        assert!(policy.should_escalate(&task, &outcomes).is_none());
    }

    #[test]
    fn test_escalates_after_three_consecutive_failures() {
        let policy = DefaultEscalationPolicy::default();
        let task = TaskNode::new([0u8; 16], "test");
        let outcomes = vec![
            make_outcome(true, 100),
            make_outcome(false, 500),
            make_outcome(false, 200),
            make_outcome(false, 300),
        ];
        let reason = policy.should_escalate(&task, &outcomes);
        assert!(reason.is_some());
        assert!(matches!(
            reason,
            Some(EscalationReason::RepeatedFailure { count: 3, .. })
        ));
    }

    #[test]
    fn test_two_failures_not_enough() {
        let policy = DefaultEscalationPolicy::default();
        let task = TaskNode::new([0u8; 16], "test");
        let outcomes = vec![
            make_outcome(true, 100),
            make_outcome(false, 200),
            make_outcome(false, 300),
        ];
        assert!(policy.should_escalate(&task, &outcomes).is_none());
    }

    #[test]
    fn test_escalates_on_high_latency() {
        let policy = DefaultEscalationPolicy {
            max_consecutive_failures: 3,
            latency_threshold_ms: 100,
        };
        let task = TaskNode::new([0u8; 16], "test");
        let outcomes = vec![make_outcome(true, 500)];
        assert!(matches!(
            policy.should_escalate(&task, &outcomes),
            Some(EscalationReason::HumanRequired { .. })
        ));
    }
}
