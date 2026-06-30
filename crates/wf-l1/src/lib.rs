use wf_core::EMBEDDING_DIM;
use wf_core::simd::cosine_similarity_384;
use wf_core::{ExperienceEntry, SpawnRejection};

pub struct L1Retriever {
    experiences: Vec<ExperienceEntry>,
    confidence_threshold: f32,
}

impl L1Retriever {
    pub fn new(confidence_threshold: f32) -> Self {
        Self {
            experiences: Vec::new(),
            confidence_threshold,
        }
    }

    pub fn experience_count(&self) -> usize {
        self.experiences.len()
    }

    pub fn add_experience(&mut self, entry: ExperienceEntry) {
        self.experiences.push(entry);
    }

    pub fn retrieve(
        &self,
        query_embedding: &[f32; EMBEDDING_DIM],
        k: usize,
    ) -> Vec<(&ExperienceEntry, f32)> {
        let mut scored: Vec<(&ExperienceEntry, f32)> = self
            .experiences
            .iter()
            .map(|entry| {
                let sim = cosine_similarity_384(query_embedding, &entry.embedding);
                (entry, sim * entry.weight)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        scored
    }

    pub fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
        _: Option<u32>,
        _: Option<usize>,
    ) -> Result<L1Assessment, SpawnRejection> {
        if self.experiences.is_empty() {
            return Ok(L1Assessment {
                confidence: 0.6,
                recommended_tools: !0,
                matched_experiences: 0,
            });
        }

        if self.experiences.len() < 5 {
            return Ok(L1Assessment {
                confidence: 0.6,
                recommended_tools: !0,
                matched_experiences: self.experiences.len(),
            });
        }

        let task_matches = self.retrieve(task_embedding, 5);
        let role_matches = self.retrieve(role_embedding, 5);

        let task_score = task_matches.first().map(|(_, s)| *s).unwrap_or(0.0);
        let role_score = role_matches.first().map(|(_, s)| *s).unwrap_or(0.0);

        let combined = (task_score + role_score) / 2.0;

        if combined >= self.confidence_threshold {
            let recommended_tools = self.infer_tools(&task_matches);
            Ok(L1Assessment {
                confidence: combined,
                recommended_tools,
                matched_experiences: task_matches.len(),
            })
        } else {
            Err(SpawnRejection::L1Rejected {
                reason: "Low confidence".to_string(),
                confidence: combined,
            })
        }
    }

    fn infer_tools(&self, matches: &[(&ExperienceEntry, f32)]) -> u64 {
        let mut tool_votes = [0u32; 64];
        for (entry, score) in matches {
            let bitmap = entry.tool_bitmap;
            for (bit, vote) in tool_votes.iter_mut().enumerate() {
                if (bitmap >> bit) & 1 == 1 {
                    *vote += (score * 100.0) as u32;
                }
            }
        }

        let mut result = 0u64;
        for (bit, &vote) in tool_votes.iter().enumerate() {
            if vote > 50 {
                result |= 1 << bit;
            }
        }
        result
    }

    pub fn len(&self) -> usize {
        self.experiences.len()
    }

    pub fn is_empty(&self) -> bool {
        self.experiences.is_empty()
    }
}

pub struct L1Assessment {
    pub confidence: f32,
    pub recommended_tools: u64,
    pub matched_experiences: usize,
}

/// L1: Experience-driven confidence assessment.
pub trait ExperienceRetrieval: Send + Sync {
    fn retrieve(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)>;
    fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
        role_template_id: Option<u32>,
        role_min_experiences: Option<usize>,
    ) -> Result<L1Assessment, SpawnRejection>;
    fn add_experience(&mut self, entry: ExperienceEntry);
    fn experience_count(&self) -> usize;

