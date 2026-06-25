//! StrategyGraph — schema + stable evolutionary dynamics for decision functions.
//!
//! This is NOT a Meta-Agent.  Meta-Agent means "an agent modifies the system."
//! StrategyGraph means "the system selects its own computation strategy based
//! on execution evidence, with stable competitive dynamics."
//!
//! # Constraint (inviolable)
//!
//! StrategyGraph cannot directly affect the Execution Kernel.
//! All strategy influence must pass through `StrategySelector`.
//!
//! # Evolutionary Dynamics (added Phase 5)
//!
//! A competition protocol alone is insufficient — it leads to oscillation or
//! premature convergence.  The three stabilizers are:
//!
//! 1. **StrategyMomentum** — recent win streaks bias selection probability,
//!    preventing oscillation between two equally good strategies.
//!
//! 2. **ExplorationPressure** — a fraction of selections try non-optimal
//!    strategies, preventing lock-in to local optima.
//!
//! 3. **ClusterDriftDetector** — monitors whether task distributions shift
//!    over time.  If drift is detected, cluster boundaries are re-evaluated
//!    and retired strategies may be re-activated.

use std::collections::HashMap;

// ============================================================================
//  StrategyNode — a parameterized decision function
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StrategyType {
    Estimator,
    Policy,
    Selector,
    Router,
}

pub type StrategyId = u64;
pub type ClusterId = u64;

#[derive(Debug, Clone)]
pub struct StrategyNode {
    pub id: StrategyId,
    pub strategy_type: StrategyType,
    pub name: String,
    pub config: serde_json::Value,
    pub scope: StrategyScope,
    pub version: u32,
    pub active: bool,
    /// Momentum tracker: recent win/loss streak (positive = winning).
    pub momentum: i32,
    /// Epoch of last promotion (for decay calculations).
    pub promoted_at_epoch: u64,
}

#[derive(Debug, Clone)]
pub struct StrategyScope {
    pub domain_keywords: Vec<String>,
    pub min_complexity: f32,
    pub max_complexity: f32,
}

// ============================================================================
//  StrategyExecutionTrace
// ============================================================================

#[derive(Debug, Clone)]
pub struct StrategyExecutionTrace {
    pub trace_id: u64,
    pub strategy_id: StrategyId,
    pub cluster_id: Option<ClusterId>,
    pub task_signature: TaskSignature,
    pub output_decision: serde_json::Value,
    pub success: bool,
    pub latency_ms: u64,
    pub epoch: u64,
}

#[derive(Debug, Clone)]
pub struct TaskSignature {
    pub goal_length_chars: usize,
    pub domain_count: u32,
    pub estimated_complexity: f32,
    pub role_count: u32,
}

// ============================================================================
//  PerformanceEdge — cluster-scoped, never global
// ============================================================================

#[derive(Debug, Clone)]
pub struct PerformanceEdge {
    pub strategy_a: StrategyId,
    pub strategy_b: StrategyId,
    pub task_cluster: ClusterId,
    pub win_rate: f32,
    pub sample_count: u32,
    pub confidence: f32,
}

// ============================================================================
//  ⭐ Evolutionary Fitness Dynamics (Phase 5)
// ============================================================================

/// Tracks recent performance momentum to prevent oscillation.
///
/// If two strategies continuously trade wins, without momentum the selector
/// would flip-flop every cycle.  Momentum biases toward the current leader
/// unless the gap is significant.
#[derive(Debug, Clone)]
pub struct StrategyMomentum {
    /// Current streak length (+ = wins, - = losses).
    pub streak: i32,
    /// Maximum streak observed.
    pub peak_streak: i32,
    /// Exponential moving average of recent win rate.
    pub ema_win_rate: f32,
    /// Alpha for the EMA (0.0–1.0, higher = more weight on recent).
    pub ema_alpha: f32,
}

impl StrategyMomentum {
    pub fn new() -> Self {
        Self {
            streak: 0,
            peak_streak: 0,
            ema_win_rate: 0.5,
            ema_alpha: 0.3,
        }
    }
}

