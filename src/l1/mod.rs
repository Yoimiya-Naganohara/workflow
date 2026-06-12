pub mod arbitration;
pub mod classifier;

use crate::core::simd::cosine_similarity_768;
use crate::core::types::EMBEDDING_DIM;
use crate::core::types::{ExperienceEntry, SpawnRejection};
pub use arbitration::L1ArbitrationResult;
pub use classifier::{L1ValueClassifier, ValueAssessment};

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

    pub fn retrieve(&self, query_embedding: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(&ExperienceEntry, f32)> {
        let mut scored: Vec<(&ExperienceEntry, f32)> = self
            .experiences
            .iter()
            .map(|entry| {
                let sim = cosine_similarity_768(query_embedding, &entry.embedding);
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
    ) -> Result<L1Assessment, SpawnRejection> {
        if self.experiences.is_empty() {
            // Presumed guilty: no experience = insufficient evidence.
            return Err(SpawnRejection::L1Rejected {
                reason: "No experience available — presumed guilty".to_string(),
                confidence: 0.0,
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
    ) -> Result<L1Assessment, SpawnRejection>;
    fn add_experience(&mut self, entry: ExperienceEntry);
    fn experience_count(&self) -> usize;

    /// Flush persistent storage to disk (no-op for in-memory retrievers).
    fn flush(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    /// Number of bedrock (persistent) entries.
    fn bedrock_count(&self) -> usize {
        0
    }
    /// Number of fluid (volatile) entries.
    fn fluid_count(&self) -> usize {
        0
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
    ) -> Result<L1Assessment, SpawnRejection> {
        self.check_confidence(task_embedding, role_embedding)
    }

    fn add_experience(&mut self, entry: ExperienceEntry) {
        self.add_experience(entry)
    }

    fn experience_count(&self) -> usize {
        self.experience_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_experience(embedding: [f32; EMBEDDING_DIM], weight: f32, tools: u64) -> ExperienceEntry {
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

        let result = retriever.check_confidence(&query, &query);
        assert!(result.is_ok());
    }
}
