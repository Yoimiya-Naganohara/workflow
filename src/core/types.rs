/// Embedding vector dimension (all-MiniLM-L6-v2).
pub const EMBEDDING_DIM: usize = 384;

/// Default LLM temperature for chat agents.
pub const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Default max tokens for chat agents.
pub const DEFAULT_MAX_TOKENS: u64 = 4000;

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

pub type TaskId = [u8; 16];
pub type TraceId = [u8; 16];
pub type SpanId = u64;
pub type AgentId = [u8; 16];

#[derive(Clone)]
pub struct RawTextRef {
    pub offset: u64,
    pub length: u32,
    pub source_hash: [u8; 32],
}

#[derive(Clone)]
pub struct SpawnRequest {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub parent_span_id: SpanId,
    pub task_description_embedding: [f32; EMBEDDING_DIM],
    pub role_description_embedding: [f32; EMBEDDING_DIM],
    pub value_statement_embedding: [f32; EMBEDDING_DIM],
    pub requested_budget: u64,
    pub current_depth: u32,
    pub responsibility_chain: Vec<AgentId>,
    pub raw_text_ref: Option<RawTextRef>,
}

#[derive(Debug, Clone)]
pub struct ChildAgentConfig {
    pub agent_id: AgentId,
    pub task_id: TaskId,
    pub allocated_budget: u64,
    pub allowed_tools: u64,
    pub role_template_id: Option<u32>,
}

pub enum SpawnDecision {
    Approved(ChildAgentConfig),
    Rejected(SpawnRejection),
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnRejection {
    #[error("system overloaded")]
    SystemOverloaded,
    #[error("budget exhausted: requested {requested}, remaining {remaining}")]
    BudgetExhausted { requested: u64, remaining: i64 },
    #[error("depth exceeded: current {current}, max {max}")]
    DepthExceeded { current: u32, max: u32 },
    #[error("resource conflict on tool {tool_id}")]
    ResourceConflict { tool_id: u64, holder: AgentId },
    #[error("L1 rejected: {reason}")]
    L1Rejected { reason: String, confidence: f32 },
    #[error("L2 rejected ({category}): {reason}")]
    L2Rejected { reason: String, category: String },
    #[error("L2 collapsed")]
    L2Collapsed,
}

#[repr(C)]
#[derive(Clone)]
pub struct ExperienceEntry {
    pub embedding: [f32; EMBEDDING_DIM],
    pub applicability_vector: [f32; 128],
    pub tool_bitmap: u64,
    pub role_template_id: Option<u32>,
    pub weight: f32,
    pub domain_version: u64,
    pub timestamp: u64,
    pub l2_override_weight: f32,
    pub l2_override_created_at: u64,
}
