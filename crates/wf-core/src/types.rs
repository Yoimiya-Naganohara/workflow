use serde::{Deserialize, Serialize};

use crate::EMBEDDING_DIM;

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

impl Default for ChildAgentConfig {
    fn default() -> Self {
        Self {
            agent_id: [0; 16],
            task_id: [0; 16],
            allocated_budget: 0,
            allowed_tools: !0,
            role_template_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtaskDef {
    pub id: String,
    pub role: String,
    pub goal: String,
    #[serde(default)]
    pub depend_on: Vec<String>,
    #[serde(default)]
    pub auto_confirm: bool,
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

impl Serialize for ExperienceEntry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ExperienceEntry", 9)?;
        st.serialize_field("embedding", &self.embedding[..])?;
        st.serialize_field("applicability_vector", &self.applicability_vector[..])?;
        st.serialize_field("tool_bitmap", &self.tool_bitmap)?;
        st.serialize_field("role_template_id", &self.role_template_id)?;
        st.serialize_field("weight", &self.weight)?;
        st.serialize_field("domain_version", &self.domain_version)?;
        st.serialize_field("timestamp", &self.timestamp)?;
        st.serialize_field("l2_override_weight", &self.l2_override_weight)?;
        st.serialize_field("l2_override_created_at", &self.l2_override_created_at)?;
        st.end()
    }
}

impl<'de> Deserialize<'de> for ExperienceEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};
        use std::fmt;

        struct EntryVisitor;

        impl<'de> Visitor<'de> for EntryVisitor {
            type Value = ExperienceEntry;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("struct ExperienceEntry")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let embedding: Vec<f32> = seq
                    .next_element::<Vec<f32>>()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;
                let applicability_vector: Vec<f32> = seq
                    .next_element::<Vec<f32>>()?
                    .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                let tool_bitmap: u64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                let role_template_id: Option<u32> = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                let weight: f32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(4, &self))?;
                let domain_version: u64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(5, &self))?;
                let timestamp: u64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(6, &self))?;
                let l2_override_weight: f32 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(7, &self))?;
                let l2_override_created_at: u64 = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(8, &self))?;

                let emb_len = embedding.len();
                let embedding: [f32; EMBEDDING_DIM] = embedding.try_into().map_err(|_| {
                    de::Error::custom(format!(
                        "expected embedding of length {}, got {}",
                        EMBEDDING_DIM, emb_len
                    ))
                })?;
                let app_len = applicability_vector.len();
                let applicability_vector: [f32; 128] =
                    applicability_vector.try_into().map_err(|_| {
                        de::Error::custom(format!(
                            "expected applicability_vector of length 128, got {}",
                            app_len
                        ))
                    })?;

                Ok(ExperienceEntry {
                    embedding,
                    applicability_vector,
                    tool_bitmap,
                    role_template_id,
                    weight,
                    domain_version,
                    timestamp,
                    l2_override_weight,
                    l2_override_created_at,
                })
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut embedding: Option<Vec<f32>> = None;
                let mut applicability_vector: Option<Vec<f32>> = None;
                let mut tool_bitmap: Option<u64> = None;
                let mut role_template_id: Option<Option<u32>> = None;
                let mut weight: Option<f32> = None;
                let mut domain_version: Option<u64> = None;
                let mut timestamp: Option<u64> = None;
                let mut l2_override_weight: Option<f32> = None;
                let mut l2_override_created_at: Option<u64> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "embedding" => embedding = Some(map.next_value()?),
                        "applicability_vector" => applicability_vector = Some(map.next_value()?),
                        "tool_bitmap" => tool_bitmap = Some(map.next_value()?),
                        "role_template_id" => role_template_id = Some(map.next_value()?),
                        "weight" => weight = Some(map.next_value()?),
                        "domain_version" => domain_version = Some(map.next_value()?),
                        "timestamp" => timestamp = Some(map.next_value()?),
                        "l2_override_weight" => l2_override_weight = Some(map.next_value()?),
                        "l2_override_created_at" => {
                            l2_override_created_at = Some(map.next_value()?)
                        }
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                let embedding: [f32; EMBEDDING_DIM] = embedding
                    .ok_or_else(|| de::Error::missing_field("embedding"))?
                    .try_into()
                    .map_err(|_| de::Error::custom("embedding length mismatch"))?;
                let applicability_vector: [f32; 128] = applicability_vector
                    .ok_or_else(|| de::Error::missing_field("applicability_vector"))?
                    .try_into()
                    .map_err(|_| de::Error::custom("applicability_vector length mismatch"))?;

                Ok(ExperienceEntry {
                    embedding,
                    applicability_vector,
                    tool_bitmap: tool_bitmap
                        .ok_or_else(|| de::Error::missing_field("tool_bitmap"))?,
                    role_template_id: role_template_id
                        .ok_or_else(|| de::Error::missing_field("role_template_id"))?,
                    weight: weight.ok_or_else(|| de::Error::missing_field("weight"))?,
                    domain_version: domain_version
                        .ok_or_else(|| de::Error::missing_field("domain_version"))?,
                    timestamp: timestamp.ok_or_else(|| de::Error::missing_field("timestamp"))?,
                    l2_override_weight: l2_override_weight
                        .ok_or_else(|| de::Error::missing_field("l2_override_weight"))?,
                    l2_override_created_at: l2_override_created_at
                        .ok_or_else(|| de::Error::missing_field("l2_override_created_at"))?,
                })
            }
        }

        d.deserialize_struct(
            "ExperienceEntry",
            &[
                "embedding",
                "applicability_vector",
                "tool_bitmap",
                "role_template_id",
                "weight",
                "domain_version",
                "timestamp",
                "l2_override_weight",
                "l2_override_created_at",
            ],
            EntryVisitor,
        )
    }
}

