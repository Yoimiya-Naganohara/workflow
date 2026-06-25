#![allow(clippy::module_inception)]

pub mod agent_lifecycle;
pub mod capability;
pub mod config;
pub mod decomposition;
pub mod dispatch;
pub mod embedding_analyzer;
pub mod escalation;
pub mod event;
pub mod graph_analytics;
pub mod optimizer;
pub mod pipeline;
pub mod runtime;
pub mod runtime_loop;
pub mod scheduler;
pub mod strategy_graph;
pub mod task_graph;
pub mod validation;

pub use config::{AgentRuntimeConfig, RoleTemplate};
pub use event::RuntimeEvent;
pub use runtime::AgentRuntime;
pub mod agent_exec;
pub mod agent_stream;
