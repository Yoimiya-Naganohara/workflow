//! Simple in-memory experience retriever — fallback when no persistent pool is configured.
//!
//! This is the minimal [`ExperienceRetrieval`] implementation used when
//! [`DualTrackMemory`] is not injected. It stores experiences in a `Vec`
//! and performs linear search with weighted cosine similarity.

use crate::core::simd::cosine_similarity_768;
use crate::core::types::{EMBEDDING_DIM, ExperienceEntry, SpawnRejection};
use crate::l1::{ExperienceRetrieval, L1Assessment};

/// Simple in-memory experience store.
pub struct SimpleRetriever {
    experiences: Vec<ExperienceEntry>,
    confidence_threshold: f32,
}

impl SimpleRetriever {
    pub fn new(confidence_threshold: f32) -> Self {
        Self {
            experiences: Vec::new(),
            confidence_threshold,
        }
    }
}

impl ExperienceRetrieval for SimpleRetriever {
    fn retrieve(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        let mut scored: Vec<(&ExperienceEntry, f32)> = self
            .experiences
            .iter()
            .map(|entry| {
                let sim = cosine_similarity_768(query, &entry.embedding);
                (entry, sim * entry.weight)
            })
            .collect();

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);
        scored.into_iter().map(|(e, s)| (e.clone(), s)).collect()
    }

    fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
    ) -> Result<L1Assessment, SpawnRejection> {
        if self.experiences.is_empty() {
            // Cold start: allow with low confidence.
            return Ok(L1Assessment {
                confidence: 0.1,
                recommended_tools: 0,
                matched_experiences: 0,
            });
        }

        let task_matches = self.retrieve(task_embedding, 5);
        let role_matches = self.retrieve(role_embedding, 5);

        let task_score = task_matches.first().map(|(_, s)| *s).unwrap_or(0.0);
        let role_score = role_matches.first().map(|(_, s)| *s).unwrap_or(0.0);

        let combined = (task_score + role_score) / 2.0;

        if combined >= self.confidence_threshold {
            let recommended_tools = infer_tools_from_matches(&task_matches);
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

    fn add_experience(&mut self, entry: ExperienceEntry) {
        self.experiences.push(entry);
    }

    fn experience_count(&self) -> usize {
        self.experiences.len()
    }
}

fn infer_tools_from_matches(matches: &[(ExperienceEntry, f32)]) -> u64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_experience(weight: f32, tools: u64) -> ExperienceEntry {
        ExperienceEntry {
            embedding: [0.0f32; EMBEDDING_DIM],
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
        let mut retriever = SimpleRetriever::new(0.5);
        let mut e1 = [0.0f32; EMBEDDING_DIM];
        e1[0] = 1.0;
        let mut e2 = [0.0f32; EMBEDDING_DIM];
        e2[0] = 0.8;

        retriever.add_experience(ExperienceEntry {
            embedding: e1,
            tool_bitmap: 0b101,
            ..make_experience(1.0, 0b101)
        });
        retriever.add_experience(ExperienceEntry {
            embedding: e2,
            tool_bitmap: 0b010,
            ..make_experience(0.9, 0b010)
        });

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;
        let results = retriever.retrieve(&query, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.tool_bitmap, 0b101);
    }

    #[test]
    fn test_confidence_threshold() {
        let mut retriever = SimpleRetriever::new(0.8);
        let mut e = [0.0f32; EMBEDDING_DIM];
        e[0] = 1.0;
        retriever.add_experience(ExperienceEntry {
            embedding: e,
            weight: 1.0,
            ..make_experience(1.0, 0)
        });

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;

        let result = retriever.check_confidence(&query, &query);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cold_start_allows_spawn() {
        let retriever = SimpleRetriever::new(0.5);
        let result = retriever.check_confidence(&[0.0; EMBEDDING_DIM], &[0.0; EMBEDDING_DIM]);
        assert!(result.is_ok(), "empty pool should allow cold-start spawn");
        let assessment = result.unwrap();
        assert!(assessment.confidence < 0.2);
        assert_eq!(assessment.recommended_tools, 0);
    }
}
