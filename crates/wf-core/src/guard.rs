//! Unified guard layer — merges L-1 admission control and L0 budget/CAS.
//!
//! Previously split across `admission.rs` (L-1) and `l0.rs` (L0), these
//! two layers manage the same RAII lifecycle (`BudgetGuard`) and share
//! the same resource ownership semantics.  This module consolidates them
//! into a single `GuardLayer` with no functional change.
//!
//! # Architecture
//!
//! ```text
//! GuardLayer
//!   ├── AdmissionController  — tokio semaphore (max concurrent agents)
//!   ├── TaskResourceState    — CAS atomic budget + depth + tool bitmap
//!   ├── L0CircuitBreaker     — orchestrates acquire (budget + depth + tools)
//!   ├── L0Permit             — intermediate resource handle
//!   └── BudgetGuard          — RAII guard (budget + tools)
//! ```
//!
//! # Old module compatibility
//!
//! `admission.rs` and `l0.rs` re-export all public items from here.
//! Existing imports continue to work unchanged.

use std::panic::catch_unwind;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::SpawnRejection;
use crate::*;

// ============================================================================
//  L-1: AdmissionController — tokio semaphore concurrency gate
// ============================================================================

/// Tokio semaphore-based admission control.
///
/// Limits the number of concurrently executing agents.  Permits are
/// released when the `AdmissionPermit` is dropped.
pub struct AdmissionController {
    semaphore: Arc<Semaphore>,
    timeout_ms: u64,
}

impl AdmissionController {
    pub fn new(max_concurrent: usize, timeout_ms: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            timeout_ms,
        }
    }

    /// Acquire a semaphore permit with timeout.
    pub async fn acquire_owned(&self) -> Result<AdmissionPermit, SpawnRejection> {
        let permit = timeout(
            Duration::from_millis(self.timeout_ms),
            self.semaphore.clone().acquire_owned(),
        )
        .await;

        match permit {
            Ok(Ok(permit)) => Ok(AdmissionPermit { permit }),
            Ok(Err(_)) => Err(SpawnRejection::SystemOverloaded),
            Err(_) => Err(SpawnRejection::SystemOverloaded),
        }
    }

    pub fn available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }
}

/// L-1: Concurrency admission trait.
#[async_trait::async_trait]
pub trait AdmissionControl: Send + Sync {
    async fn acquire(&self) -> Result<AdmissionPermit, SpawnRejection>;
    fn available_permits(&self) -> usize;
}

#[async_trait::async_trait]
impl AdmissionControl for AdmissionController {
    async fn acquire(&self) -> Result<AdmissionPermit, SpawnRejection> {
        self.acquire_owned().await
    }

    fn available_permits(&self) -> usize {
        AdmissionController::available_permits(self)
    }
}

/// A held admission permit.  Released on drop (returns permit to semaphore).
pub struct AdmissionPermit {
    #[allow(dead_code)]
    /// Held for RAII — returned to semaphore on drop. Not read directly.
    permit: tokio::sync::OwnedSemaphorePermit,
}

// ============================================================================
//  L0: TaskResourceState — atomic resource accounting
// ============================================================================

/// Atomic resource state shared across agents.
///
/// Uses CAS (compare-and-swap) for lock-free concurrent access.
/// Backoff strategies prevent cache-line ping-pong under contention.
#[derive(Debug)]
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

    /// Spin-wait backoff for CAS contention.
    /// Uses only CPU pauses (not thread::yield_now) so it's safe in async contexts.
    fn cas_backoff(rounds: u32) {
        // Cap total spin iterations to avoid blocking the tokio thread.
        let r = rounds.min(8);
        for _ in 0..1u32 << r {
            std::hint::spin_loop();
        }
    }

    /// Atomically deduct `requested` from remaining budget.
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

    /// Atomically acquire tool bits (no overlap with current bitmap).
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
        self.current_depth
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                let max = self.max_dynamic_depth.load(Ordering::Acquire);
                if current >= max {
                    None
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

    pub fn budget_remaining(&self) -> i64 {
        self.remaining_budget.load(Ordering::Acquire)
    }

    pub fn current_depth_value(&self) -> u32 {
        self.current_depth.load(Ordering::Acquire)
    }

    pub fn max_depth(&self) -> u32 {
        self.max_dynamic_depth.load(Ordering::Acquire)
    }
}

// ============================================================================
//  BudgetGuard — RAII resource guard (budget + depth + tools + permit)
// ============================================================================

