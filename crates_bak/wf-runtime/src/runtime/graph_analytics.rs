//! Graph Analytics — observability + pattern discovery + template evolution.
//!
//! Phase 4A: GraphMetrics + WorkflowSignature (observability)
//! Phase 4B: PatternDiscovery (pattern extraction)
//! Phase 4C: WorkflowTemplate + TemplateRegistry + TemplateMatcher + Instantiation
//! Phase 4D: TemplateEvolution (evaluate + compete + retire)

use std::collections::HashMap;
use wf_core::TaskId;
use wf_core::task_graph::{TaskGraph, TaskStatus};

// ============================================================================
//  GraphMetrics
// ============================================================================

#[derive(Debug, Clone)]
pub struct GraphMetrics {
    pub root_count: usize,
    pub node_count: usize,
    pub avg_depth: f32,
    pub max_depth: usize,
    pub avg_width: f32,
    pub max_width: usize,
    pub leaf_count: usize,
    pub status_counts: HashMap<TaskStatus, usize>,
}

impl GraphMetrics {
    pub fn from_graph(graph: &TaskGraph) -> Self {
        let roots = graph.roots();
        let nodes: Vec<_> = graph.all_nodes().collect();
        let mut total_depth = 0usize;
        let mut max_depth = 0usize;
        let mut depth_samples = 0usize;
        let mut total_width = 0.0f32;
        let mut max_width = 0usize;
        let mut width_samples = 0usize;
        for node in &nodes {
            let depth = graph.ancestor_chain(node.id).len().saturating_sub(1);
            total_depth += depth;
            max_depth = max_depth.max(depth);
            depth_samples += 1;
            if !node.children.is_empty() {
                total_width += node.children.len() as f32;
                max_width = max_width.max(node.children.len());
                width_samples += 1;
            }
        }
        Self {
            root_count: roots.len(),
            node_count: nodes.len(),
            avg_depth: if depth_samples > 0 {
                total_depth as f32 / depth_samples as f32
            } else {
                0.0
            },
            max_depth,
            avg_width: if width_samples > 0 {
                total_width / width_samples as f32
            } else {
                0.0
            },
            max_width,
            leaf_count: nodes.iter().filter(|n| n.children.is_empty()).count(),
            status_counts: graph.status_counts(None),
        }
    }
}

// ============================================================================
//  WorkflowSignature + DependencyShape
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DependencyShape {
    Chain,
    FanOut,
    FanIn,
    Dag,
    Atomic,
}

#[derive(Debug, Clone)]
pub struct WorkflowSignature {
    pub shape: DependencyShape,
    pub role_count: usize,
    pub roles_used: Vec<String>,
    pub depth: usize,
    pub critical_path_length: usize,
}

