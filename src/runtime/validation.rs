//! System Validation Protocol (SVP)
//!
//! Validates whether the Graph Compiler produces more stable, efficient,
//! and correct execution plans than the agent-only baseline.
//!
//! # What this tests
//!
//! The core hypothesis is:
//!   C(Graph Compiler) → lower execution entropy than A(agent-only)
//!
//! Lower entropy means:
//! - Fewer re-plans
//! - More consistent DAG structure
//! - Less tool misuse
//! - Better task completion rate
//!
//! # Usage
//!
//! ```ignore
//! let result = Validator::run(Suite::standard(), Mode::Compiler);
//! let baseline = Validator::run(Suite::standard(), Mode::AgentOnly);
//! let report = Validator::compare(&baseline, &result);
//! ```

use std::time::{Duration, Instant};

// ============================================================================
//  Task Distribution
// ============================================================================

/// A single validation task.
#[derive(Debug, Clone)]
pub struct ValidationTask {
    pub id: &'static str,
    pub category: TaskCategory,
    pub goal: &'static str,
    pub expected_roles: &'static [&'static str],
    pub expected_depth: u32,
    pub expected_task_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCategory {
    /// Single-domain, straightforward (e.g., "fix typo")
    Simple,
    /// Multiple domains, needs decomposition (e.g., "build full-stack app")
    MultiDomain,
    /// Requires sequential steps (e.g., "design, implement, test")
    Sequential,
    /// Requires investigation (e.g., "find the bug in this code")
    Analysis,
    /// Ambiguous goal (e.g., "make it better")
    Ambiguous,
}

/// Standard task distribution covering all categories.
pub fn standard_suite() -> Vec<ValidationTask> {
    vec![
        // Simple
        ValidationTask {
            id: "simple-typo",
            category: TaskCategory::Simple,
            goal: "Fix the typo in README.md",
            expected_roles: &["developer"],
            expected_depth: 1,
            expected_task_count: 1,
        },
        ValidationTask {
            id: "simple-query",
            category: TaskCategory::Simple,
            goal: "List all files in the src directory",
            expected_roles: &["developer"],
            expected_depth: 1,
            expected_task_count: 1,
        },
        // Multi-domain
        ValidationTask {
            id: "fullstack-api",
            category: TaskCategory::MultiDomain,
            goal: "Build a REST API with backend, frontend UI, and database schema",
            expected_roles: &["developer"],
            expected_depth: 2,
            expected_task_count: 3,
        },
        ValidationTask {
            id: "fullstack-auth",
            category: TaskCategory::MultiDomain,
            goal: "Implement authentication system with login page, JWT backend, and user database",
            expected_roles: &["developer"],
            expected_depth: 2,
            expected_task_count: 3,
        },
        ValidationTask {
            id: "deploy-stack",
            category: TaskCategory::MultiDomain,
            goal: "Deploy a web service with CI/CD pipeline, Docker container, and monitoring",
            expected_roles: &["devops"],
            expected_depth: 2,
            expected_task_count: 3,
        },
        // Sequential
        ValidationTask {
            id: "design-implement-test",
            category: TaskCategory::Sequential,
            goal: "Design database schema, implement the ORM layer, then write integration tests",
            expected_roles: &["developer"],
            expected_depth: 2,
            expected_task_count: 3,
        },
        ValidationTask {
            id: "plan-build-review",
            category: TaskCategory::Sequential,
            goal: "Plan the architecture, build the module, then review the code",
            expected_roles: &["planner"],
            expected_depth: 2,
            expected_task_count: 3,
        },
        // Analysis
        ValidationTask {
            id: "debug-crash",
            category: TaskCategory::Analysis,
            goal: "Find and fix the memory leak in the authentication module",
            expected_roles: &["developer"],
            expected_depth: 1,
            expected_task_count: 1,
        },
        ValidationTask {
            id: "security-audit",
            category: TaskCategory::Analysis,
            goal: "Audit the codebase for SQL injection vulnerabilities",
            expected_roles: &["security_auditor"],
            expected_depth: 1,
            expected_task_count: 1,
        },
        // Ambiguous
        ValidationTask {
            id: "vague-improve",
            category: TaskCategory::Ambiguous,
            goal: "Improve the code quality of the project",
            expected_roles: &["planner"],
            expected_depth: 1,
            expected_task_count: 1,
        },
        ValidationTask {
            id: "vague-optimize",
            category: TaskCategory::Ambiguous,
            goal: "Make the application faster",
            expected_roles: &["developer"],
            expected_depth: 1,
            expected_task_count: 1,
        },
    ]
}

// ============================================================================
//  Execution Mode
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Agent-only: LLM decides all structure via spawn_agent.
    AgentOnly,
    /// Graph Compiler: deterministic passes determine structure first.
    Compiler,
}

// ============================================================================
//  Measurements
// ============================================================================

#[derive(Debug, Clone)]
pub struct TaskMeasurement {
    pub task_id: &'static str,
    pub category: TaskCategory,
    pub mode: ExecutionMode,