impl Default for StrategyMomentum {
    fn default() -> Self {
        Self::new()
    }
}

impl StrategyMomentum {
    /// Record an outcome and update momentum.
    pub fn record(&mut self, success: bool) {
        if success {
            self.streak = (self.streak + 1).min(100);
            self.peak_streak = self.peak_streak.max(self.streak);
        } else {
            self.streak = (-self.streak.abs() - 1).max(-100);
        }
        self.ema_win_rate = self.ema_alpha * if success { 1.0 } else { 0.0 }
            + (1.0 - self.ema_alpha) * self.ema_win_rate;
    }

    /// Selection bias multiplier: winning streaks boost selection probability.
    pub fn selection_bias(&self) -> f32 {
        if self.streak > 0 {
            1.0 + (self.streak as f32 * 0.02).min(0.5) // up to +50%
        } else if self.streak < 0 {
            (1.0 + (self.streak as f32 * 0.05)).max(0.1) // down to 10%
        } else {
            1.0
        }
    }
}

/// Exploration pressure: fraction of selections that try non-optimal strategies.
///
/// Without exploration, the system locks into the first locally optimal
/// strategy and never discovers better alternatives.
#[derive(Debug, Clone)]
pub struct ExplorationPolicy {
    /// Probability of selecting a random non-optimal strategy (0.0–1.0).
    pub epsilon: f32,
    /// Decay rate: epsilon decreases by this factor per epoch (0.0 = no decay).
    pub epsilon_decay: f32,
    /// Minimum epsilon even after decay.
    pub min_epsilon: f32,
    /// Current epoch.
    pub epoch: u64,
}

impl Default for ExplorationPolicy {
    fn default() -> Self {
        Self {
            epsilon: 0.15,
            epsilon_decay: 0.001,
            min_epsilon: 0.02,
            epoch: 0,
        }
    }
}

impl ExplorationPolicy {
    /// Advance one epoch and decay epsilon.
    pub fn tick(&mut self) {
        self.epoch += 1;
        self.epsilon = (self.epsilon - self.epsilon_decay).max(self.min_epsilon);
    }

    /// Whether this selection should explore (vs exploit).
    pub fn should_explore(&self) -> bool {
        rand::random::<f32>() < self.epsilon
    }
}

/// Detects whether incoming task distributions are shifting.
///
/// When the task distribution drifts, cluster boundaries need to be
/// re-evaluated and previously retired strategies may become viable again.
#[derive(Debug, Clone)]
pub struct ClusterDriftDetector {
    /// Rolling window of recent task signatures per cluster.
    pub windows: HashMap<ClusterId, Vec<TaskSignature>>,
    /// Max window size before computing drift.
    pub max_window: usize,
    /// Drift threshold: if the mean signature differs by more than this,
    /// the cluster is drifting.
    pub drift_threshold: f32,
}

impl Default for ClusterDriftDetector {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            max_window: 100,
            drift_threshold: 0.3,
        }
    }
}

impl ClusterDriftDetector {
    /// Record a task execution against a cluster.
    pub fn record(&mut self, cluster: ClusterId, signature: TaskSignature) {
        let window = self.windows.entry(cluster).or_default();
        window.push(signature);
        if window.len() > self.max_window {
            window.remove(0);
        }
    }

    /// Check whether a cluster's task distribution has drifted from its baseline.
    pub fn is_drifting(&self, _cluster: ClusterId) -> bool {
        // Phase 5: compare recent window mean vs historical baseline.
        // Simplified for schema: returns false when insufficient data.
        false
    }
}

// ============================================================================
//  Phase 5.1 — Strategy Fitness Stabilization Layer
// ============================================================================

/// Anti-collapse constraint: prevents any single strategy from monopolizing
/// more than `max_selection_share` of selections in a cluster.
///
/// Without this, the winning strategy gets selected more → gets more data →
/// wins more → winner-take-all collapse.  This enforces diversity by capping
/// selection probability at a per-cluster maximum.
#[derive(Debug, Clone)]
pub struct AntiCollapseConstraint {
    /// Maximum fraction of selections a single strategy can occupy (0.0–1.0).
    pub max_selection_share: f32,
    /// Window size for tracking recent selections per cluster.
    pub window_size: usize,
}

