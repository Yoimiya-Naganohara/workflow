//! GoalAnalyzer trait — the single decomposition heuristic.
//!
//! The only implementation is `EmbeddingGoalAnalyzer`: cosine similarity
//! against pre-computed prototype vectors.  No keywords, no if-else chains.
//! Every decision is a dot product.
//!
//! # Data flow
//!
//! ```text
//! Async init (once):            Sync inference (per goal):
//!   embed("developer")  ───┐    goal_embedding × role_prototypes
//!   embed("tester")     ───┤→     → cosine_similarity_384
//!   embed("security")   ───┤     → highest score → role + confidence
//!   embed(vague_phrase) ───┤
//!   embed(domain_phrases) ──┘
//! ```
//!
//! No file I/O, no config loading, no polymorphic pattern registry.
//! The prototypes are computed once at startup from role names and
//! reference phrases, stored in the analyzer, and queried via SIMD.

use std::sync::Mutex;

use crate::core::simd::cosine_similarity_384;
use crate::core::types::EMBEDDING_DIM;

// ============================================================================
//  GoalAnalyzer trait
// ============================================================================

pub trait GoalAnalyzer: Send + Sync {
    fn estimate_domain_count(&self, goal: &str) -> u32;
    fn estimate_ambiguity(&self, goal: &str) -> f32;
    fn estimate_role(&self, goal: &str) -> Option<(String, f32)>;
}

// ============================================================================
//  Reference data — what gets embedded at startup
// ============================================================================

pub(crate) static ROLE_NAMES: &[&str] = &[
    "developer",
    "tester",
    "security_auditor",
    "reviewer",
    "planner",
    "devops",
    "researcher",
    "general_business_analyst",
];

pub(crate) static AMBIGUITY_PHRASE: &str =
    "Make it better, improve this, fix things up, do something";

pub(crate) static DOMAIN_PHRASES: &[(&str, &str)] = &[
    ("backend", "Build server-side API, database, authentication, business logic"),
    ("frontend", "Build UI, client-side, user interface, dashboard, web pages"),
    ("database", "Schema, tables, migrations, data model, query optimization"),
    ("devops", "Deploy, CI/CD, Docker, infrastructure, monitoring, scaling"),
    ("security", "Authentication, authorization, permissions, encryption, audit"),
    ("testing", "Unit tests, integration tests, QA, validation, assertions"),
];

/// Pre-computed reference embeddings.  Built once at runtime init.
#[derive(Debug, Clone)]
pub struct ReferenceEmbeddings {
    pub role_prototypes: Vec<(String, [f32; EMBEDDING_DIM])>,
    pub ambiguity_reference: [f32; EMBEDDING_DIM],
    pub domain_references: Vec<(String, [f32; EMBEDDING_DIM])>,
}

impl ReferenceEmbeddings {
    pub async fn compute(embedder: &dyn crate::llm::EmbeddingService) -> Self {
        let mut role_protos = Vec::with_capacity(ROLE_NAMES.len());
        for role in ROLE_NAMES {
            if let Ok(emb) = embedder.embed(role).await {
                role_protos.push((role.to_string(), emb));
            }
        }

        let ambiguity_ref = embedder
            .embed(AMBIGUITY_PHRASE)
            .await
            .unwrap_or([0.0; EMBEDDING_DIM]);

        let mut domain_refs = Vec::with_capacity(DOMAIN_PHRASES.len());
        for (label, phrase) in DOMAIN_PHRASES {
            if let Ok(emb) = embedder.embed(phrase).await {
                domain_refs.push((label.to_string(), emb));
            }
        }

        Self { role_prototypes: role_protos, ambiguity_reference: ambiguity_ref, domain_references: domain_refs }
    }
}

// ============================================================================
//  EmbeddingGoalAnalyzer — the only production GoalAnalyzer
// ============================================================================

pub struct EmbeddingGoalAnalyzer {
    role_prototypes: Vec<(String, [f32; EMBEDDING_DIM])>,
    ambiguity_reference: [f32; EMBEDDING_DIM],
    domain_references: Vec<(String, [f32; EMBEDDING_DIM])>,
    goal_embedding: Mutex<Option<[f32; EMBEDDING_DIM]>>,
    domain_threshold: f32,
    role_threshold: f32,
}

impl EmbeddingGoalAnalyzer {
    pub fn new(references: ReferenceEmbeddings) -> Self {
        Self {
            role_prototypes: references.role_prototypes,
            ambiguity_reference: references.ambiguity_reference,
            domain_references: references.domain_references,
            goal_embedding: Mutex::new(None),
            domain_threshold: 0.7,
            role_threshold: 0.3,
        }
    }

    pub fn with_goal(references: ReferenceEmbeddings, goal_embedding: [f32; EMBEDDING_DIM]) -> Self {
        let mut s = Self::new(references);
        s.goal_embedding = Mutex::new(Some(goal_embedding));
        s
    }

