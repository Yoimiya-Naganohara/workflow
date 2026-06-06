use std::panic::catch_unwind;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use crate::core::types::TaskId;

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

    pub fn try_acquire_budget(&self, requested: u64) -> Option<u64> {
        let mut current = self.remaining_budget.load(Ordering::Acquire);
        loop {
            if current < requested as i64 {
                return None;
            }
            let new = current - requested as i64;
            match self
                .remaining_budget
                .compare_exchange_weak(current, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(requested),
                Err(actual) => current = actual,
            }
        }
    }

    pub fn try_acquire_tools(&self, tool_bitmap: u64) -> Result<(), u64> {
        let mut current = self.tool_bitmap.load(Ordering::Acquire);
        loop {
            if current & tool_bitmap != 0 {
                return Err(current);
            }
            let new = current | tool_bitmap;
            match self
                .tool_bitmap
                .compare_exchange_weak(current, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(()),
                Err(actual) => current = actual,
            }
        }
    }

    pub fn release_budget(&self, amount: u64) {
        self.remaining_budget.fetch_add(amount as i64, Ordering::AcqRel);
    }

    pub fn release_tools(&self, tool_bitmap: u64) {
        self.tool_bitmap.fetch_and(!tool_bitmap, Ordering::AcqRel);
    }

    pub fn increment_depth(&self) -> Result<u32, u32> {
        let current = self.current_depth.load(Ordering::Acquire);
        let max = self.max_dynamic_depth.load(Ordering::Acquire);
        if current >= max {
            return Err(current);
        }
        self.current_depth.fetch_add(1, Ordering::AcqRel);
        Ok(current + 1)
    }

    pub fn decrement_depth(&self) {
        self.current_depth.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn increment_spawned(&self) -> u32 {
        self.total_spawned.fetch_add(1, Ordering::AcqRel) + 1
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

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
}