/// RAII guard that holds budget + tools.
///
/// Dropping the guard releases budget and tool bits unless committed.
#[derive(Debug, Clone)]
pub struct BudgetGuard {
    root_task_id: TaskId,
    amount: u64,
    resource_state: Weak<TaskResourceState>,
    committed: bool,
    requested_tools: u64,
}

impl BudgetGuard {
    /// Create a new BudgetGuard by acquiring budget and tools.
    /// Returns `None` if budget is insufficient.
    pub fn new(
        root_task_id: TaskId,
        requested: u64,
        state: Arc<TaskResourceState>,
        tools: u64,
    ) -> Option<Self> {
        let amount = state.try_acquire_budget(requested)?;
        if tools != 0 {
            let _ = state.try_acquire_tools(tools);
        }
        Some(Self {
            root_task_id,
            amount,
            resource_state: Arc::downgrade(&state),
            committed: false,
            requested_tools: tools,
        })
    }

    /// Settle to a final (possibly smaller) budget amount.
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

    /// Commit — mark as settled; Drop will NOT refund.
    pub fn commit(&mut self) {
        self.committed = true;
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn task_id(&self) -> &TaskId {
        &self.root_task_id
    }

    /// Create a BudgetGuard from already-acquired resources (no new acquire).
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

// ============================================================================
//  L0CircuitBreaker — orchestrates deep resource acquisition
// ============================================================================

/// Circuit breaker trait for L0 resource acquisition.
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

/// The standard L0 circuit breaker implementation.
pub struct L0CircuitBreaker {
    pub(crate) resource_state: Arc<TaskResourceState>,
}

impl L0CircuitBreaker {
    pub fn new(resource_state: Arc<TaskResourceState>) -> Self {
        Self { resource_state }
    }

    /// Try to acquire resources for a new agent spawn.
    /// Checks depth, budget, and tool conflicts atomically.
    pub fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection> {
        // Depth check
        let _ =
            self.resource_state
                .increment_depth()
                .map_err(|_| SpawnRejection::DepthExceeded {
                    current: current_depth,
                    max: self.resource_state.max_depth(),
                })?;

        // Budget check
        let budget = self
            .resource_state
            .try_acquire_budget(requested_budget)
            .ok_or_else(|| {
                self.resource_state.decrement_depth();
                let remaining = self.resource_state.budget_remaining();
                SpawnRejection::BudgetExhausted {
                    requested: requested_budget,
                    remaining,
                }
            })?;

        // Tool bitmap check
        if requested_tools != 0
            && self
                .resource_state
                .try_acquire_tools(requested_tools)
                .is_err()
        {
            self.resource_state.release_budget(budget);
            self.resource_state.decrement_depth();
            return Err(SpawnRejection::ResourceConflict {
                tool_id: requested_tools,
                holder: [0u8; 16],
            });
        }

        Ok(L0Permit {
            budget_amount: budget,
            requested_tools,
            resource_state: Some(self.resource_state.clone()),
        })
    }

    pub fn calculate_priority(budget_remaining: i64, budget_requested: u64, depth: u32) -> f32 {
        if budget_remaining <= 0 || budget_requested == 0 {
            return 0.0;
        }
        let budget_ratio = budget_requested as f32 / budget_remaining as f32;
        let depth_penalty = depth as f32 * 0.1;
        (1.0 / (budget_ratio + 0.1)) - depth_penalty
    }
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

/// Intermediate resource permit from L0 acquisition.
/// Can be converted into a `BudgetGuard` which adds RAII semantics.
#[derive(Debug)]
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

    /// Convert into a `BudgetGuard` (consumes the permit).
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

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use tokio::task::JoinSet;

    // ── AdmissionController tests ──

    #[tokio::test]
    async fn test_admission_basic() {
        let controller = AdmissionController::new(10, 100);
        let permit = controller.acquire_owned().await;
        assert!(permit.is_ok());
        assert_eq!(controller.available_permits(), 9);
    }

    #[allow(unused_variables)]
    #[tokio::test]
    async fn test_admission_timeout() {
        let controller = AdmissionController::new(1, 50);
        let p = controller.acquire_owned().await.unwrap();
        let result = controller.acquire_owned().await;
        assert!(matches!(result, Err(SpawnRejection::SystemOverloaded)));
    }

    #[tokio::test]
    async fn test_admission_concurrent() {
        let controller = Arc::new(AdmissionController::new(5, 100));
        let mut set = JoinSet::new();
        let mut permits = Vec::new();
        for _ in 0..10 {
            let c = controller.clone();
            set.spawn(async move { c.acquire_owned().await });
        }
        let mut successes = 0;
        let mut failures = 0;
        while let Some(result) = set.join_next().await {
            match result.unwrap() {
                Ok(permit) => {
                    successes += 1;
                    permits.push(permit);
                }
                Err(_) => failures += 1,
            }
        }
        assert_eq!(successes, 5);
        assert_eq!(failures, 5);
    }

    #[allow(unused_variables)]
    #[tokio::test]
    async fn test_admission_release_on_drop() {
        let controller = Arc::new(AdmissionController::new(2, 100));
        {
            let p1 = controller.acquire_owned().await.unwrap();
            let p2 = controller.acquire_owned().await.unwrap();
            assert_eq!(controller.available_permits(), 0);
        }
        assert_eq!(controller.available_permits(), 2);
    }

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
        let tools = 0b1010;
        assert!(state.try_acquire_tools(tools).is_ok());
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), tools);
        state.release_tools(tools);
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_tool_bitmap_conflict() {
        let state = TaskResourceState::new(1000, 10);
        assert!(state.try_acquire_tools(0b0011).is_ok());
        assert!(state.try_acquire_tools(0b0010).is_err());
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

    // ── L0CircuitBreaker tests ──

    #[test]
    fn test_l0_basic_acquire() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let permit = breaker.try_acquire(500, 0, 0);
        assert!(permit.is_ok());
    }

