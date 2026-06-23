use crate::runtime::capability::TaskOutcome;
use crate::runtime::task_graph::TaskNode;

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
        _task: &TaskNode,
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
        _task: &TaskNode,
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
        if let Some(last) = recent_outcomes.last() {
            if last.latency_ms > self.latency_threshold_ms {
                return Some(EscalationReason::HumanRequired {
                    reason: format!(
                        "Latency {}ms > {}ms",
                        last.latency_ms, self.latency_threshold_ms
                    ),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::TaskId;

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
