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
    /// Minimum experiences for this role before L1 uses similarity.
    /// Below this count, spawns pass unconditionally with full tools.
    #[serde(default = "default_min_experiences")]
    pub min_experiences: usize,
    /// Optimisation version, incremented on each /role optimize.
    #[serde(default)]
    pub version: u32,
    /// Unix timestamp of creation.
    #[serde(default)]
    pub created_at: u64,
    /// Unix timestamp of last update.
    #[serde(default)]
    pub updated_at: u64,
    #[serde(with = "opt_big_array_384")]
    pub embedding: Option<[f32; crate::core::types::EMBEDDING_DIM]>,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── AgentRuntimeConfig ──

    #[test]
    fn test_default_config() {
        let cfg = AgentRuntimeConfig::default();
        assert_eq!(cfg.max_concurrent_agents, crate::core::types::DEFAULT_MAX_AGENTS);
        assert_eq!(
            cfg.admission_timeout_ms,
            crate::core::types::DEFAULT_ADMISSION_TIMEOUT_MS
        );
        assert_eq!(cfg.max_depth, crate::core::types::DEFAULT_MAX_DEPTH);
        assert_eq!(cfg.initial_budget, crate::core::types::DEFAULT_RUNTIME_BUDGET);
        let eps = f32::EPSILON;
        assert!((cfg.l1_confidence_threshold - crate::core::types::DEFAULT_L1_CONFIDENCE).abs() < eps);
        assert!((cfg.semantic_conflict_threshold - crate::core::types::DEFAULT_SEMANTIC_THRESHOLD).abs() < eps);
        assert_eq!(cfg.suspend_timeout_ms, crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS);
        assert!(cfg.bedrock_path.is_none());
    }

    #[test]
    fn test_config_custom_values() {
        let cfg = AgentRuntimeConfig {
            max_concurrent_agents: 5,
            admission_timeout_ms: 200,
            max_depth: 3,
            initial_budget: 20_000,
            l1_confidence_threshold: 0.8,
            semantic_conflict_threshold: -0.3,
            suspend_timeout_ms: 100,
            bedrock_path: Some(std::path::PathBuf::from("/tmp/test.bin")),
        };
        assert_eq!(cfg.max_concurrent_agents, 5);
        assert_eq!(cfg.admission_timeout_ms, 200);
        assert_eq!(cfg.max_depth, 3);
        assert_eq!(cfg.initial_budget, 20_000);
        assert!((cfg.l1_confidence_threshold - 0.8).abs() < f32::EPSILON);
        assert!((cfg.semantic_conflict_threshold - (-0.3)).abs() < f32::EPSILON);
        assert_eq!(cfg.suspend_timeout_ms, 100);
        assert_eq!(cfg.bedrock_path.as_deref(), Some(std::path::Path::new("/tmp/test.bin")));
    }

    // ── RoleTemplate ──

    fn sample_template() -> RoleTemplate {
        RoleTemplate {
            role: "planner".into(),
            label: "Senior Planner".into(),
            system_prompt: "You are a senior planner.".into(),
            template_id: 1,
            min_experiences: 3,
            version: 2,
            created_at: 1000,
            updated_at: 2000,
            embedding: None,
        }
    }

    #[test]
    fn test_role_template_default() {
        let t = RoleTemplate::default();
        assert_eq!(t.role, "");
        assert_eq!(t.label, "");
        assert_eq!(t.system_prompt, "");
        assert_eq!(t.template_id, 0);
        assert_eq!(t.min_experiences, 5);
        assert_eq!(t.version, 0);
        assert_eq!(t.created_at, 0);
        assert_eq!(t.updated_at, 0);
        assert!(t.embedding.is_none());
    }

    #[test]
    fn test_role_template_custom() {
        let t = sample_template();
        assert_eq!(t.role, "planner");
        assert_eq!(t.label, "Senior Planner");
        assert_eq!(t.template_id, 1);
        assert_eq!(t.min_experiences, 3);
        assert_eq!(t.version, 2);
        assert_eq!(t.created_at, 1000);
        assert_eq!(t.updated_at, 2000);
    }

    #[test]
    fn test_role_template_with_embedding() {
        let mut emb = [0.0f32; crate::core::types::EMBEDDING_DIM];
        emb[0] = 0.5;
        emb[42] = -0.3;
        emb[383] = 0.9;
        let t = RoleTemplate {
            embedding: Some(emb),
            ..sample_template()
        };
        let e = t.embedding.unwrap();
        assert!((e[0] - 0.5).abs() < f32::EPSILON);
        assert!((e[42] - (-0.3)).abs() < f32::EPSILON);
        assert!((e[383] - 0.9).abs() < f32::EPSILON);
        assert_eq!(e.len(), crate::core::types::EMBEDDING_DIM);
    }

    #[test]
    fn test_role_template_serde_roundtrip() {
        let t = sample_template();
        let json = serde_json::to_string(&t).unwrap();
        let back: RoleTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, t.role);
        assert_eq!(back.label, t.label);
        assert_eq!(back.system_prompt, t.system_prompt);
        assert_eq!(back.template_id, t.template_id);
        assert_eq!(back.min_experiences, t.min_experiences);
        assert_eq!(back.version, t.version);
        assert_eq!(back.created_at, t.created_at);
        assert_eq!(back.updated_at, t.updated_at);
    }

    #[test]
    fn test_role_template_serde_with_embedding() {
        let mut emb = [0.0f32; crate::core::types::EMBEDDING_DIM];
        emb[0] = 1.0;
        emb[383] = -1.0;
        let t = RoleTemplate {
            embedding: Some(emb),
            ..sample_template()
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: RoleTemplate = serde_json::from_str(&json).unwrap();
        let e = back.embedding.unwrap();
        assert!((e[0] - 1.0).abs() < f32::EPSILON);
        assert!((e[383] - (-1.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_role_template_serde_null_embedding() {
        let t = RoleTemplate {
            embedding: None,
            ..sample_template()
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: RoleTemplate = serde_json::from_str(&json).unwrap();
        assert!(back.embedding.is_none());
    }

    #[test]
    fn test_role_template_serde_wrong_embedding_length() {
        let json = format!(
            r#"{{"role":"test","label":"Test","system_prompt":"...","template_id":0,"min_experiences":5,"version":0,"created_at":0,"updated_at":0,"embedding":{}}}"#,
            serde_json::to_string(&vec![0.0f32; 10]).unwrap()
        );
        let result: Result<RoleTemplate, _> = serde_json::from_str(&json);
        assert!(result.is_err());
    }

    // ── opt_big_array_384 ──

    #[test]
    fn test_opt_big_array_serialize_some() {
        let arr: Option<[f32; crate::core::types::EMBEDDING_DIM]> = Some([0.5; crate::core::types::EMBEDDING_DIM]);
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::new(&mut buf);
        super::opt_big_array_384::serialize(&arr, &mut ser).unwrap();
        let json = String::from_utf8(buf).unwrap();
        assert!(json.starts_with('['));
    }

    #[test]
    fn test_opt_big_array_serialize_none() {
        let arr: Option<[f32; crate::core::types::EMBEDDING_DIM]> = None;
        let mut buf = Vec::new();
        let mut ser = serde_json::Serializer::new(&mut buf);
        super::opt_big_array_384::serialize(&arr, &mut ser).unwrap();
        let json = String::from_utf8(buf).unwrap();
        assert_eq!(json, "null");
    }

    #[test]
    fn test_opt_big_array_deserialize_some() {
        let json = format!("[{}]", "0.5,".repeat(crate::core::types::EMBEDDING_DIM - 1) + "0.5");
        let mut de = serde_json::Deserializer::from_str(&json);
        let arr: Option<[f32; crate::core::types::EMBEDDING_DIM]> =
            super::opt_big_array_384::deserialize(&mut de).unwrap();
        let a = arr.unwrap();
        assert!((a[0] - 0.5).abs() < f32::EPSILON);
        assert!((a[383] - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_opt_big_array_deserialize_none() {
        let mut de = serde_json::Deserializer::from_str("null");
        let arr: Option<[f32; crate::core::types::EMBEDDING_DIM]> =
            super::opt_big_array_384::deserialize(&mut de).unwrap();
        assert!(arr.is_none());
    }

    #[test]
    fn test_opt_big_array_deserialize_wrong_length() {
        let json = "[0.5, 0.5]";
        let mut de = serde_json::Deserializer::from_str(json);
        let result = super::opt_big_array_384::deserialize(&mut de);
        assert!(result.is_err());
    }
}
