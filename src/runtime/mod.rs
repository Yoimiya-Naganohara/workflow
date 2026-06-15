#![allow(clippy::module_inception)]

pub mod config;
pub mod event;
pub mod optimizer;
pub mod pipeline;
pub mod runtime;
pub mod runtime_loop;

pub use config::{AgentRuntimeConfig, RoleTemplate};
pub use event::RuntimeEvent;
pub use runtime::AgentRuntime;