impl WorkflowSignature {
    pub fn from_graph(graph: &TaskGraph) -> Self {
        let nodes: Vec<_> = graph.all_nodes().collect();
        let non_root: Vec<_> = nodes.iter().filter(|n| n.parent.is_some()).collect();
        let mut roles_map: HashMap<String, usize> = HashMap::new();
        for node in &nodes {
            if let Some(ref r) = node.role {
                *roles_map.entry(r.clone()).or_insert(0) += 1;
            }
        }
        let mut roles_sorted: Vec<_> = roles_map.into_iter().collect();
        roles_sorted.sort_by(|a, b| b.1.cmp(&a.1));
        let shape = if nodes.len() <= 1 {
            DependencyShape::Atomic
        } else if non_root.iter().all(|n| n.dependencies.is_empty()) {
            DependencyShape::FanOut
        } else if non_root.iter().any(|n| n.dependencies.len() >= 2) {
            DependencyShape::FanIn
        } else if non_root.len() == nodes.len().saturating_sub(1) {
            DependencyShape::Chain
        } else {
            DependencyShape::Dag
        };
        let depth = nodes
            .iter()
            .map(|n| graph.ancestor_chain(n.id).len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        let cp = graph
            .topological_sort()
            .map(|sorted| {
                let mut dist: HashMap<TaskId, usize> = HashMap::new();
                let mut max_d = 0usize;
                for id in &sorted {
                    let d = dist.get(id).copied().unwrap_or(0);
                    max_d = max_d.max(d);
                    if let Some(node) = graph.get(id) {
                        for c in &node.children {
                            *dist.entry(*c).or_insert(0) = (*dist.get(c).unwrap_or(&0)).max(d + 1);
                        }
                        for d2 in &node.dependencies {
                            *dist.entry(*d2).or_insert(0) =
                                (*dist.get(d2).unwrap_or(&0)).max(d + 1);
                        }
                    }
                }
                max_d
            })
            .unwrap_or(0);
        WorkflowSignature {
            shape,
            role_count: roles_sorted.len(),
            roles_used: roles_sorted.into_iter().map(|(r, _)| r).collect(),
            depth,
            critical_path_length: cp,
        }
    }
}

// ============================================================================
//  DiscoveredPattern + PatternDiscovery (Phase 4B)
// ============================================================================

#[derive(Debug, Clone)]
pub struct DiscoveredPattern {
    pub goal_signature: Vec<String>,
    pub shape: DependencyShape,
    pub roles_used: Vec<String>,
    pub success_rate: f32,
    pub sample_count: usize,
    pub avg_completion_time_ms: f64,
    pub is_template: bool,
}

impl DiscoveredPattern {
    pub fn ready_for_promotion(&self) -> bool {
        self.sample_count >= PROMOTION_THRESHOLD && self.success_rate >= SUCCESS_THRESHOLD
    }
}

pub const PROMOTION_THRESHOLD: usize = 5;
pub const SUCCESS_THRESHOLD: f32 = 0.7;

pub struct PatternDiscovery;

impl PatternDiscovery {
    pub fn discover(executions: &[ExecutionRecord]) -> Vec<DiscoveredPattern> {
        if executions.is_empty() {
            return Vec::new();
        }
        let mut clusters: Vec<Vec<&ExecutionRecord>> = Vec::new();
        let mut assigned = vec![false; executions.len()];
        for i in 0..executions.len() {
            if assigned[i] {
                continue;
            }
            let mut cluster = vec![&executions[i]];
            assigned[i] = true;
            for j in (i + 1)..executions.len() {
                if assigned[j] {
                    continue;
                }
                if Self::goal_similarity(&executions[i].goal, &executions[j].goal) > 0.3 {
                    cluster.push(&executions[j]);
                    assigned[j] = true;
                }
            }
            if cluster.len() >= 2 {
                clusters.push(cluster);
            }
        }
        let mut patterns: Vec<DiscoveredPattern> = clusters
            .iter()
            .map(|c| Self::pattern_from_cluster(c))
            .collect();
        patterns.sort_by(|a, b| b.sample_count.cmp(&a.sample_count));
        patterns
    }

    fn goal_similarity(a: &str, b: &str) -> f32 {
        let a_lower = a.to_lowercase();
        let b_lower = b.to_lowercase();
        let words_a: std::collections::HashSet<&str> = a_lower.split_whitespace().collect();
        let words_b: std::collections::HashSet<&str> = b_lower.split_whitespace().collect();
        let intersection = words_a.intersection(&words_b).count();
        let union = words_a.union(&words_b).count();
        if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        }
    }