    /// Clear all experiences (no-op for in-memory retrievers).
    fn clear(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Flush persistent storage to disk (no-op for in-memory retrievers).
    fn flush(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Number of bedrock (persistent) entries.
    fn bedrock_count(&self) -> usize {
        0
    }

    /// Export all entries (for serialization / backup).
    fn export_entries(&self) -> Vec<ExperienceEntry> {
        Vec::new()
    }

    /// Import entries from an external source.
    fn import_entries(&mut self, entries: Vec<ExperienceEntry>) {
        for e in entries {
            self.add_experience(e);
        }
    }
    /// Number of fluid (volatile) entries.
    fn fluid_count(&self) -> usize {
        0
    }

    /// Consolidate fluid experiences to bedrock (cluster + promote).
    /// No-op for single-tier retrievers.
    fn consolidate(&mut self) {
        // Default: no-op (in-memory retrievers don't have dual-track)
    }

    /// Search by role ID — only experiences with matching role_template_id.
    fn search_by_role(
        &self,
        query: &[f32; EMBEDDING_DIM],
        _: u32,
        k: usize,
    ) -> Vec<(ExperienceEntry, f32)> {
        // Default: fall back to regular search (no role filter).
        self.retrieve(query, k)
    }

    /// Collect all experiences belonging to a specific role.
    fn get_experiences_by_role(&self, _: u32) -> Vec<ExperienceEntry> {
        Vec::new()
    }
}

impl ExperienceRetrieval for L1Retriever {
    fn retrieve(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        self.retrieve(query, k)
            .into_iter()
            .map(|(e, s)| (e.clone(), s))
            .collect()
    }

    fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
        role_template_id: Option<u32>,
        role_min_experiences: Option<usize>,
    ) -> Result<L1Assessment, SpawnRejection> {
        self.check_confidence(
            task_embedding,
            role_embedding,
            role_template_id,
            role_min_experiences,
        )
    }

    fn add_experience(&mut self, entry: ExperienceEntry) {
        self.add_experience(entry)
    }

    fn experience_count(&self) -> usize {
        self.experience_count()
    }

    fn export_entries(&self) -> Vec<ExperienceEntry> {
        self.experiences.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_experience(
        embedding: [f32; EMBEDDING_DIM],
        weight: f32,
        tools: u64,
    ) -> ExperienceEntry {
        ExperienceEntry {
            embedding,
            applicability_vector: [0.0f32; 128],
            tool_bitmap: tools,
            role_template_id: None,
            weight,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        }
    }

    #[test]
    fn test_retrieve_basic() {
        let mut retriever = L1Retriever::new(0.5);
        let mut e1 = [0.0f32; EMBEDDING_DIM];
        e1[0] = 1.0;
        let mut e2 = [0.0f32; EMBEDDING_DIM];
        e2[0] = 0.8;

        retriever.add_experience(make_experience(e1, 1.0, 0b101));
        retriever.add_experience(make_experience(e2, 0.9, 0b010));

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;
        let results = retriever.retrieve(&query, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.tool_bitmap, 0b101);
    }

    #[test]
    fn test_confidence_threshold() {
        let mut retriever = L1Retriever::new(0.8);
        let mut e = [0.0f32; EMBEDDING_DIM];
        e[0] = 1.0;
        retriever.add_experience(make_experience(e, 1.0, 0));

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;

        let result = retriever.check_confidence(&query, &query, None, None);
        assert!(result.is_ok());
    }
}
use smallvec::SmallVec;
use wf_core::AgentId;
use wf_core::conflict::{ConflictManifest, ConflictType};

pub struct L1Arbitrator {
    semantic_threshold: f32,
}

impl L1Arbitrator {
    pub fn new(semantic_threshold: f32) -> Self {
        Self { semantic_threshold }
    }

    pub fn detect_semantic_conflict(
        &self,
        embedding_a: &[f32; EMBEDDING_DIM],
        embedding_b: &[f32; EMBEDDING_DIM],
    ) -> bool {
        let sim = cosine_similarity_384(embedding_a, embedding_b);
        sim < self.semantic_threshold
    }

    pub fn create_conflict_manifest(
        &self,
        agent_a: AgentId,
        agent_b: AgentId,
        embedding_a: [f32; EMBEDDING_DIM],
        embedding_b: [f32; EMBEDDING_DIM],
        trace_id: [u8; 16],
    ) -> ConflictManifest {
        let sim = cosine_similarity_384(&embedding_a, &embedding_b);

        // Use agent_id bytes as deterministic tiebreaker when embeddings are
        // near-identical (sim > 0.99) to avoid artificially favoring one agent.
        let (priority_a, priority_b) = if sim > 0.99 {
            if agent_a <= agent_b {
                (1.0, 0.0)
            } else {
                (0.0, 1.0)
            }
        } else {
            (1.0 - sim, sim)
        };

        ConflictManifest {
            conflict_id: rand::random(),
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_slice(&[agent_a, agent_b]),
            trace_id,
            context_embeddings: SmallVec::from_slice(&[embedding_a, embedding_b]),
            dynamic_priority_scores: SmallVec::from_slice(&[priority_a, priority_b]),
        }
    }

    pub fn arbitrate_by_priority(&self, manifest: &ConflictManifest) -> L1ArbitrationResult {
        if manifest.dynamic_priority_scores.len() < 2 {
            return L1ArbitrationResult::NoConflict;
        }

        let score_a = manifest.dynamic_priority_scores[0];
        let score_b = manifest.dynamic_priority_scores[1];

        if score_a > score_b {
            L1ArbitrationResult::Override {
                winner: manifest.contending_agents[0],
                loser: manifest.contending_agents[1],
            }
        } else if score_b > score_a {
            L1ArbitrationResult::Override {
                winner: manifest.contending_agents[1],
                loser: manifest.contending_agents[0],
            }
        } else {
            L1ArbitrationResult::RequiresL2
        }
    }
}

pub enum L1ArbitrationResult {
    NoConflict,
    Override { winner: AgentId, loser: AgentId },
    RequiresL2,
}

#[cfg(test)]
mod arbitration_tests {
    use super::*;
    use wf_core::EMBEDDING_DIM;

    #[test]
    fn test_detect_semantic_conflict() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = -1.0;

        assert!(arbitrator.detect_semantic_conflict(&a, &b));
    }

    #[test]
    fn test_no_semantic_conflict() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 1.0;

        assert!(!arbitrator.detect_semantic_conflict(&a, &b));
    }