// ============================================================================
//  Chat / UI domain types (moved from tui::state)
// ============================================================================

/// Status of a chat message in the stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageStatus {
    Thinking,
    Streaming,
    Completed,
    Error,
}

/// Role of a chat message sender.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

/// A single chunk of streaming output, preserving original token order.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolCall { name: String, args: String },
}

/// A single chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    #[serde(default)]
    pub chunks: Vec<StreamChunk>,
    pub timestamp: String,
    pub status: MessageStatus,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        Self {
            role: MessageRole::System,
            content: content.into(),
            reasoning: String::new(),
            chunks: vec![],
            timestamp: now,
            status: MessageStatus::Completed,
        }
    }
}

/// A selected model for chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedModel {
    pub provider_id: String,
    pub model_id: String,
    pub provider_name: String,
    pub model_name: String,
}

// ============================================================================
//  RoleTemplate — per-role system prompt template
// ============================================================================

#[derive(Clone, Serialize, Deserialize)]
pub struct RoleTemplate {
    pub role: String,
    pub label: String,
    pub system_prompt: String,
    pub template_id: u32,
    #[serde(default = "default_min_experiences")]
    pub min_experiences: usize,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(with = "opt_big_array_384")]
    pub embedding: Option<[f32; EMBEDDING_DIM]>,
}

impl Default for RoleTemplate {
    fn default() -> Self {
        Self {
            role: String::new(),
            label: String::new(),
            system_prompt: String::new(),
            template_id: 0,
            min_experiences: default_min_experiences(),
            version: 0,
            created_at: 0,
            updated_at: 0,
            embedding: None,
        }
    }
}

fn default_min_experiences() -> usize {
    5
}

/// Serde helpers for `Option<[f32; EMBEDDING_DIM]>`.
pub mod opt_big_array_384 {
    use crate::EMBEDDING_DIM;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(
        val: &Option<[f32; EMBEDDING_DIM]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match val {
            Some(arr) => arr.as_slice().serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<[f32; EMBEDDING_DIM]>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<Vec<f32>> = Option::deserialize(deserializer)?;
        match opt {
            Some(v) => {
                let arr: [f32; EMBEDDING_DIM] = v.try_into().map_err(|e: Vec<f32>| {
                    serde::de::Error::custom(format!(
                        "expected {} floats, got {}",
                        EMBEDDING_DIM,
                        e.len()
                    ))
                })?;
                Ok(Some(arr))
            }
            None => Ok(None),
        }
    }
}

/// Status of an agent in the diagnostic tree.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Suspended,
    Completed,
    Failed,
}

/// Entry in the agent diagnostic tree.
#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub budget: u64,
}

// ============================================================================
//  ReasoningOption — model reasoning configuration
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ReasoningOption {
    #[serde(rename = "toggle")]
    Toggle,
    #[serde(rename = "effort")]
    Effort {
        #[serde(default)]
        values: Vec<String>,
    },
    #[serde(rename = "budget_tokens")]
    BudgetTokens {
        #[serde(default)]
        values: Vec<String>,
    },
    #[serde(other)]
    Unknown,
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

impl Default for ChatMessage {
    fn default() -> Self {
        Self {
            role: MessageRole::System,
            content: String::new(),
            reasoning: String::new(),
            chunks: vec![],
            timestamp: String::new(),
            status: MessageStatus::Completed,
        }
    }
}
