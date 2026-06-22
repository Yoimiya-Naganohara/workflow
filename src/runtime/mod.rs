#![allow(clippy::module_inception)]

pub mod config;
pub mod decomposition;
pub mod dispatch;
pub mod event;
pub mod graph_analytics;
pub mod optimizer;
pub mod pipeline;
pub mod runtime;
pub mod runtime_loop;
pub mod scheduler;
pub mod strategy_graph;
pub mod task_graph;

pub use config::{AgentRuntimeConfig, RoleTemplate};
pub use event::RuntimeEvent;
pub use runtime::AgentRuntime;
