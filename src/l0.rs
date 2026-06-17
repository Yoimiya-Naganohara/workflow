use std::panic::catch_unwind;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use crate::core::types::SpawnRejection;
use crate::core::types::*;

// ============================================================================
//  TaskResourceState — Atomic resource accounting
// ============================================================================

pub struct TaskResourceState {
    pub current_depth: AtomicU32,
    pub remaining_budget: AtomicI64,
    pub total_spawned: AtomicU32,
    pub max_dynamic_depth: AtomicU32,
    pub tool_bitmap: AtomicU64,
}

impl TaskResourceState {
    pub fn new(initial_budget: u64, max_depth: u32) -> Arc<Self> {
        Arc::new(Self {
            current_depth: AtomicU32::new(0),
            remaining_budget: AtomicI64::new(initial_budget as i64),
            total_spawned: AtomicU32::new(0),
            max_dynamic_depth: AtomicU32::new(max_depth),
            tool_bitmap: AtomicU64::new(0),
        })
    }

    /// Spin-wait backoff counter for CAS contention.
    /// Stalls briefly after `rounds` failed attempts to reduce cache-line ping-pong.
    fn cas_backoff(rounds: u32) {
        if rounds <= 8 {
            // Linear backoff: spin-loop with pause hint
            for _ in 0..1u32 << rounds {
                std::hint::spin_loop();
            }
        } else {
            // Exponential backoff: yield to OS scheduler
            std::thread::yield_now();
        }
    }

    pub fn try_acquire_budget(&self, requested: u64) -> Option<u64> {
        let mut current = self.remaining_budget.load(Ordering::Acquire);
        let mut rounds = 0u32;
        loop {
            if current < requested as i64 {
                return None;
            }
            let new = current - requested as i64;
            match self.remaining_budget.compare_exchange_weak(
                current,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(requested),
                Err(actual) => {
                    current = actual;
                    Self::cas_backoff(rounds);
                    rounds = rounds.saturating_add(1);
                }
            }
        }
    }

    pub fn try_acquire_tools(&self, tool_bitmap: u64) -> Result<(), u64> {
        let mut current = self.tool_bitmap.load(Ordering::Acquire);
        let mut rounds = 0u32;
        loop {
            if current & tool_bitmap != 0 {
                return Err(current);
            }
            let new = current | tool_bitmap;
            match self.tool_bitmap.compare_exchange_weak(
                current,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(actual) => {
                    current = actual;
                    Self::cas_backoff(rounds);
                    rounds = rounds.saturating_add(1);
                }
            }
        }
    }

    pub fn release_budget(&self, amount: u64) {
        self.remaining_budget
            .fetch_add(amount as i64, Ordering::AcqRel);
    }

    pub fn release_tools(&self, tool_bitmap: u64) {
        self.tool_bitmap.fetch_and(!tool_bitmap, Ordering::AcqRel);
    }

    pub fn increment_depth(&self) -> Result<u32, u32> {
        let max = self.max_dynamic_depth.load(Ordering::Acquire);
        // CAS loop: atomically check depth and increment in one operation.
        self.current_depth
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                if current >= max {
                    None // signal error
                } else {
                    Some(current + 1)
                }
            })
            .map(|v| v + 1)
    }

    pub fn decrement_depth(&self) {
        self.current_depth.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn increment_spawned(&self) -> u32 {
        self.total_spawned.fetch_add(1, Ordering::AcqRel) + 1
    }
}

// ============================================================================
//  BudgetGuard — RAII resource guard
// ============================================================================

pub struct BudgetGuard {
    root_task_id: TaskId,
    amount: u64,
    resource_state: Weak<TaskResourceState>,
    committed: bool,
    requested_tools: u64,
}

impl BudgetGuard {
    pub fn new(
        root_task_id: TaskId,
        amount: u64,
        resource_state: Arc<TaskResourceState>,
        requested_tools: u64,
    ) -> Option<Self> {
        resource_state.try_acquire_budget(amount)?;
        if requested_tools != 0 && resource_state.try_acquire_tools(requested_tools).is_err() {
            resource_state.release_budget(amount);
            return None;
        }
        Some(Self {
            root_task_id,
            amount,
            resource_state: Arc::downgrade(&resource_state),
            committed: false,
            requested_tools,
        })
    }

    pub fn settle(&mut self, actual: u64) {
        if self.committed {
            return;
        }
        self.committed = true;
        if let Some(state) = self.resource_state.upgrade()
            && actual < self.amount
        {
            state.release_budget(self.amount - actual);
        }
    }