    fn pattern_from_cluster(cluster: &[&ExecutionRecord]) -> DiscoveredPattern {
        let mut kw_freq: HashMap<String, usize> = HashMap::new();
        let mut shape_counts: HashMap<DependencyShape, usize> = HashMap::new();
        let mut total_success = 0usize;
        let mut total_lat = 0.0f64;
        for e in cluster {
            for w in e.goal.to_lowercase().split_whitespace() {
                if w.len() > 2 {
                    *kw_freq.entry(w.to_string()).or_insert(0) += 1;
                }
            }
            *shape_counts.entry(e.signature.shape.clone()).or_insert(0) += 1;
            if e.success {
                total_success += 1;
            }
            total_lat += e.latency_ms;
        }
        let threshold = (cluster.len() / 2).max(1);
        let mut gs: Vec<String> = kw_freq
            .into_iter()
            .filter(|(_, c)| *c >= threshold)
            .map(|(w, _)| w)
            .collect();
        gs.sort();
        let dom_shape = shape_counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(s, _)| s)
            .unwrap_or(DependencyShape::Atomic);
        let mut roles = std::collections::BTreeSet::new();
        for e in cluster {
            for r in &e.signature.roles_used {
                roles.insert(r.clone());
            }
        }
        DiscoveredPattern {
            goal_signature: gs,
            shape: dom_shape,
            roles_used: roles.into_iter().collect(),
            success_rate: total_success as f32 / cluster.len() as f32,
            sample_count: cluster.len(),
            avg_completion_time_ms: if cluster.is_empty() {
                0.0
            } else {
                total_lat / cluster.len() as f64
            },
            is_template: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub goal: String,
    pub signature: WorkflowSignature,
    pub success: bool,
    pub latency_ms: f64,
}

// ============================================================================
//  Phase 4C — WorkflowTemplate + TemplateRegistry + Matcher + Instantiation
// ============================================================================

pub type TemplateId = usize;

#[derive(Debug, Clone)]
pub struct WorkflowTemplate {
    pub id: TemplateId,
    pub name: String,
    pub signature: WorkflowSignature,
    pub roles: Vec<TemplateRole>,
    pub dependency_edges: Vec<(usize, usize)>,
    pub success_rate: f32,
    pub sample_count: u32,
    pub goal_keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TemplateRole {
    pub role_name: String,
    pub goal_fragment: String,
}

pub struct TemplateRegistry {
    templates: Vec<WorkflowTemplate>,
    next_id: TemplateId,
}

impl TemplateRegistry {
    pub fn new() -> Self {
        Self {
            templates: Vec::new(),
            next_id: 0,
        }
    }
}

impl Default for TemplateRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TemplateRegistry {
    pub fn insert(&mut self, mut t: WorkflowTemplate) -> TemplateId {
        let id = self.next_id;
        self.next_id += 1;
        t.id = id;
        self.templates.push(t);
        id
    }
    pub fn get(&self, id: TemplateId) -> Option<&WorkflowTemplate> {
        self.templates.iter().find(|t| t.id == id)
    }
    pub fn all(&self) -> &[WorkflowTemplate] {
        &self.templates
    }
    pub fn promote(&mut self, pattern: &DiscoveredPattern) -> Option<TemplateId> {
        if !pattern.ready_for_promotion() {
            return None;
        }
        let roles: Vec<TemplateRole> = pattern
            .roles_used
            .iter()
            .map(|r| TemplateRole {
                role_name: r.clone(),
                goal_fragment: r.clone(),
            })
            .collect();
        let deps: Vec<(usize, usize)> = (1..roles.len()).map(|i| (i - 1, i)).collect();
        let name = pattern
            .goal_signature
            .first()
            .cloned()
            .unwrap_or_else(|| "pattern".to_string());
        let id = self.next_id;
        self.next_id += 1;
        self.templates.push(WorkflowTemplate {
            id,
            name: format!("tpl-{}-{}", name, id),
            signature: WorkflowSignature {
                shape: pattern.shape.clone(),
                role_count: roles.len(),
                roles_used: pattern.roles_used.clone(),
                depth: 2,
                critical_path_length: roles.len(),
            },
            roles,
            dependency_edges: deps,
            success_rate: pattern.success_rate,
            sample_count: pattern.sample_count as u32,
            goal_keywords: pattern.goal_signature.clone(),
        });
        Some(id)
    }
}

pub struct TemplateMatcher {
    threshold: f32,
}
impl Default for TemplateMatcher {
    fn default() -> Self {
        Self { threshold: 0.3 }
    }
}
impl TemplateMatcher {
    pub fn match_goal<'a>(
        &self,
        goal: &str,
        templates: &'a [WorkflowTemplate],
    ) -> Option<&'a WorkflowTemplate> {
        let g = goal.to_lowercase();
        let gw: std::collections::HashSet<&str> = g.split_whitespace().collect();
        let (mut best_score, mut best_idx) = (0.0f32, None);
        for (i, t) in templates.iter().enumerate() {
            let tw: std::collections::HashSet<&str> =
                t.goal_keywords.iter().map(|s| s.as_str()).collect();
            let inter = gw.intersection(&tw).count();
            let union = gw.union(&tw).count();
            let score = if union == 0 {
                0.0
            } else {
                inter as f32 / union as f32
            };
            if score > best_score {
                best_score = score;
                best_idx = Some(i);
            }
        }
        if best_score >= self.threshold {
            best_idx.map(|i| &templates[i])
        } else {
            None
        }
    }
}

