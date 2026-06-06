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
    pub task_description_embedding: [f32; 768],
    pub role_description_embedding: [f32; 768],
    pub value_statement_embedding: [f32; 768],
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
    pub embedding: [f32; 768],
    pub applicability_vector: [f32; 128],
    pub tool_bitmap: u64,
    pub role_template_id: Option<u32>,
    pub weight: f32,
    pub domain_version: u64,
    pub timestamp: u64,
    pub l2_override_weight: f32,
    pub l2_override_created_at: u64,
}