    #[test]
    fn test_arbitrate_by_priority() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let manifest = ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_slice(&[[1u8; 16], [2u8; 16]]),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::from_slice(&[
                [0.0f32; EMBEDDING_DIM],
                [0.0f32; EMBEDDING_DIM],
            ]),
            dynamic_priority_scores: SmallVec::from_slice(&[0.8, 0.3]),
        };

        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, loser } => {
                assert_eq!(winner, [1u8; 16]);
                assert_eq!(loser, [2u8; 16]);
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_identical_embeddings_tiebreaker_lower_id_wins() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 1.0;

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // Tiebreaker: lower agent_id bytes gets higher priority
        // agent_a = [1; 16] < agent_b = [2; 16], so agent_a should win
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                assert_eq!(
                    winner, [1u8; 16],
                    "lower agent_id bytes should win tiebreaker"
                );
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_complement_formula_with_non_collinear_embeddings() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 0.8;
        b[1] = 0.6; // norm = 1.0, different direction from a

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // sim = dot(a,b) / (|a| * |b|) = 0.8 / (1.0 * 1.0) = 0.8 (< 0.99, no tiebreaker)
        // priority_a = 1.0 - 0.8 = 0.2, priority_b = 0.8
        // Agent B wins from the complement formula
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                assert_eq!(
                    winner, [2u8; 16],
                    "agent B has higher priority from complement formula"
                );
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_arbitrate_empty_agents() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let manifest = ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::new(),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::new(),
            dynamic_priority_scores: SmallVec::new(),
        };

        let result = arbitrator.arbitrate_by_priority(&manifest);
        assert!(matches!(result, L1ArbitrationResult::NoConflict));
    }
}
pub struct L1ValueClassifier {
    keywords: Vec<String>,
}

impl L1ValueClassifier {
    pub fn new(keywords: Vec<String>) -> Self {
        Self { keywords }
    }

    pub fn classify(&self, text: &str) -> ValueAssessment {
        let text_lower = text.to_lowercase();
        let mut jargon_count = 0;

        for keyword in &self.keywords {
            if text_lower.contains(&keyword.to_lowercase()) {
                jargon_count += 1;
            }
        }

        let is_jargon = jargon_count > 2;
        let probability = if is_jargon { 0.3 } else { 0.9 };

        ValueAssessment {
            probability,
            is_jargon,
            jargon_count,
        }
    }
}

pub struct ValueAssessment {
    pub probability: f32,
    pub is_jargon: bool,
    pub jargon_count: u32,
}

#[cfg(test)]
mod classifier_tests {
    use super::*;

    #[test]
    fn test_value_classifier() {
        let classifier = L1ValueClassifier::new(vec![
            "urgent".to_string(),
            "critical".to_string(),
            "emergency".to_string(),
        ]);

        let assessment = classifier.classify("This is an urgent critical emergency");
        assert!(assessment.is_jargon);

        let assessment = classifier.classify("This is a normal task");
        assert!(!assessment.is_jargon);
    }
}
