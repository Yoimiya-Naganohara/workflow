//! Re-export from the merged guard module.
//!
//! L0 budget/resource management is now defined in `core::guard`.
//! This file is kept for backward compatibility.

pub use wf_core::guard::{
    BudgetGuard, CircuitBreaker, L0CircuitBreaker, L0Permit, TaskResourceState,
};