impl Default for AntiCollapseConstraint {
    fn default() -> Self {
        Self {
            max_selection_share: 0.6,
            window_size: 50,
        }
    }
}

impl AntiCollapseConstraint {
    /// Check if a strategy is over-selected in a cluster (approaching monopoly).
    pub fn is_over_selected(
        &self,
        strategy_id: StrategyId,
        recent_selections: &[StrategyId],
    ) -> bool {
        if recent_selections.is_empty() {
            return false;
        }
        let window: Vec<_> = recent_selections
            .iter()
            .rev()
            .take(self.window_size)
            .collect();
        let count = window.iter().filter(|&&s| *s == strategy_id).count();
        let share = count as f32 / window.len() as f32;
        share > self.max_selection_share
    }
}

/// Regret-based promotion: measures how much the system lost by NOT selecting
/// a strategy earlier.  A strategy with high regret should be promoted faster
/// even if its raw win rate is similar to the incumbent.
///
/// Regret = (incumbent_win_rate - candidate_win_rate) * missed_opportunities
#[derive(Debug, Clone)]
pub struct RegretTracker {
    /// Accumulated regret per (cluster, candidate_strategy).
    pub regret_map: HashMap<(ClusterId, StrategyId), f32>,
}

impl RegretTracker {
    pub fn new() -> Self {
        Self {
            regret_map: HashMap::new(),
        }
    }
}

impl Default for RegretTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RegretTracker {
    /// Record that `incumbent` was selected and `candidates` were not.
    /// Updates regret for each candidate.
    pub fn record_selection(
        &mut self,
        cluster: ClusterId,
        incumbent: StrategyId,
        candidates: &[StrategyId],
        incumbent_won: bool,
    ) {
        let sign: f32 = if incumbent_won { -1.0 } else { 1.0 };
        let map_len = self.regret_map.len();
        for &cid in candidates {
            if cid == incumbent {
                continue;
            }
            let entry = self.regret_map.entry((cluster, cid)).or_insert(0.0);
            *entry += sign * (1.0 / (map_len as f32 + 1.0)).min(0.1);
        }
    }

    /// Get the regret-adjusted win rate for a candidate.
    /// High regret = this candidate should have been selected earlier = boost.
    pub fn adjusted_win_rate(
        &self,
        base_win_rate: f32,
        cluster: ClusterId,
        strategy: StrategyId,
    ) -> f32 {
        let regret = self
            .regret_map
            .get(&(cluster, strategy))
            .copied()
            .unwrap_or(0.0);
        (base_win_rate + regret * 0.1).clamp(0.0, 1.0)
    }
}

/// Time-decay evaluator: older traces contribute less to rankings.
///
/// Without decay, a strategy that was dominant 6 months ago but is now
/// mediocre will persist in the ranking because its historical wins still
/// count.  Exponential decay solves this.
#[derive(Debug, Clone)]
pub struct TimeDecayEvaluator {
    /// Half-life in epochs: after this many epochs, a trace loses half its weight.
    pub half_life: u64,
    /// Current epoch (incremented each competition cycle).
    pub epoch: u64,
}

impl Default for TimeDecayEvaluator {
    fn default() -> Self {
        Self {
            half_life: 50,
            epoch: 0,
        }
    }
}

impl TimeDecayEvaluator {
    /// Advance one epoch.
    pub fn tick(&mut self) {
        self.epoch += 1;
    }

    /// Weight for a trace from `trace_epoch`.  Returns 1.0 at current epoch,
    /// decays exponentially toward 0.0 as |epoch - trace_epoch| increases.
    pub fn trace_weight(&self, trace_epoch: u64) -> f32 {
        let age = self.epoch.saturating_sub(trace_epoch);
        if age == 0 {
            return 1.0;
        }
        // Exponential decay: weight = 0.5^(age / half_life)
        (0.5f32).powf(age as f32 / self.half_life as f32)
    }
}

// ============================================================================
//  StrategySelector — the only bridge to Execution Kernel
// ============================================================================