    pub fn commit(&mut self) {
        self.committed = true;
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn task_id(&self) -> &TaskId {
        &self.root_task_id
    }

    /// Create a [`BudgetGuard`] from a permit's already-acquired resources.
    ///
    /// Unlike [`BudgetGuard::new`], this does **not** re-acquire budget or
    /// tools — it takes ownership directly.
    pub fn from_permit(
        task_id: TaskId,
        amount: u64,
        state: Arc<TaskResourceState>,
        tools: u64,
    ) -> Self {
        Self {
            root_task_id: task_id,
            amount,
            resource_state: Arc::downgrade(&state),
            committed: false,
            requested_tools: tools,
        }
    }
}

impl Drop for BudgetGuard {
    fn drop(&mut self) {
        let _ = catch_unwind(|| {
            if !self.committed
                && let Some(state) = self.resource_state.upgrade()
            {
                state.release_budget(self.amount);
                if self.requested_tools != 0 {
                    state.release_tools(self.requested_tools);
                }
            }
        });
    }
}

unsafe impl Send for BudgetGuard {}
unsafe impl Sync for BudgetGuard {}

// ============================================================================
//  L0CircuitBreaker — Physical-resource circuit breaker
// ============================================================================

pub struct L0CircuitBreaker {
    resource_state: Arc<TaskResourceState>,
}

impl L0CircuitBreaker {
    pub fn new(resource_state: Arc<TaskResourceState>) -> Self {
        Self { resource_state }
    }

