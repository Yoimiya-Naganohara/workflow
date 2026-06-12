//! Runtime configuration and role templates.

use serde::{Deserialize, Serialize};

// ============================================================================
//  Runtime Configuration
// ============================================================================

/// Configuration passed to [`AgentRuntime`](super::runtime::AgentRuntime) as guidance.
///
/// Individual layers may override these values if their injected
/// implementations have their own configuration.
#[derive(Clone)]
pub struct AgentRuntimeConfig {
    pub max_concurrent_agents: usize,
    pub admission_timeout_ms: u64,
    pub max_depth: u32,
    pub initial_budget: u64,
    pub l1_confidence_threshold: f32,
    pub semantic_conflict_threshold: f32,
    pub suspend_timeout_ms: u64,
    /// Path to the bedrock experience pool mmap file.
    /// Defaults to `~/.workflow/experience_a.bin`.
    pub bedrock_path: Option<std::path::PathBuf>,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: crate::core::types::DEFAULT_MAX_AGENTS,
            admission_timeout_ms: crate::core::types::DEFAULT_ADMISSION_TIMEOUT_MS,
            max_depth: crate::core::types::DEFAULT_MAX_DEPTH,
            initial_budget: crate::core::types::DEFAULT_RUNTIME_BUDGET,
            l1_confidence_threshold: crate::core::types::DEFAULT_L1_CONFIDENCE,
            semantic_conflict_threshold: crate::core::types::DEFAULT_SEMANTIC_THRESHOLD,
            suspend_timeout_ms: crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS,
            bedrock_path: None,
        }
    }
}

// ============================================================================
//  RoleTemplate
// ============================================================================

#[derive(Clone, Serialize, Deserialize)]
pub struct RoleTemplate {
    pub role: String,
    pub label: String,
    pub system_prompt: String,
    pub template_id: u32,
    #[serde(with = "opt_big_array_384")]
    pub embedding: Option<[f32; crate::core::types::EMBEDDING_DIM]>,
}

/// Serde helpers for `Option<[f32; EMBEDDING_DIM]>`.
///
/// Serializes as `null` or a JSON array of floats.
mod opt_big_array_384 {
    use crate::core::types::EMBEDDING_DIM;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(val: &Option<[f32; EMBEDDING_DIM]>, serializer: S) -> Result<S::Ok, S::Error>
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
                if v.len() != EMBEDDING_DIM {
                    return Err(serde::de::Error::custom(format!(
                        "expected {} elements, got {}",
                        EMBEDDING_DIM,
                        v.len()
                    )));
                }
                let mut arr = [0.0f32; EMBEDDING_DIM];
                for (i, val) in v.into_iter().enumerate() {
                    arr[i] = val;
                }
                Ok(Some(arr))
            }
            None => Ok(None),
        }
    }
}
