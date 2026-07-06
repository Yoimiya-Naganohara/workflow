//! Re-export from the merged guard module.
//!
//! L-1 admission control is now defined in `core::guard`.
//! This file is kept for backward compatibility.

pub use wf_core::guard::{AdmissionControl, AdmissionController, AdmissionPermit};