    #[allow(unused_variables)]
    #[test]
    fn test_l0_depth_exceeded() {
        let state = TaskResourceState::new(1000, 2);
        let breaker = L0CircuitBreaker::new(state);
        let p1 = breaker
            .try_acquire(100, 0, 0)
            .expect("first should succeed");
        let p2 = breaker
            .try_acquire(100, 1, 0)
            .expect("second should succeed");
        assert!(
            matches!(
                breaker.try_acquire(100, 2, 0),
                Err(SpawnRejection::DepthExceeded { .. })
            ),
            "third should fail: depth limit reached"
        );
        drop(p1);
        drop(p2);
        let p3 = breaker
            .try_acquire(100, 2, 0)
            .expect("after drop should succeed");
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

    #[allow(unused_variables)]
    #[test]
    fn test_l0_tool_conflict() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state);
        let p = breaker.try_acquire(100, 0, 0b101).unwrap();
        assert!(matches!(
            breaker.try_acquire(100, 0, 0b100),
            Err(SpawnRejection::ResourceConflict { .. })
        ));
    }

    #[allow(unused_variables)]
    #[test]
    fn test_l0_permit_drop_rollback() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state.clone());
        {
            let p = breaker.try_acquire(500, 0, 0b1).unwrap();
            assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 500);
        }
        assert_eq!(state.remaining_budget.load(Ordering::Relaxed), 1000);
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

    #[test]
    fn test_l0_permit_into_budget_guard() {
        let state = TaskResourceState::new(1000, 10);
        let breaker = L0CircuitBreaker::new(state.clone());
        let permit = breaker.try_acquire(300, 0, 0).unwrap();
        let guard = permit.into_budget_guard([42u8; 16]);
        assert!(guard.is_some());
        assert_eq!(guard.unwrap().amount(), 300);
    }

    // ── Concurrent stress tests ──

    #[test]
    fn test_concurrent_cas_budget() {
        let state = TaskResourceState::new(100_000, 100);
        let mut handles = Vec::new();
        for _ in 0..16 {
            let s = state.clone();
            handles.push(thread::spawn(move || {
                let mut acquired = 0u64;
                for _ in 0..100 {
                    if let Some(amt) = s.try_acquire_budget(100) {
                        acquired += amt;
                    }
                }
                acquired
            }));
        }
        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        let remaining = state.budget_remaining();
        assert_eq!(total as i64 + remaining, 100_000);
    }

    #[tokio::test]
    async fn test_concurrent_tool_bitmap_no_overlap() {
        let state = TaskResourceState::new(100_000, 100);
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = state.clone();
            handles.push(tokio::spawn(async move {
                let tool_bit = 1u64 << i;
                for _ in 0..64 {
                    if let Ok(()) = s.try_acquire_tools(tool_bit) {
                        tokio::task::yield_now().await;
                        s.release_tools(tool_bit);
                    }
                    tokio::task::yield_now().await;
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(state.tool_bitmap.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_budget_guard_concurrent_drop() {
        let state = TaskResourceState::new(100_000, 100);
        let mut handles = Vec::new();
        for _ in 0..16 {
            let s = state.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    let guard = BudgetGuard::new([0u8; 16], 1000, s.clone(), 0);
                    drop(guard);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(state.budget_remaining(), 100_000);
    }
}