pub struct TemplateInstantiation;
impl TemplateInstantiation {
    pub fn instantiate(
        template: &WorkflowTemplate,
        parent_id: TaskId,
        graph: &mut TaskGraph,
    ) -> Vec<TaskId> {
        let mut child_ids = Vec::new();
        for r in &template.roles {
            let g = if r.goal_fragment.is_empty() {
                format!("{} task from template", r.role_name)
            } else {
                r.goal_fragment.clone()
            };
            if let Some(cid) = graph.spawn_child(parent_id, &g) {
                if let Some(node) = graph.get_mut(&cid) {
                    node.role = Some(r.role_name.clone());
                }
                child_ids.push(cid);
            }
        }
        for (prereq, dep) in &template.dependency_edges {
            if *prereq < child_ids.len() && *dep < child_ids.len() {
                let _ = graph.add_dependency(child_ids[*dep], child_ids[*prereq]);
            }
        }
        graph.mark_decomposed(parent_id).ok();
        child_ids
    }
}

// ============================================================================
//  Phase 4D — Template Evolution
// ============================================================================

#[derive(Debug, Clone)]
pub struct TemplateScore {
    pub template_id: TemplateId,
    pub total: f32,
    pub goal_similarity: f32,
    pub success_rate: f32,
    pub recency: f32,
    pub sample_reliability: f32,
}

pub trait TemplateEvaluator: Send + Sync {
    fn evaluate(&self, goal: &str, template: &WorkflowTemplate) -> TemplateScore;
}

pub struct DefaultTemplateEvaluator;
impl TemplateEvaluator for DefaultTemplateEvaluator {
    fn evaluate(&self, goal: &str, template: &WorkflowTemplate) -> TemplateScore {
        let g = goal.to_lowercase();
        let gw: std::collections::HashSet<&str> = g.split_whitespace().collect();
        let tw: std::collections::HashSet<&str> =
            template.goal_keywords.iter().map(|s| s.as_str()).collect();
        let inter = gw.intersection(&tw).count();
        let union = gw.union(&tw).count();
        let goal_sim = if union == 0 {
            0.0
        } else {
            inter as f32 / union as f32
        };
        let sample_rel = template.sample_count as f32 / (template.sample_count as f32 + 10.0);
        let recency = (template.id as f32 / 100.0).min(1.0);
        let total =
            0.40 * goal_sim + 0.30 * template.success_rate + 0.20 * sample_rel + 0.10 * recency;
        TemplateScore {
            template_id: template.id,
            total,
            goal_similarity: goal_sim,
            success_rate: template.success_rate,
            recency,
            sample_reliability: sample_rel,
        }
    }
}