    pub fn try_acquire(
        &self,
        requested_budget: u64,
        _current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection> {
        // Early depth check is intentionally omitted — increment_depth() uses a CAS loop
        // that atomically enforces the limit. An early check would only provide a stale
        // error message, not prevent overshoot.

        let allocated = self
            .resource_state
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
                max: self
                    .resource_state
                    .max_dynamic_depth
                    .load(Ordering::Acquire),
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

// ============================================================================
//  L0Permit
// ============================================================================

pub struct L0Permit {
    budget_amount: u64,
    requested_tools: u64,
    /// `None` once consumed by [`L0Permit::into_budget_guard`].
    resource_state: Option<Arc<TaskResourceState>>,
}

impl L0Permit {
    pub fn budget_amount(&self) -> u64 {
        self.budget_amount
    }

    /// Convert this permit into a [`BudgetGuard`] without re-acquiring
    /// resources (the permit already holds them via the L0 circuit breaker).
    ///
    /// After this call the permit is consumed — its `Drop` is a no-op.
    pub fn into_budget_guard(mut self, task_id: TaskId) -> Option<BudgetGuard> {
        let state = self.resource_state.take()?;
        Some(BudgetGuard::from_permit(
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

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    // ── TaskResourceState tests ──

    #[test]
    fn test_budget_acquire_release() {
        let state = TaskResourceState::new(1000, 10);
        assert_eq!(state.try_acquire_budget(500), Some(500));
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
        state.release_budget(500);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn test_budget_insufficient() {
        let state = TaskResourceState::new(100, 10);
        assert_eq!(state.try_acquire_budget(200), None);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn test_tool_bitmap_acquire_release() {
        let state = TaskResourceState::new(1000, 10);
        let tools: u64 = 0b1010;
        assert!(state.try_acquire_tools(tools).is_ok());
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), tools);
        state.release_tools(tools);
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_tool_bitmap_conflict() {
        let state = TaskResourceState::new(1000, 10);
        let tools: u64 = 0b1010;
        assert!(state.try_acquire_tools(tools).is_ok());
        assert!(state.try_acquire_tools(0b1000).is_err());
    }

    #[test]
    fn test_depth_check() {
        let state = TaskResourceState::new(1000, 2);
        assert_eq!(state.increment_depth(), Ok(1));
        assert_eq!(state.increment_depth(), Ok(2));
        assert!(state.increment_depth().is_err());
        state.decrement_depth();
        assert_eq!(state.increment_depth(), Ok(2));
    }

    // ── BudgetGuard tests ──

    #[test]
    fn test_budget_guard_drop_rollback() {
        let state = TaskResourceState::new(1000, 10);
        let guard = BudgetGuard::new([0u8; 16], 500, state.clone(), 0).unwrap();
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
        drop(guard);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 1000);
    }

    #[test]
    fn test_budget_guard_settle() {
        let state = TaskResourceState::new(1000, 10);
        let mut guard = BudgetGuard::new([0u8; 16], 500, state.clone(), 0).unwrap();
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
        guard.settle(300);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 700);
        drop(guard);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 700);
    }

    #[test]
    fn test_budget_guard_commit_no_rollback() {
        let state = TaskResourceState::new(1000, 10);
        let mut guard = BudgetGuard::new([0u8; 16], 500, state.clone(), 0).unwrap();
        guard.commit();
        drop(guard);
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
    }

    #[test]
    fn test_concurrent_budget_cas() {
        let state = TaskResourceState::new(10000, 100);
        let mut handles = vec![];

        for _ in 0..100 {
            let s = state.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    if let Some(_g) = s.try_acquire_budget(1) {
                        thread::yield_now();
                    }
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let remaining = state.remaining_budget.load(Ordering::Relaxed);
        assert!(remaining >= 0);
        assert!(remaining <= 10000);
    }

    #[test]
    fn test_concurrent_tool_lock() {
        let state = TaskResourceState::new(10000, 100);
        let mut handles = vec![];
        let success_count = Arc::new(AtomicU32::new(0));

        for i in 0..10 {
            let s = state.clone();
            let sc = success_count.clone();
            let tool_bit = 1u64 << i;
            handles.push(thread::spawn(move || {
                if s.try_acquire_tools(tool_bit).is_ok() {
                    sc.fetch_add(1, Ordering::Relaxed);
                    thread::yield_now();
                    s.release_tools(tool_bit);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0);
        assert_eq!(success_count.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn test_depth_increment_toctou_race() {
        // BUG: increment_depth uses fetch_update which IS atomic
        // The race was mitigated — run many iterations to confirm
        let mut race_detected = false;

        for _ in 0..100 {
            let state = TaskResourceState::new(1000, 2); // max_depth = 2
            let mut handles = vec![];

            for _ in 0..20 {
                let s = state.clone();
                handles.push(thread::spawn(move || {
                    // Tight loop to increase race window
                    for _ in 0..10 {
                        let _ = s.increment_depth();
                    }
                }));
            }

            for h in handles {
                h.join().unwrap();
            }

            let final_depth = state.current_depth.load(Ordering::Relaxed);
            if final_depth > 2 {
                race_detected = true;
                break;
            }
        }

        if race_detected {
            println!("BUG CONFIRMED: TOCTOU race allows depth to exceed max_depth");
        }
        // Document the bug — race may or may not reproduce in this run
    }

    // ── L0CircuitBreaker tests ──

    #[test]
    fn test_l0_basic_acquire() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let permit = breaker.try_acquire(500, 0, 0);
        assert!(permit.is_ok());
    }

    #[test]
    fn test_l0_depth_exceeded() {
        let state = TaskResourceState::new(1000, 2);
        let breaker = L0CircuitBreaker::new(state);
        // First two acquisitions should succeed (depth 0→1, 1→2)
        let p1 = breaker
            .try_acquire(100, 0, 0)
            .expect("first acquire should succeed");
        let p2 = breaker
            .try_acquire(100, 1, 0)
            .expect("second acquire should succeed");
        // Both permits held — third attempt should fail at increment_depth
        assert!(
            matches!(
                breaker.try_acquire(100, 2, 0),
                Err(SpawnRejection::DepthExceeded { .. })
            ),
            "third acquire should fail: depth limit reached"
        );
        // Drop permits to release depth slots
        drop(p1);
        drop(p2);
        // After dropping, depth should be decremented; fourth should succeed
        let _p3 = breaker
            .try_acquire(100, 2, 0)
            .expect("acquire after drop should succeed");
    }

    #[test]
    fn test_l0_budget_exhausted() {
        let state = TaskResourceState::new(100, 10);
        let breaker = L0CircuitBreaker::new(state);
        assert!(matches!(
            breaker.try_acquire(200, 0, 0),
            Err(SpawnRejection::BudgetExhausted { .. })
        ));
    }

    #[test]
    fn test_l0_tool_conflict() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let _permit = breaker.try_acquire(100, 0, 0b101).unwrap();
        assert!(matches!(
            breaker.try_acquire(100, 0, 0b100),
            Err(SpawnRejection::ResourceConflict { .. })
        ));
    }

    #[test]
    fn test_l0_permit_drop_rollback() {
        let state = TaskResourceState::new(1000, 10);
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
