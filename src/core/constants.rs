//! Central constants for the workflow system.

/// Embedding vector dimension (all-MiniLM-L6-v2).
pub const EMBEDDING_DIM: usize = 384;

/// Default LLM temperature for chat agents.
pub const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Default max tokens for chat agents.
pub const DEFAULT_MAX_TOKENS: u64 = 40000;

/// Priority weight for budget ratio (vs depth factor).
pub const BUDGET_PRIORITY_WEIGHT: f32 = 0.6;

/// Priority weight for depth factor (vs budget ratio).
pub const DEPTH_PRIORITY_WEIGHT: f32 = 0.4;

/// Default L1 confidence threshold.
pub const DEFAULT_L1_CONFIDENCE: f32 = 0.5;

/// Default semantic conflict threshold.
pub const DEFAULT_SEMANTIC_THRESHOLD: f32 = -0.6;

/// Default budget for a new runtime.
pub const DEFAULT_RUNTIME_BUDGET: u64 = 10_000;

/// Default max agent spawn depth.
pub const DEFAULT_MAX_DEPTH: u32 = 5;

/// Default max concurrent agents.
pub const DEFAULT_MAX_AGENTS: usize = 10;

/// Default admission timeout in ms.
pub const DEFAULT_ADMISSION_TIMEOUT_MS: u64 = 100;

/// Default suspend timeout in ms.
pub const DEFAULT_SUSPEND_TIMEOUT_MS: u64 = 50;

/// Default side width for TUI sidebar.
pub const SIDEBAR_WIDTH: u16 = 28;

/// Max consecutive failures before L2 collapses.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Default max_tokens for L2 LLM judge.
pub const L2_JUDGE_MAX_TOKENS: u64 = 500;

/// Default temperature for L2 LLM judge.
pub const L2_JUDGE_TEMPERATURE: f64 = 0.3;

/// Weight for task similarity in L1 confidence (default 0.35).
pub const L1_TASK_WEIGHT: f32 = 0.35;

/// Weight for role similarity in L1 confidence (default 0.25).
pub const L1_ROLE_WEIGHT: f32 = 0.25;

/// Weight for value alignment in L1 confidence (default 0.25).
pub const L1_VALUE_WEIGHT: f32 = 0.25;

/// Weight for recency in L1 confidence (decay per hour, default 0.15).
pub const L1_RECENCY_WEIGHT: f32 = 0.15;

/// How much the L2 override boosts a matching experience (multiplier).
pub const L2_OVERRIDE_BOOST: f32 = 1.5;

/// Anomaly ratio threshold: if requested_budget > remaining * this, flag it.
pub const BUDGET_ANOMALY_RATIO: f64 = 0.8;

/// Maximum responsibility chain length before flagging.
pub const MAX_CHAIN_LENGTH: usize = 20;
