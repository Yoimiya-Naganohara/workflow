pub mod resource;

pub use resource::*;

// Re-export l0.rs content (moved from src/l0.rs)
use crate::core::types::SpawnRejection;
use crate::core::types::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub struct L0CircuitBreaker {
    resource_state: Arc<resource::TaskResourceState>,
}

impl L0CircuitBreaker {
    pub fn new(resource_state: Arc<resource::TaskResourceState>) -> Self {
        Self { resource_state }
    }

    pub fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection> {
        let max_depth = self.resource_state.max_dynamic_depth.load(Ordering::Acquire);
        if current_depth >= max_depth {
            return Err(SpawnRejection::DepthExceeded {
                current: current_depth,
                max: max_depth,
            });
        }

        let allocated =
            self.resource_state
                .try_acquire_budget(requested_budget)
                .ok_or(SpawnRejection::BudgetExhausted {
                    requested: requested_budget,
                    remaining: self.resource_state.remaining_budget.load(Ordering::Acquire),
                })?;

        if requested_tools != 0
            && let Err(_holder_bitmap) = self.resource_state.try_acquire_tools(requested_tools)
        {
            self.resource_state.release_budget(allocated);
            let tool_id = requested_tools.trailing_zeros() as u64;
            return Err(SpawnRejection::ResourceConflict {
                tool_id,
                holder: [0u8; 16],
            });
        }

        if self.resource_state.increment_depth().is_err() {
            self.resource_state.release_budget(allocated);
            if requested_tools != 0 {
                self.resource_state.release_tools(requested_tools);
            }
            return Err(SpawnRejection::DepthExceeded {
                current: self.resource_state.current_depth.load(Ordering::Acquire),
                max: self.resource_state.max_dynamic_depth.load(Ordering::Acquire),
            });
        }
        self.resource_state.increment_spawned();

        Ok(L0Permit {
            budget_amount: allocated,
            requested_tools,
            resource_state: Some(self.resource_state.clone()),
        })
    }

    pub fn calculate_priority(budget_remaining: i64, budget_requested: u64, depth: u32) -> f32 {
        let budget_ratio = if budget_requested == 0 {
            1.0
        } else {
            (budget_remaining as f32 / budget_requested as f32).clamp(0.0, 1.0)
        };
        let depth_factor = if depth == 0 { 1.0 } else { 1.0 / depth as f32 };
        crate::core::types::BUDGET_PRIORITY_WEIGHT * budget_ratio
            + crate::core::types::DEPTH_PRIORITY_WEIGHT * depth_factor
    }
}

pub struct L0Permit {
    budget_amount: u64,
    requested_tools: u64,
    /// `None` once consumed by [`L0Permit::into_budget_guard`].
    resource_state: Option<Arc<resource::TaskResourceState>>,
}

impl L0Permit {
    pub fn budget_amount(&self) -> u64 {
        self.budget_amount
    }

    /// Convert this permit into a [`BudgetGuard`] without re-acquiring
    /// resources (the permit already holds them via the L0 circuit breaker).
    ///
    /// After this call the permit is consumed — its `Drop` is a no-op.
    pub fn into_budget_guard(mut self, task_id: TaskId) -> Option<resource::BudgetGuard> {
        let state = self.resource_state.take()?;
        Some(resource::BudgetGuard::from_permit(
            task_id,
            self.budget_amount,
            state,
            self.requested_tools,
        ))
    }
}

impl Drop for L0Permit {
    fn drop(&mut self) {
        if let Some(ref state) = self.resource_state {
            state.release_budget(self.budget_amount);
            if self.requested_tools != 0 {
                state.release_tools(self.requested_tools);
            }
            state.decrement_depth();
        }
    }
}

/// L0: Physical-resource circuit breaker.
pub trait CircuitBreaker: Send + Sync {
    fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection>;
    fn calculate_priority(&self, budget_remaining: i64, budget_requested: u64, depth: u32) -> f32;
    fn remaining_budget(&self) -> i64;
}

impl CircuitBreaker for L0CircuitBreaker {
    fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection> {
        self.try_acquire(requested_budget, current_depth, requested_tools)
    }

    fn calculate_priority(&self, budget_remaining: i64, budget_requested: u64, depth: u32) -> f32 {
        Self::calculate_priority(budget_remaining, budget_requested, depth)
    }

    fn remaining_budget(&self) -> i64 {
        self.resource_state
            .remaining_budget
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l0_basic_acquire() {
        let state = resource::TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let permit = breaker.try_acquire(500, 0, 0);
        assert!(permit.is_ok());
    }

    #[test]
    fn test_l0_depth_exceeded() {
        let state = resource::TaskResourceState::new(1000, 2);
        let breaker = L0CircuitBreaker::new(state);
        assert!(matches!(
            breaker.try_acquire(100, 2, 0),
            Err(SpawnRejection::DepthExceeded { .. })
        ));
    }

    #[test]
    fn test_l0_budget_exhausted() {
        let state = resource::TaskResourceState::new(100, 10);
        let breaker = L0CircuitBreaker::new(state);
        assert!(matches!(
            breaker.try_acquire(200, 0, 0),
            Err(SpawnRejection::BudgetExhausted { .. })
        ));
    }

    #[test]
    fn test_l0_tool_conflict() {
        let state = resource::TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let _permit = breaker.try_acquire(100, 0, 0b101).unwrap();
        assert!(matches!(
            breaker.try_acquire(100, 0, 0b100),
            Err(SpawnRejection::ResourceConflict { .. })
        ));
    }

    #[test]
    fn test_l0_permit_drop_rollback() {
        let state = resource::TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state.clone());
        {
            let _permit = breaker.try_acquire(500, 0, 0b1).unwrap();
            assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
            assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0b1);
            assert_eq!(state.current_depth.load(Ordering::Relaxed), 1);
        }
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 1000);
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0);
        assert_eq!(state.current_depth.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_priority_formula() {
        let p1 = L0CircuitBreaker::calculate_priority(1000, 500, 1);
        let p2 = L0CircuitBreaker::calculate_priority(100, 500, 1);
        assert!(p1 > p2);
        let p3 = L0CircuitBreaker::calculate_priority(500, 500, 1);
        let p4 = L0CircuitBreaker::calculate_priority(500, 500, 5);
        assert!(p3 > p4);
    }
}