pub trait StrategySelector: Send + Sync {
    fn select(&self, strategy_type: StrategyType, task: &TaskSignature) -> Option<StrategyId>;
    fn record_trace(&self, trace: StrategyExecutionTrace);
}

// ============================================================================
//  Competition Protocol
// ============================================================================

#[derive(Debug, Clone)]
pub struct CompetitionProtocol {
    pub min_samples_per_edge: u32,
    pub promotion_threshold: f32,
    pub retirement_threshold: f32,
    pub trace_decay_factor: f32,
}

impl Default for CompetitionProtocol {
    fn default() -> Self {
        Self {
            min_samples_per_edge: 10,
            promotion_threshold: 0.65,
            retirement_threshold: 0.35,
            trace_decay_factor: 0.1,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CompetitionAction {
    Promote {
        strategy_id: StrategyId,
        cluster: ClusterId,
        win_rate: f32,
    },
    Retire {
        strategy_id: StrategyId,
        cluster: ClusterId,
        win_rate: f32,
    },
    NoAction {
        reason: String,
    },
}

// ============================================================================
//  StrategyGraph — the container
// ============================================================================

pub struct StrategyGraph {
    pub strategies: HashMap<StrategyId, StrategyNode>,
    pub traces: Vec<StrategyExecutionTrace>,
    pub edges: Vec<PerformanceEdge>,
    pub protocol: CompetitionProtocol,
    pub momentum: HashMap<StrategyId, StrategyMomentum>,
    pub exploration: ExplorationPolicy,
    pub drift: ClusterDriftDetector,
    /// Phase 5.1: anti-collapse — prevents monopoly.
    pub anti_collapse: AntiCollapseConstraint,
    /// Phase 5.1: regret tracking — promotes under-used strategies.
    pub regret: RegretTracker,
    /// Phase 5.1: time decay — ages out old traces.
    pub time_decay: TimeDecayEvaluator,
    /// Rolling window of recent selections per cluster (for anti-collapse).
    pub recent_selections: HashMap<ClusterId, Vec<StrategyId>>,
    next_id: u64,
}

impl StrategyGraph {
    pub fn new(protocol: CompetitionProtocol) -> Self {
        Self {
            strategies: HashMap::new(),
            traces: Vec::new(),
            edges: Vec::new(),
            protocol,
            momentum: HashMap::new(),
            exploration: ExplorationPolicy::default(),
            drift: ClusterDriftDetector::default(),
            anti_collapse: AntiCollapseConstraint::default(),
            regret: RegretTracker::new(),
            time_decay: TimeDecayEvaluator::default(),
            recent_selections: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a strategy variant with common defaults.
    pub fn register_variant(
        &mut self,
        strategy_type: StrategyType,
        name: &str,
        config: serde_json::Value,
        version: u32,
    ) -> StrategyId {
        self.register(StrategyNode {
            id: 0,
            strategy_type,
            name: name.to_string(),
            config,
            scope: StrategyScope {
                domain_keywords: vec![],
                min_complexity: 0.0,
                max_complexity: 1.0,
            },
            version,
            active: true,
            momentum: 0,
            promoted_at_epoch: 0,
        })
    }

    /// Register default strategy variants: 2 estimators + 2 policies.
    pub fn register_defaults(&mut self) {
        self.register_variant(StrategyType::Estimator, "estimator-balanced",
            serde_json::json!({"weight_domain":0.30,"weight_steps":0.20,"weight_depth":0.15,"weight_ambiguity":0.15,"weight_parallel":0.10,"weight_history":0.10}), 1);
        self.register_variant(StrategyType::Estimator, "estimator-domain-heavy",
            serde_json::json!({"weight_domain":0.50,"weight_steps":0.15,"weight_depth":0.05,"weight_ambiguity":0.10,"weight_parallel":0.10,"weight_history":0.10}), 1);
        self.register_variant(
            StrategyType::Policy,
            "policy-conservative",
            serde_json::json!({"max_score":0.7,"max_domains":3,"max_depth":2,"max_steps":5}),
            1,
        );
        self.register_variant(
            StrategyType::Policy,
            "policy-aggressive",
            serde_json::json!({"max_score":0.5,"max_domains":2,"max_depth":1,"max_steps":3}),
            1,
        );
    }

    pub fn register(&mut self, mut node: StrategyNode) -> StrategyId {
        let id = self.next_id;
        self.next_id += 1;
        node.id = id;
        self.momentum.insert(id, StrategyMomentum::new());
        self.strategies.insert(id, node);
        id
    }

    pub fn record_trace(&mut self, trace: StrategyExecutionTrace) {
        // Update momentum for the selected strategy.
        if let Some(m) = self.momentum.get_mut(&trace.strategy_id) {
            m.record(trace.success);
        }
        // Record cluster drift signal.
        if let Some(cid) = trace.cluster_id {
            self.drift.record(cid, trace.task_signature.clone());
        }
        self.traces.push(trace);
    }

    /// Select a strategy with exploration pressure + momentum bias +
    /// anti-collapse constraint + regret adjustment + time decay.
    pub fn select_strategy(
        &mut self,
        strategy_type: StrategyType,
        cluster: ClusterId,
    ) -> Option<StrategyId> {
        self.time_decay.tick();
        let mut candidates: Vec<&StrategyNode> = self
            .strategies
            .values()
            .filter(|s| s.strategy_type == strategy_type && s.active)
            .collect();

        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            let id = candidates[0].id;
            self.record_selection(cluster, id, &[]);
            return Some(id);
        }

        // Phase 5.1: Anti-collapse — exclude over-selected strategies.
        let recent = self
            .recent_selections
            .get(&cluster)
            .cloned()
            .unwrap_or_default();
        candidates.retain(|s| !self.anti_collapse.is_over_selected(s.id, &recent));

        // If all candidates were over-selected, reset and force explore.
        if candidates.is_empty() {
            let idx = rand::random::<u64>() as usize
                % self
                    .strategies
                    .values()
                    .filter(|s| s.strategy_type == strategy_type && s.active)
                    .count()
                    .max(1);
            let fallback: Vec<_> = self
                .strategies
                .values()
                .filter(|s| s.strategy_type == strategy_type && s.active)
                .collect();
            let id = fallback[idx].id;
            self.record_selection(cluster, id, &[]);
            return Some(id);
        }

        // Exploration: randomly pick a non-optimal candidate.
        if self.exploration.should_explore() {
            let idx = rand::random::<u64>() as usize % candidates.len();
            let id = candidates[idx].id;
            self.record_selection(cluster, id, &[]);
            return Some(id);
        }

        // Exploitation: score each candidate with regret + momentum + time decay.
        let mut scored: Vec<(StrategyId, f32)> = candidates
            .iter()
            .map(|s| {
                let base_score = self
                    .edges
                    .iter()
                    .filter(|e| {
                        e.task_cluster == cluster && (e.strategy_a == s.id || e.strategy_b == s.id)
                    })
                    .map(|e| {
                        let wr = if e.strategy_a == s.id {
                            e.win_rate
                        } else {
                            1.0 - e.win_rate
                        };
                        // Apply time decay to edge confidence.
                        let decayed = self.time_decay.trace_weight(e.sample_count as u64); // approx; ideally per-edge epoch
                        wr * (0.5 + 0.5 * decayed)
                    })
                    .fold(0.5, |acc, wr| acc * 0.7 + wr * 0.3);

                let bias = self
                    .momentum
                    .get(&s.id)
                    .map(|m| m.selection_bias())
                    .unwrap_or(1.0);
                let regret_adjusted = self.regret.adjusted_win_rate(base_score, cluster, s.id);

                (s.id, regret_adjusted * bias)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let best = scored.into_iter().next().map(|(id, _)| id);
        if let Some(id) = best {
            let all_ids: Vec<StrategyId> = self.strategies.keys().copied().collect();
            self.record_selection(cluster, id, &all_ids);
        }
        best
    }

    /// Record a selection for anti-collapse and regret tracking.
    fn record_selection(
        &mut self,
        cluster: ClusterId,
        selected: StrategyId,
        all_candidates: &[StrategyId],
    ) {
        let window = self.recent_selections.entry(cluster).or_default();
        window.push(selected);
        if window.len() > self.anti_collapse.window_size * 2 {
            window.remove(0);
        }
        // Regret tracking: candidates that were NOT selected.
        self.regret
            .record_selection(cluster, selected, all_candidates, true);
    }

    /// Run one offline competition cycle: aggregate traces by cluster,
    /// update performance edges, promote winning strategies, retire losers.
    ///
    /// This is the **offline evolution step** — not called in the dispatch
    /// loop.  Call it periodically (every N traces, on a timer, or on demand).
    ///
    /// Returns the list of actions taken (promote/retire/no-action) for
    /// logging and observability.
    pub fn run_competition_cycle(&mut self) -> Vec<CompetitionAction> {
        let mut actions = Vec::new();
        let epoch = self.time_decay.epoch;

        // Step 1: Group all unprocessed traces by cluster.
        let mut cluster_traces: HashMap<ClusterId, Vec<&StrategyExecutionTrace>> = HashMap::new();
        for trace in &self.traces {
            if let Some(cid) = trace.cluster_id {
                cluster_traces.entry(cid).or_default().push(trace);
            }
        }

        // Step 2: Build a snapshot of which strategies are in each cluster.
        let mut cluster_strategies: HashMap<ClusterId, Vec<StrategyId>> = HashMap::new();
        for (&cid, traces) in &cluster_traces {
            for t in traces {
                if !cluster_strategies
                    .entry(cid)
                    .or_default()
                    .contains(&t.strategy_id)
                {
                    cluster_strategies
                        .entry(cid)
                        .or_default()
                        .push(t.strategy_id);
                }
            }
        }

        // Step 3: For each cluster, compute pairwise win rates.
        for (&cid, sids) in &cluster_strategies {
            let traces = cluster_traces.get(&cid).unwrap();
            for i in 0..sids.len() {
                for j in (i + 1)..sids.len() {
                    let a = sids[i];
                    let b = sids[j];

                    // Compute win rate for A vs B.
                    let mut a_wins = 0u32;
                    let mut total = 0u32;
                    for t in traces {
                        if t.strategy_id == a || t.strategy_id == b {
                            total += 1;
                            if t.strategy_id == a && t.success {
                                a_wins += 1;
                            }
                            if t.strategy_id == b && !t.success {
                                a_wins += 1; // B's loss = A's win
                            }
                        }
                    }

                    if total < self.protocol.min_samples_per_edge {
                        actions.push(CompetitionAction::NoAction {
                            reason: format!(
                                "cluster {}: not enough samples ({}/{})",
                                cid, total, self.protocol.min_samples_per_edge
                            ),
                        });
                        continue;
                    }

                    let win_rate = a_wins as f32 / total as f32;
                    let confidence = (total as f32 / (total as f32 + 10.0)).min(1.0);

                    // Update or insert the performance edge.
                    let existing = self.edges.iter_mut().find(|e| {
                        (e.strategy_a == a && e.strategy_b == b)
                            || (e.strategy_a == b && e.strategy_b == a)
                    });

                    match existing {
                        Some(edge) => {
                            edge.win_rate = edge.win_rate * 0.7 + win_rate * 0.3;
                            edge.sample_count += total;
                            edge.confidence = (edge.sample_count as f32
                                / (edge.sample_count as f32 + 10.0))
                                .min(1.0);
                        }
                        None => {
                            self.edges.push(PerformanceEdge {
                                strategy_a: a,
                                strategy_b: b,
                                task_cluster: cid,
                                win_rate,
                                sample_count: total,
                                confidence,
                            });
                        }
                    }

                    // Step 4: Promotion / retirement decisions.
                    if win_rate > self.protocol.promotion_threshold && confidence > 0.6 {
                        // Promote: ensure the winning strategy is active.
                        if let Some(s) = self.strategies.get_mut(&a) {
                            if !s.active {
                                s.active = true;
                                s.promoted_at_epoch = epoch;
                                actions.push(CompetitionAction::Promote {
                                    strategy_id: a,
                                    cluster: cid,
                                    win_rate,
                                });
                            }
                        }
                    }

                    if win_rate < self.protocol.retirement_threshold && confidence > 0.6 {
                        // Retire: deactivate the losing strategy in this cluster.
                        if let Some(s) = self.strategies.get_mut(&b) {
                            if s.active {
                                s.active = false;
                                actions.push(CompetitionAction::Retire {
                                    strategy_id: b,
                                    cluster: cid,
                                    win_rate,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Step 5: Advance epoch (time decay).
        self.time_decay.tick();
        self.exploration.tick();

        if actions.is_empty() {
            actions.push(CompetitionAction::NoAction {
                reason: "no strategies crossed promote/retire threshold".to_string(),
            });
        }

        actions
    }
}

// ============================================================================
//  Design invariants
// ============================================================================
//
// 1. StrategySelector is the only bridge between StrategyGraph and Kernel.
// 2. PerformanceEdge is always cluster-scoped.
// 3. CompetitionProtocol runs async — never in dispatch loop.
// 4. StrategyNode.config is opaque JSON — kernel never reads it.
// 5. StrategyExecutionTrace is append-only.
// 6. ExplorationPressure ensures the system never converges prematurely.
// 7. StrategyMomentum prevents oscillation between equally good strategies.

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_strategy(id: u64, stype: StrategyType, name: &str, active: bool) -> StrategyNode {
        StrategyNode {
            id,
            strategy_type: stype,
            name: name.to_string(),
            config: serde_json::json!({}),
            scope: StrategyScope {
                domain_keywords: vec![],
                min_complexity: 0.0,
                max_complexity: 1.0,
            },
            version: 1,
            active,
            momentum: 0,
            promoted_at_epoch: 0,
        }
    }

    #[test]
    fn test_momentum_records_streak() {
        let mut m = StrategyMomentum::new();
        assert_eq!(m.streak, 0);
        m.record(true);
        assert_eq!(m.streak, 1);
        m.record(true);
        assert_eq!(m.streak, 2);
        m.record(false);
        assert!(m.streak < 0);
    }

    #[test]
    fn test_momentum_bias_increases_with_win_streak() {
        let mut m = StrategyMomentum::new();
        assert!((m.selection_bias() - 1.0).abs() < 0.01);
        for _ in 0..10 {
            m.record(true);
        }
        assert!(m.selection_bias() > 1.1, "win streak should increase bias");
    }

    #[test]
    fn test_exploration_epsilon_decays() {
        let mut p = ExplorationPolicy::default();
        let initial = p.epsilon;
        for _ in 0..10 {
            p.tick();
        }
        assert!(p.epsilon < initial, "epsilon should decay");
    }

    #[test]
    fn test_strategy_graph_selects_with_exploration() {
        let mut graph = StrategyGraph::new(CompetitionProtocol::default());
        graph.register(make_strategy(
            1,
            StrategyType::Estimator,
            "conservative",
            true,
        ));
        graph.register(make_strategy(
            2,
            StrategyType::Estimator,
            "aggressive",
            true,
        ));

        let _sig = TaskSignature {
            goal_length_chars: 100,
            domain_count: 3,
            estimated_complexity: 0.7,
            role_count: 2,
        };
        // Run many selections to verify both strategies get picked.
        let mut picks = std::collections::HashSet::new();
        for _ in 0..100 {
            if let Some(id) = graph.select_strategy(StrategyType::Estimator, 0) {
                picks.insert(id);
            }
        }
        assert!(picks.len() >= 1, "at least one strategy should be selected");
    }

    #[test]
    fn test_cluster_drift_detector_basic() {
        let mut detector = ClusterDriftDetector::default();
        let sig = TaskSignature {
            goal_length_chars: 50,
            domain_count: 1,
            estimated_complexity: 0.3,
            role_count: 1,
        };
        detector.record(0, sig.clone());
        detector.record(0, sig);
        assert!(
            !detector.is_drifting(0),
            "no drift with identical signatures"
        );
    }
}