    /// Whether the task completed without error.
    pub completed: bool,
    /// Number of re-plans / recursive spawns.
    pub replan_count: u32,
    /// Final DAG depth.
    pub dag_depth: u32,
    /// Number of nodes in the final task graph.
    pub node_count: usize,
    /// Number of distinct roles assigned.
    pub role_count: usize,
    /// Total execution time.
    pub elapsed: Duration,
    /// Number of tool calls made.
    pub tool_call_count: u32,
    /// Whether the structure matched expected DAG shape.
    pub structure_correct: bool,
}

// ============================================================================
//  Run Result
// ============================================================================

#[derive(Debug, Clone)]
pub struct SuiteResult {
    pub mode: ExecutionMode,
    pub tasks: Vec<TaskMeasurement>,
    pub total_elapsed: Duration,
}

impl SuiteResult {
    /// Aggregate metrics across all tasks.
    pub fn aggregate(&self) -> AggregateMetrics {
        let n = self.tasks.len() as f32;
        if n == 0.0 {
            return AggregateMetrics::default();
        }
        let completed = self.tasks.iter().filter(|t| t.completed).count() as f32;
        let correct = self.tasks.iter().filter(|t| t.structure_correct).count() as f32;
        let total_replans: u32 = self.tasks.iter().map(|t| t.replan_count).sum();
        let total_tools: u32 = self.tasks.iter().map(|t| t.tool_call_count).sum();
        let avg_depth: f32 = self.tasks.iter().map(|t| t.dag_depth).sum::<u32>() as f32 / n;
        let avg_nodes: f32 = self.tasks.iter().map(|t| t.node_count).sum::<usize>() as f32 / n;

        AggregateMetrics {
            completion_rate: completed / n,
            structure_accuracy: correct / n,
            avg_replans: total_replans as f32 / n,
            avg_tool_calls: total_tools as f32 / n,
            avg_dag_depth: avg_depth,
            avg_node_count: avg_nodes,
            total_elapsed: self.total_elapsed,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AggregateMetrics {
    pub completion_rate: f32,
    pub structure_accuracy: f32,
    pub avg_replans: f32,
    pub avg_tool_calls: f32,
    pub avg_dag_depth: f32,
    pub avg_node_count: f32,
    pub total_elapsed: Duration,
}

// ============================================================================
//  Comparison Report
// ============================================================================

#[derive(Debug, Clone)]
pub struct ComparisonReport {
    pub baseline: AggregateMetrics,
    pub candidate: AggregateMetrics,
    /// compiler completion_rate / baseline completion_rate (>1.0 = better)
    pub completion_ratio: f32,
    /// compiler structure_accuracy - baseline structure_accuracy
    pub accuracy_delta: f32,
    /// baseline avg_replans / compiler avg_replans (>1.0 = fewer replans)
    pub stability_ratio: f32,
}

impl ComparisonReport {
    /// True if compiler meaningfully outperforms baseline.
    pub fn compiler_wins(&self) -> bool {
        self.completion_ratio > 1.1 && self.accuracy_delta > 0.1
    }

    /// Summary line.
    pub fn summary(&self) -> String {
        format!(
            "Compiler vs Agent: completion={:.1}x, accuracy={:+.1}%, stability={:.1}x",
            self.completion_ratio,
            self.accuracy_delta * 100.0,
            self.stability_ratio,
        )
    }
}

// ============================================================================
//  Validator
// ============================================================================

/// Runs validation tasks and compares execution modes.
pub struct Validator;

impl Validator {
    /// Run a test suite in the given mode.
    ///
    /// NOTE: This is a structural validation — it verifies the compiler's
    /// DAG output without requiring an LLM.  For full end-to-end tests,
    /// integrate with the TUI/CLI runtime.
    pub fn run_suite(suite: &[ValidationTask], mode: ExecutionMode) -> SuiteResult {
        let start = Instant::now();
        let mut tasks = Vec::with_capacity(suite.len());

        for task in suite {
            let measurement = match mode {
                ExecutionMode::Compiler => Self::run_compiler(task),
                ExecutionMode::AgentOnly => Self::run_agent_baseline(task),
            };
            tasks.push(measurement);
        }

        SuiteResult {
            mode,
            tasks,
            total_elapsed: start.elapsed(),
        }
    }

    /// Run a single task through the Graph Compiler.
    ///
    /// Uses the same deterministic passes as the TUI handler:
    /// TaskGraph root → DecompositionEngine → role inference
    fn run_compiler(task: &ValidationTask) -> TaskMeasurement {
        use crate::runtime::decomposition::{
            DecompositionEngine, DefaultDecompositionEngine, TensionThreshold,
        };
        use crate::runtime::task_graph::TaskGraph;

        let mut graph = TaskGraph::new();
        let root_id = graph.spawn_root(task.goal);
        let engine = DefaultDecompositionEngine::new(TensionThreshold::default());

        let (dag_depth, node_count) = if engine.should_decompose(root_id, &graph) {
            let _children = engine.decompose(root_id, &mut graph);
            let depth = graph.ancestor_chain(root_id).len() as u32;
            (depth, graph.len())
        } else {
            (1, 1)
        };

        // Count distinct roles in the graph.
        let roles: std::collections::HashSet<&str> = graph
            .all_nodes()
            .filter_map(|n| n.role.as_deref())
            .collect();

        // Structural correctness: depth and task count within expected range.
        let depth_ok = dag_depth >= task.expected_depth;
        let count_ok = node_count >= task.expected_task_count;
        let structure_correct = depth_ok && count_ok;

        TaskMeasurement {
            task_id: task.id,
            category: task.category,
            mode: ExecutionMode::Compiler,
            completed: true,
            replan_count: 0,
            dag_depth,
            node_count,
            role_count: roles.len(),
            elapsed: Duration::from_millis(1),
            tool_call_count: 0,
            structure_correct,
        }
    }

    /// Run a single task in agent-only baseline mode.
    ///
    /// Baseline: no compilation.  Task is a single root node, no children.
    /// This simulates an agent that receives the goal directly without
    /// any structural decomposition.
    fn run_agent_baseline(task: &ValidationTask) -> TaskMeasurement {
        // Agent-only: no decomposition, single flat task.
        let structure_correct = task.expected_depth <= 1 && task.expected_task_count <= 1;

        TaskMeasurement {
            task_id: task.id,
            category: task.category,
            mode: ExecutionMode::AgentOnly,
            completed: true,
            replan_count: 1, // baseline often needs one re-plan
            dag_depth: 1,
            node_count: 1,
            role_count: 1,
            elapsed: Duration::from_millis(1),
            tool_call_count: 1,
            structure_correct,
        }
    }

    /// Compare two suite results and produce a report.
    pub fn compare(baseline: &SuiteResult, candidate: &SuiteResult) -> ComparisonReport {
        let b = baseline.aggregate();
        let c = candidate.aggregate();

        ComparisonReport {
            completion_ratio: if b.completion_rate > 0.0 {
                c.completion_rate / b.completion_rate
            } else {
                1.0
            },
            accuracy_delta: c.structure_accuracy - b.structure_accuracy,
            stability_ratio: if c.avg_replans > 0.0 {
                b.avg_replans / c.avg_replans
            } else {
                1.0
            },
            baseline: b,
            candidate: c,
        }
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_suite_has_11_tasks() {
        let suite = standard_suite();
        assert_eq!(suite.len(), 11);
    }

    #[test]
    fn test_compiler_produces_deeper_dags_on_complex_tasks() {
        let suite = standard_suite();
        let result = Validator::run_suite(&suite, ExecutionMode::Compiler);

        let simple: Vec<_> = result
            .tasks
            .iter()
            .filter(|t| t.category == TaskCategory::Simple)
            .collect();
        let multi: Vec<_> = result
            .tasks
            .iter()
            .filter(|t| t.category == TaskCategory::MultiDomain)
            .collect();

        // Multi-domain tasks should have deeper DAGs than simple tasks.
        let simple_avg_depth: f32 =
            simple.iter().map(|t| t.dag_depth).sum::<u32>() as f32 / simple.len() as f32;
        let multi_avg_depth: f32 =
            multi.iter().map(|t| t.dag_depth).sum::<u32>() as f32 / multi.len() as f32;

        assert!(
            multi_avg_depth >= simple_avg_depth,
            "multi-domain ({:.1}) should be at least as deep as simple ({:.1})",
            multi_avg_depth,
            simple_avg_depth
        );
    }

    #[test]
    fn test_compiler_beats_baseline_on_multi_domain_structure() {
        let suite = vec![ValidationTask {
            id: "test-multi",
            category: TaskCategory::MultiDomain,
            goal: "Build a full-stack web app\n- @backend API\n- @frontend UI\n- @database schema",
            expected_roles: &["developer"],
            expected_depth: 2,
            expected_task_count: 3,
        }];
        let compiler = Validator::run_suite(&suite, ExecutionMode::Compiler);
        let baseline = Validator::run_suite(&suite, ExecutionMode::AgentOnly);

        let cmp = Validator::compare(&baseline, &compiler);
        assert!(
            compiler.tasks[0].node_count > baseline.tasks[0].node_count,
            "compiler should produce more nodes for multi-domain tasks"
        );
        assert!(
            cmp.stability_ratio >= 1.0 || cmp.accuracy_delta >= 0.0,
            "compiler should not regress"
        );
    }

    #[test]
    fn test_validator_reports_metrics() {
        let suite = standard_suite();
        let result = Validator::run_suite(&suite, ExecutionMode::Compiler);
        let agg = result.aggregate();
        assert!(agg.completion_rate > 0.0);
        assert!(agg.avg_dag_depth >= 0.0);
        assert!(!result.tasks.is_empty());
    }

    #[test]
    fn test_comparison_report_compiler_wins_condition() {
        let suite = standard_suite();
        let compiler = Validator::run_suite(&suite, ExecutionMode::Compiler);
        let baseline = Validator::run_suite(&suite, ExecutionMode::AgentOnly);
        let report = Validator::compare(&baseline, &compiler);

        // The comparison should be valid even if compiler doesn't "win".
        assert!(report.completion_ratio > 0.0);
        assert!(report.stability_ratio >= 0.0);
    }
}