pub struct TemplateEvolution {
    pub min_success_rate: f32,
    pub min_samples_for_stability: u32,
}
impl Default for TemplateEvolution {
    fn default() -> Self {
        Self {
            min_success_rate: 0.3,
            min_samples_for_stability: 10,
        }
    }
}
impl TemplateEvolution {
    pub fn record_outcome(&self, template: &mut WorkflowTemplate, success: bool) {
        let total = template.sample_count + 1;
        template.success_rate = ((template.success_rate * template.sample_count as f32)
            + if success { 1.0 } else { 0.0 })
            / total as f32;
        template.sample_count = total;
    }
    pub fn should_retire(&self, template: &WorkflowTemplate) -> bool {
        template.sample_count >= self.min_samples_for_stability
            && template.success_rate < self.min_success_rate
    }
    pub fn select_best<'a>(
        &self,
        goal: &str,
        templates: &'a [WorkflowTemplate],
        evaluator: &dyn TemplateEvaluator,
        min_score: f32,
    ) -> Option<&'a WorkflowTemplate> {
        let mut best: Option<(&WorkflowTemplate, f32)> = None;
        for t in templates {
            let s = evaluator.evaluate(goal, t);
            if s.total >= min_score {
                match best {
                    Some((_, bs)) if s.total > bs => best = Some((t, s.total)),
                    None => best = Some((t, s.total)),
                    _ => {}
                }
            }
        }
        best.map(|(t, _)| t)
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    fn make_graph() -> TaskGraph {
        let mut g = TaskGraph::new();
        let root = g.spawn_root("goal");
        g.spawn_child(root, "sub");
        g
    }

    #[test]
    fn test_graph_metrics_basic() {
        let g = make_graph();
        let m = GraphMetrics::from_graph(&g);
        assert_eq!(m.node_count, 2);
    }

    #[test]
    fn test_registry_empty() {
        let r = TemplateRegistry::new();
        assert!(r.all().is_empty());
    }

    #[test]
    fn test_template_evaluator_scores() {
        let e = DefaultTemplateEvaluator;
        let t = WorkflowTemplate {
            id: 1,
            name: "t".into(),
            signature: WorkflowSignature {
                shape: DependencyShape::FanOut,
                role_count: 1,
                roles_used: vec!["dev".into()],
                depth: 1,
                critical_path_length: 1,
            },
            roles: vec![TemplateRole {
                role_name: "dev".into(),
                goal_fragment: "build".into(),
            }],
            dependency_edges: vec![],
            success_rate: 0.8,
            sample_count: 20,
            goal_keywords: vec!["build".into(), "api".into()],
        };
        let s = e.evaluate("build api", &t);
        assert!(s.total > 0.5, "total={:.2}", s.total);
    }

    #[test]
    fn test_pattern_discovery() {
        let rec = ExecutionRecord {
            goal: "build api".into(),
            signature: WorkflowSignature {
                shape: DependencyShape::FanOut,
                role_count: 2,
                roles_used: vec!["dev".into(), "test".into()],
                depth: 2,
                critical_path_length: 2,
            },
            success: true,
            latency_ms: 100.0,
        };
        let patterns = PatternDiscovery::discover(&[rec]);
        assert!(
            patterns.is_empty(),
            "single execution should not form a pattern"
        );
    }

    #[test]
    fn test_template_matcher_basic() {
        let mut reg = TemplateRegistry::new();
        let pat = DiscoveredPattern {
            goal_signature: vec!["build".into(), "api".into()],
            shape: DependencyShape::FanOut,
            roles_used: vec!["dev".into()],
            success_rate: 0.85,
            sample_count: 6,
            avg_completion_time_ms: 1000.0,
            is_template: false,
        };
        reg.promote(&pat);
        let matcher = TemplateMatcher::default();
        assert!(matcher.match_goal("build rest api", reg.all()).is_some());
        assert!(matcher.match_goal("deploy infra", reg.all()).is_none());
    }
}
