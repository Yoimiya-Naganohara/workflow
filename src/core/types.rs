pub use crate::core::constants::*;

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

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpawnRequest ──

    fn sample_request() -> SpawnRequest {
        SpawnRequest {
            trace_id: [0x01; 16],
            span_id: 42,
            parent_span_id: 7,
            task_description_embedding: [0.1; EMBEDDING_DIM],
            role_description_embedding: [0.2; EMBEDDING_DIM],
            value_statement_embedding: [0.3; EMBEDDING_DIM],
            requested_budget: 5000,
            current_depth: 2,
            responsibility_chain: vec![[0xAA; 16], [0xBB; 16]],
            raw_text_ref: None,
        }
    }

    #[test]
    fn test_spawn_request_fields() {
        let req = sample_request();
        assert_eq!(req.trace_id, [0x01; 16]);
        assert_eq!(req.span_id, 42);
        assert_eq!(req.parent_span_id, 7);
        assert_eq!(req.task_description_embedding[0], 0.1);
        assert_eq!(req.role_description_embedding[0], 0.2);
        assert_eq!(req.value_statement_embedding[0], 0.3);
        assert_eq!(req.requested_budget, 5000);
        assert_eq!(req.current_depth, 2);
        assert_eq!(req.responsibility_chain.len(), 2);
        assert!(req.raw_text_ref.is_none());
    }

    #[test]
    fn test_spawn_request_empty_chain() {
        let req = SpawnRequest {
            responsibility_chain: vec![],
            ..sample_request()
        };
        assert!(req.responsibility_chain.is_empty());
    }

    #[test]
    fn test_spawn_request_raw_text_ref() {
        let req = SpawnRequest {
            raw_text_ref: Some(RawTextRef {
                offset: 100,
                length: 50,
                source_hash: [0xDE; 32],
            }),
            ..sample_request()
        };
        let r = req.raw_text_ref.unwrap();
        assert_eq!(r.offset, 100);
        assert_eq!(r.length, 50);
        assert_eq!(r.source_hash, [0xDE; 32]);
    }

    // ── SpawnDecision ──

    #[test]
    fn test_spawn_decision_approved() {
        let config = ChildAgentConfig {
            agent_id: [0xCC; 16],
            task_id: [0xDD; 16],
            allocated_budget: 3000,
            allowed_tools: 0b1111,
            role_template_id: Some(1),
        };
        let decision = SpawnDecision::Approved(config);
        match decision {
            SpawnDecision::Approved(c) => {
                assert_eq!(c.agent_id, [0xCC; 16]);
                assert_eq!(c.allocated_budget, 3000);
                assert_eq!(c.allowed_tools, 0b1111);
                assert_eq!(c.role_template_id, Some(1));
            }
            _ => panic!("expected Approved"),
        }
    }

    #[test]
    fn test_spawn_decision_no_role_template() {
        let config = ChildAgentConfig {
            agent_id: [0xEE; 16],
            task_id: [0xFF; 16],
            allocated_budget: 1000,
            allowed_tools: 0,
            role_template_id: None,
        };
        let decision = SpawnDecision::Approved(config);
        match decision {
            SpawnDecision::Approved(c) => assert!(c.role_template_id.is_none()),
            _ => panic!("expected Approved"),
        }
    }

    // ── SpawnRejection formatting ──

    #[test]
    fn test_spawn_rejection_system_overloaded() {
        let err = SpawnRejection::SystemOverloaded;
        assert_eq!(err.to_string(), "system overloaded");
    }

    #[test]
    fn test_spawn_rejection_budget_exhausted() {
        let err = SpawnRejection::BudgetExhausted {
            requested: 500,
            remaining: 100,
        };
        let msg = err.to_string();
        assert!(msg.contains("500"));
        assert!(msg.contains("100"));
    }

    #[test]
    fn test_spawn_rejection_depth_exceeded() {
        let err = SpawnRejection::DepthExceeded { current: 5, max: 3 };
        let msg = err.to_string();
        assert!(msg.contains("5"));
        assert!(msg.contains("3"));
    }

    #[test]
    fn test_spawn_rejection_resource_conflict() {
        let err = SpawnRejection::ResourceConflict {
            tool_id: 3,
            holder: [0x11; 16],
        };
        let msg = err.to_string();
        assert!(msg.contains("3"));
    }

    #[test]
    fn test_spawn_rejection_l1_rejected() {
        let err = SpawnRejection::L1Rejected {
            reason: "low confidence".to_string(),
            confidence: 0.3,
        };
        let msg = err.to_string();
        assert!(msg.contains("low confidence"));
    }

    #[test]
    fn test_spawn_rejection_l2_rejected() {
        let err = SpawnRejection::L2Rejected {
            reason: "value violation".to_string(),
            category: "safety".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("value violation"));
        assert!(msg.contains("safety"));
    }

    #[test]
    fn test_spawn_rejection_l2_collapsed() {
        let err = SpawnRejection::L2Collapsed;
        assert_eq!(err.to_string(), "L2 collapsed");
    }

    // ── ExperienceEntry ──

    fn sample_experience() -> ExperienceEntry {
        ExperienceEntry {
            embedding: [0.5; EMBEDDING_DIM],
            applicability_vector: [0.1; 128],
            tool_bitmap: 0b1010,
            role_template_id: Some(2),
            weight: 1.0,
            domain_version: 3,
            timestamp: 1_000_000,
            l2_override_weight: 1.0,
            l2_override_created_at: 0,
        }
    }

    #[test]
    fn test_experience_entry_fields() {
        let e = sample_experience();
        assert_eq!(e.embedding[0], 0.5);
        assert_eq!(e.embedding.len(), EMBEDDING_DIM);
        assert_eq!(e.applicability_vector.len(), 128);
        assert_eq!(e.applicability_vector[127], 0.1);
        assert_eq!(e.tool_bitmap, 0b1010);
        assert_eq!(e.role_template_id, Some(2));
        assert!((e.weight - 1.0).abs() < f32::EPSILON);
        assert_eq!(e.domain_version, 3);
        assert_eq!(e.timestamp, 1_000_000);
    }

    #[test]
    fn test_experience_entry_no_role_template() {
        let e = ExperienceEntry {
            role_template_id: None,
            ..sample_experience()
        };
        assert!(e.role_template_id.is_none());
    }

    #[test]
    fn test_experience_entry_l2_override() {
        let e = ExperienceEntry {
            l2_override_weight: 2.0,
            l2_override_created_at: 999,
            ..sample_experience()
        };
        assert!((e.l2_override_weight - 2.0).abs() < f32::EPSILON);
        assert_eq!(e.l2_override_created_at, 999);
    }

    // ── RawTextRef ──

    #[test]
    fn test_raw_text_ref() {
        let r = RawTextRef {
            offset: 0,
            length: 100,
            source_hash: [0xAB; 32],
        };
        assert_eq!(r.offset, 0);
        assert_eq!(r.length, 100);
        assert_eq!(r.source_hash, [0xAB; 32]);
    }

    // ── Type aliases ──

    #[test]
    fn test_type_aliases_size() {
        assert_eq!(std::mem::size_of::<TaskId>(), 16);
        assert_eq!(std::mem::size_of::<TraceId>(), 16);
        assert_eq!(std::mem::size_of::<AgentId>(), 16);
        assert_eq!(std::mem::size_of::<SpanId>(), 8);
    }
}