    pub fn set_goal_embedding(&self, embedding: [f32; EMBEDDING_DIM]) {
        if let Ok(mut g) = self.goal_embedding.lock() {
            *g = Some(embedding);
        }
    }

    pub fn get_goal_embedding(&self) -> Option<[f32; EMBEDDING_DIM]> {
        self.goal_embedding.lock().ok().and_then(|g| *g)
    }
}

impl GoalAnalyzer for EmbeddingGoalAnalyzer {
    fn estimate_domain_count(&self, _goal: &str) -> u32 {
        let goal_emb = match self.goal_embedding.lock().ok().and_then(|g| *g) {
            Some(e) => e,
            None => return 0,
        };
        let c = self.domain_references.iter()
            .filter(|(_, ref_emb)| cosine_similarity_384(&goal_emb, ref_emb) > self.domain_threshold)
            .count() as u32;
        if c > 0 { c } else { 1 }
    }

    fn estimate_ambiguity(&self, _goal: &str) -> f32 {
        match self.goal_embedding.lock().ok().and_then(|g| *g) {
            Some(ref goal_emb) => cosine_similarity_384(goal_emb, &self.ambiguity_reference),
            None => 0.0,
        }
    }

    fn estimate_role(&self, _goal: &str) -> Option<(String, f32)> {
        let goal_emb = self.goal_embedding.lock().ok().and_then(|g| *g)?;
        let mut best: Option<(String, f32)> = None;
        for (role, prot_emb) in &self.role_prototypes {
            let sim = cosine_similarity_384(&goal_emb, prot_emb);
            if sim > self.role_threshold {
                match &best {
                    Some((_, best_sim)) if sim > *best_sim => best = Some((role.clone(), sim)),
                    None => best = Some((role.clone(), sim)),
                    _ => {}
                }
            }
        }
        best.or_else(|| {
            self.role_prototypes.iter()
                .max_by(|a, b| cosine_similarity_384(&goal_emb, &a.1)
                    .partial_cmp(&cosine_similarity_384(&goal_emb, &b.1)).unwrap())
                .map(|(role, prot_emb)| (role.clone(), cosine_similarity_384(&goal_emb, prot_emb)))
        })
    }
}

// ============================================================================
//  MockGoalAnalyzer — deterministic, for tests
// ============================================================================

pub struct MockGoalAnalyzer {
    pub domain_count: u32,
    pub ambiguity: f32,
    pub role: Option<(String, f32)>,
}

impl GoalAnalyzer for MockGoalAnalyzer {
    fn estimate_domain_count(&self, _goal: &str) -> u32 { self.domain_count }
    fn estimate_ambiguity(&self, _goal: &str) -> f32 { self.ambiguity }
    fn estimate_role(&self, _goal: &str) -> Option<(String, f32)> { self.role.clone() }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_goal_analyzer() {
        let a = MockGoalAnalyzer { domain_count: 3, ambiguity: 0.8, role: Some(("tester".into(), 0.95)) };
        assert_eq!(a.estimate_domain_count("anything"), 3);
        assert!((a.estimate_ambiguity("anything") - 0.8).abs() < 1e-6);
        assert_eq!(a.estimate_role("anything"), Some(("tester".into(), 0.95)));
    }

    #[test]
    fn test_goal_analyzer_trait_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EmbeddingGoalAnalyzer>();
        assert_send_sync::<MockGoalAnalyzer>();
    }

    #[test]
    fn test_embedding_analyzer_returns_closest_role_by_cosine_similarity() {
        let mut dev_emb = [0.0f32; EMBEDDING_DIM];
        dev_emb[0] = 1.0;
        let mut test_emb = [0.0f32; EMBEDDING_DIM];
        test_emb[1] = 1.0;
        let protos = ReferenceEmbeddings {
            role_prototypes: vec![("developer".into(), dev_emb), ("tester".into(), test_emb)],
            ambiguity_reference: [0.5; EMBEDDING_DIM],
            domain_references: vec![],
        };
        let mut goal_emb = [0.0f32; EMBEDDING_DIM];
        goal_emb[0] = 1.0;
        let a = EmbeddingGoalAnalyzer::with_goal(protos, goal_emb);
        let role = a.estimate_role("build api");
        assert!(role.is_some());
        assert_eq!(role.unwrap().0, "developer");
    }

    #[test]
    fn test_domain_count_returns_1_when_no_domain_refs() {
        let protos = ReferenceEmbeddings {
            role_prototypes: vec![("dev".into(), [0.5; EMBEDDING_DIM])],
            ambiguity_reference: [0.5; EMBEDDING_DIM],
            domain_references: vec![],
        };
        let a = EmbeddingGoalAnalyzer::with_goal(protos, [0.5; EMBEDDING_DIM]);
        assert_eq!(a.estimate_domain_count("anything"), 1);
    }
}
