//! DecompositionEngine — the **single authority** for task splitting.
//!
//! The engine delegates all heuristic analysis to a `dyn GoalAnalyzer`
//! (trait in `embedding_analyzer.rs`).  The engine never touches embedding
//! vectors, async infrastructure, or keyword lists.
//!
//! ```text
//! DefaultDecompositionEngine
//!   └── Arc<dyn GoalAnalyzer>
//!         ├── EmbeddingGoalAnalyzer (cosine similarity, production)
//!         └── MockGoalAnalyzer      (fixed values, tests)
//! ```

use std::sync::Arc;

use crate::core::types::TaskId;
use crate::runtime::embedding_analyzer::GoalAnalyzer;
use crate::runtime::task_graph::{TaskGraph, TaskNode};

// ============================================================================
//  StructuralTension — the "why" behind decomposition
// ============================================================================

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

// ============================================================================
//  TensionThreshold
// ============================================================================

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

// ============================================================================
//  DecompositionEngine trait
// ============================================================================

pub trait DecompositionEngine: Send + Sync {
    fn should_decompose(&self, task_id: TaskId, graph: &TaskGraph) -> bool;
    fn decompose(&self, task_id: TaskId, graph: &mut TaskGraph) -> Vec<TaskId>;
}

// ============================================================================
//  DefaultDecompositionEngine
// ============================================================================

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
                if let Some((role, _confidence)) = self.analyzer.estimate_role(sg) {
                    if let Some(child) = graph.get_mut(&cid) {
                        child.role = Some(role);
                    }
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

// ============================================================================
//  NoopDecompositionEngine
// ============================================================================

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
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::TaskId;
    use crate::runtime::embedding_analyzer::MockGoalAnalyzer;
    use crate::runtime::task_graph::TaskStatus;

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
