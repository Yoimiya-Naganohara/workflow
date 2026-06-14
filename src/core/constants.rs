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

/// Default max tool-calling turns for MCP chat (30 = safe upper bound).
pub const DEFAULT_MAX_TOOL_TURNS: usize = 30;

/// Memo usage instructions appended to every role's system prompt.
pub const MEMO_INSTRUCTIONS: &str = "\n\n## Persistent Memory (Memos)\n\nYou MUST use write_memo to persist information the user asks you to remember, and read_memo at the start of each task to recall previously stored context.\n\nCRITICAL: When the user says anything like \"remember\", \"keep in mind\", \"note that\", \"store this\", or asks you to recall something from earlier — you MUST call write_memo immediately. This is how you remember across conversation turns.\n\nMemo tools available:\n- write_memo(key, value) — Store a note. Overwrites if key exists.\n- read_memo(key) — Retrieve a stored note.\n- list_memos(prefix) — List all stored keys, optionally filtered.\n- delete_memo(key) — Remove a stored note.\n\nKey naming convention: use slash-prefixed paths like \"task/findings\", \"decision/approach\", \"user/preferences\", \"project/deadlines\".\n\nExample: If the user says \"remember the port is 8080\", call write_memo with key=\"config/port\" and value=\"8080\". Next turn, call read_memo(\"config/port\") to recall it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(EMBEDDING_DIM, 384);
        assert_eq!(DEFAULT_TEMPERATURE, 0.7);
        assert_eq!(DEFAULT_MAX_TOKENS, 40000);
        assert!((BUDGET_PRIORITY_WEIGHT - 0.6).abs() < f32::EPSILON);
        assert!((DEPTH_PRIORITY_WEIGHT - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn test_default_config_constants() {
        assert!((DEFAULT_L1_CONFIDENCE - 0.5).abs() < f32::EPSILON);
        assert!((DEFAULT_SEMANTIC_THRESHOLD - (-0.6)).abs() < f32::EPSILON);
        assert_eq!(DEFAULT_RUNTIME_BUDGET, 10_000);
        assert_eq!(DEFAULT_MAX_DEPTH, 5);
        assert_eq!(DEFAULT_MAX_AGENTS, 10);
    }

    #[test]
    fn test_timeout_constants() {
        assert_eq!(DEFAULT_ADMISSION_TIMEOUT_MS, 100);
        assert_eq!(DEFAULT_SUSPEND_TIMEOUT_MS, 50);
    }

    #[test]
    fn test_l2_constants() {
        assert_eq!(MAX_CONSECUTIVE_FAILURES, 5);
        assert_eq!(L2_JUDGE_MAX_TOKENS, 500);
        assert!((L2_JUDGE_TEMPERATURE - 0.3).abs() < f64::EPSILON);
        assert!((L2_OVERRIDE_BOOST - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_l1_weight_constants() {
        assert!((L1_TASK_WEIGHT - 0.35).abs() < f32::EPSILON);
        assert!((L1_ROLE_WEIGHT - 0.25).abs() < f32::EPSILON);
        assert!((L1_VALUE_WEIGHT - 0.25).abs() < f32::EPSILON);
        assert!((L1_RECENCY_WEIGHT - 0.15).abs() < f32::EPSILON);
        let sum = L1_TASK_WEIGHT + L1_ROLE_WEIGHT + L1_VALUE_WEIGHT + L1_RECENCY_WEIGHT;
        assert!((sum - 1.0).abs() < f32::EPSILON, "L1 weights should sum to 1.0");
    }

    #[test]
    fn test_other_constants() {
        assert!((BUDGET_ANOMALY_RATIO - 0.8).abs() < f64::EPSILON);
        assert_eq!(MAX_CHAIN_LENGTH, 20);
        assert_eq!(SIDEBAR_WIDTH, 28);
    }
}
