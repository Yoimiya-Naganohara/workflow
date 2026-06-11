//! Dual-track memory — stable A-track (bedrock) + volatile B-track (fluid).
//!
//! # A-track (bedrock)
//!
//! Durable, mmap-backed [`ExperiencePool`].  Experiences here are
//! long-term and persist across restarts.  Added via explicit
//! [`DualTrackMemory::promote_to_bedrock`] or automatically during
//! consolidation.
//!
//! # B-track (fluid)
//!
//! In-memory, bounded [`FluidTrack`].  New experiences land here
//! first.  When the fluid track exceeds its capacity, a clustering
//! pass consolidates similar entries into representatives that are
//! promoted to the A-track.
//!
//! # Query merging
//!
//! Searches consult both tracks and return the combined top-k,
//! weighted by each track's credibility factor.

use anyhow::Result;
use tracing::trace;

use crate::core::simd::cosine_similarity_768;
use crate::core::types::{EMBEDDING_DIM, ExperienceEntry, SpawnRejection};
use crate::experience::clustering::ClusterConsolidator;
use crate::experience::pool::ExperiencePool;
use crate::l1::L1Assessment;

// ---------------------------------------------------------------------------
//  FluidTrack
// ---------------------------------------------------------------------------

/// Volatile, in-memory B-track store.
///
/// Entries are stored in a `Vec` with a configurable maximum capacity.
/// When the capacity is exceeded, new pushes evict the oldest entry
/// (FIFO).
pub struct FluidTrack {
    entries: Vec<ExperienceEntry>,
    max_size: usize,
}

impl FluidTrack {
    /// Create a fluid track with the given maximum size.
    ///
    /// Once `max_size` is reached, new pushes will evict the oldest
    /// entry (FIFO).
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: Vec::with_capacity(max_size),
            max_size,
        }
    }

    /// Number of entries currently stored.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Access all entries.
    pub fn entries(&self) -> &[ExperienceEntry] {
        &self.entries
    }

    /// Add an entry.  If the track is over capacity, the oldest
    /// entry is evicted (returned) to make room.
    ///
    /// Returns the evicted entry if any.
    pub fn add(&mut self, entry: ExperienceEntry) -> Option<ExperienceEntry> {
        let evicted = if self.entries.len() >= self.max_size {
            Some(self.entries.remove(0))
        } else {
            None
        };
        self.entries.push(entry);
        evicted
    }

    /// Drain all entries, leaving the track empty.
    pub fn drain_all(&mut self) -> Vec<ExperienceEntry> {
        std::mem::take(&mut self.entries)
    }

    /// Drain up to `n` oldest entries.
    pub fn drain_oldest(&mut self, n: usize) -> Vec<ExperienceEntry> {
        let count = n.min(self.entries.len());
        self.entries.drain(0..count).collect()
    }

    /// Clear the track entirely.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Search within fluid entries only.
    pub fn search(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let mut scored: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let sim = cosine_similarity_768(query, &e.embedding);
                (i, sim * e.weight)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);

        scored.into_iter().map(|(i, s)| (self.entries[i].clone(), s)).collect()
    }
}

impl Default for FluidTrack {
    fn default() -> Self {
        Self::new(512)
    }
}

// ---------------------------------------------------------------------------
//  DualTrackMemory
// ---------------------------------------------------------------------------

/// Combines a persistent A-track (bedrock) and a volatile B-track (fluid).
///
/// Queries merge results from both tracks.  When the fluid track
/// grows large enough, a consolidation pass promotes cluster
/// representatives into the bedrock.
pub struct DualTrackMemory {
    /// Stable, mmap-backed A-track.
    bedrock: ExperiencePool,
    /// Volatile, in-memory B-track.
    fluid: FluidTrack,
    /// Cluster consolidator for promoting fluid → bedrock.
    consolidator: ClusterConsolidator,
    /// Minimum cluster size for consolidation.
    min_cluster_size: usize,
    /// Weight assigned to consolidated representatives.
    consolidated_weight: f32,
    /// Credibility factor for bedrock entries (0.0–1.0).
    /// Bedrock scores are multiplied by this when merging.
    bedrock_credibility: f32,
    /// Credibility factor for fluid entries.
    fluid_credibility: f32,
    /// L1 confidence threshold.
    confidence_threshold: f32,
    /// Auto-consolidation threshold: when fluid exceeds this count,
    /// the next add triggers a consolidation pass.
    auto_consolidate_at: usize,
}

impl DualTrackMemory {
    /// Open or create a dual-track memory.
    ///
    /// The bedrock path points to the mmap file for persistent
    /// storage.
    pub fn open<P: AsRef<std::path::Path>>(
        bedrock_path: P,
        fluid_max_size: usize,
        confidence_threshold: f32,
    ) -> Result<Self> {
        let bedrock = ExperiencePool::open(bedrock_path)?;

        Ok(Self {
            bedrock,
            fluid: FluidTrack::new(fluid_max_size),
            consolidator: ClusterConsolidator::default(),
            min_cluster_size: 3,
            consolidated_weight: 0.7,
            bedrock_credibility: 1.0,
            fluid_credibility: 0.6,
            confidence_threshold,
            auto_consolidate_at: fluid_max_size,
        })
    }

    // ── Public accessors ──

    pub fn bedrock_len(&self) -> usize {
        self.bedrock.len()
    }

    pub fn fluid_len(&self) -> usize {
        self.fluid.len()
    }

    pub fn total_count(&self) -> usize {
        self.bedrock.len() + self.fluid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bedrock.is_empty() && self.fluid.is_empty()
    }

    pub fn bedrock_entries(&self) -> &[ExperienceEntry] {
        self.bedrock.entries()
    }

    pub fn fluid_entries(&self) -> &[ExperienceEntry] {
        self.fluid.entries()
    }

    /// Flush bedrock to disk.
    pub fn flush_bedrock(&mut self) -> Result<()> {
        self.bedrock.flush()
    }

    // ── Configuration setters (builder pattern) ──

    pub fn with_consolidator(mut self, consolidator: ClusterConsolidator) -> Self {
        self.consolidator = consolidator;
        self
    }

    pub fn with_min_cluster_size(mut self, min: usize) -> Self {
        self.min_cluster_size = min;
        self
    }

    pub fn with_bedrock_credibility(mut self, cred: f32) -> Self {
        self.bedrock_credibility = cred;
        self
    }

    pub fn with_fluid_credibility(mut self, cred: f32) -> Self {
        self.fluid_credibility = cred;
        self
    }

    pub fn with_auto_consolidate_at(mut self, n: usize) -> Self {
        self.auto_consolidate_at = n;
        self
    }

    // ── Experience lifecycle ──

    /// Add an experience to the fluid track.
    ///
    /// If the fluid track exceeds `auto_consolidate_at`, a
    /// consolidation pass is automatically triggered.
    pub fn add_experience(&mut self, entry: ExperienceEntry) {
        self.fluid.add(entry);

        // Auto-consolidate if fluid is getting full.
        if self.fluid.len() >= self.auto_consolidate_at {
            self.consolidate();
        }
    }

    /// Promote a single experience directly to bedrock, bypassing
    /// the fluid track.
    pub fn promote_to_bedrock(&mut self, entry: ExperienceEntry) {
        self.bedrock.add(entry);
    }

    /// Run consolidation: cluster fluid entries and promote
    /// representatives to bedrock, then clear the fluid track.
    pub fn consolidate(&mut self) {
        if self.fluid.is_empty() {
            return;
        }

        let fluid_entries = self.fluid.drain_all();
        let representatives =
            self.consolidator
                .consolidate(&fluid_entries, self.min_cluster_size, self.consolidated_weight);

        if !representatives.is_empty() {
            trace!(
                consolidated = representatives.len(),
                from_fluid = fluid_entries.len(),
                "Consolidating fluid → bedrock"
            );
            self.bedrock.extend(representatives);
        } else {
            // No clusters qualified — reinstate fluid entries to avoid data loss.
            trace!(
                from_fluid = fluid_entries.len(),
                "No qualifying clusters — reinstating fluid"
            );
            for entry in fluid_entries {
                self.fluid.add(entry);
            }
        }
    }

    /// Clear both tracks entirely.
    pub fn clear(&mut self) -> Result<()> {
        self.fluid.clear();
        self.bedrock.clear()
    }

    // ── Query ──

    /// Retrieve top-k results merged from both tracks.
    ///
    /// Bedrock scores are boosted by `bedrock_credibility`, fluid
    /// scores by `fluid_credibility`.
    pub fn search(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        let mut results: Vec<(ExperienceEntry, f32)> = Vec::with_capacity(k);

        // Bedrock results.
        for (entry, score) in self.bedrock.search(query, k) {
            results.push((entry, score * self.bedrock_credibility));
        }

        // Fluid results (lower credibility).
        for (entry, score) in self.fluid.search(query, k) {
            results.push((entry, score * self.fluid_credibility));
        }

        // Merge and sort.
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(k);
        results
    }

    /// Combined confidence check using both tracks.
    pub fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
    ) -> std::result::Result<L1Assessment, SpawnRejection> {
        if self.is_empty() {
            // Presumed guilty: no experience = insufficient evidence.
            return Err(SpawnRejection::L1Rejected {
                reason: "No experience available — presumed guilty".to_string(),
                confidence: 0.0,
            });
        }

        let task_matches = self.search(task_embedding, 5);
        let role_matches = self.search(role_embedding, 5);

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
                reason: format!(
                    "Low confidence: combined={:.4}, threshold={:.4}",
                    combined, self.confidence_threshold
                ),
                confidence: combined,
            })
        }
    }

    fn infer_tools(&self, matches: &[(ExperienceEntry, f32)]) -> u64 {
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
}

// ---------------------------------------------------------------------------
//  ExperienceRetrieval trait impl for DualTrackMemory
// ---------------------------------------------------------------------------

impl crate::l1::ExperienceRetrieval for DualTrackMemory {
    fn retrieve(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        self.search(query, k)
    }

    fn check_confidence(
        &self,
        task_embedding: &[f32; EMBEDDING_DIM],
        role_embedding: &[f32; EMBEDDING_DIM],
    ) -> std::result::Result<L1Assessment, SpawnRejection> {
        self.check_confidence(task_embedding, role_embedding)
    }

    fn add_experience(&mut self, entry: ExperienceEntry) {
        self.add_experience(entry);
    }

    fn experience_count(&self) -> usize {
        self.total_count()
    }

    fn flush(&mut self) -> anyhow::Result<()> {
        self.flush_bedrock()
    }

    fn bedrock_count(&self) -> usize {
        self.bedrock_len()
    }

    fn fluid_count(&self) -> usize {
        self.fluid_len()
    }
}

// ---------------------------------------------------------------------------
//  Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(embedding_val: f32, weight: f32) -> ExperienceEntry {
        let mut e = ExperienceEntry {
            embedding: [0.0f32; EMBEDDING_DIM],
            applicability_vector: [0.0f32; 128],
            tool_bitmap: 0,
            role_template_id: None,
            weight,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        };
        e.embedding[0] = embedding_val;
        e
    }

    fn tmp_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("test_dual_{}", rand::random::<u64>()))
    }

    #[test]
    fn test_basic_add_and_count() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();
        assert!(mem.is_empty());

        mem.add_experience(make_entry(1.0, 1.0));
        assert_eq!(mem.fluid_len(), 1);
        assert_eq!(mem.total_count(), 1);

        // Bedrock is still empty.
        assert_eq!(mem.bedrock_len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_bedrock_promotion() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();

        mem.promote_to_bedrock(make_entry(1.0, 1.0));
        assert_eq!(mem.bedrock_len(), 1);
        assert_eq!(mem.total_count(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_search_merges_tracks() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();

        // Add to bedrock.
        let mut e1 = make_entry(1.0, 1.0);
        e1.tool_bitmap = 0b001;
        mem.promote_to_bedrock(e1);

        // Add to fluid.
        let mut e2 = make_entry(0.8, 0.5);
        e2.tool_bitmap = 0b010;
        mem.add_experience(e2);

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;

        let results = mem.search(&query, 5);
        assert_eq!(results.len(), 2);

        // Bedrock entry should be first (higher similarity × credibility).
        assert_eq!(results[0].0.tool_bitmap, 0b001);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_confidence_check() {
        let path = tmp_path();
        let mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();

        // Empty -> presumed guilty: rejected.
        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;
        let result = mem.check_confidence(&query, &query);
        assert!(result.is_err(), "empty pool should reject (presumed guilty)");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_consolidation() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 100, 0.5).unwrap().with_min_cluster_size(2);

        // Add 3 similar entries to fluid.
        mem.add_experience(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(0.95, 1.0));
        mem.add_experience(make_entry(1.05, 1.0));

        assert_eq!(mem.fluid_len(), 3);
        mem.consolidate();

        // After consolidation, fluid should be empty and bedrock
        // should have the consolidated representative.
        assert_eq!(mem.fluid_len(), 0);
        assert_eq!(mem.bedrock_len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_consolidation_respects_min_size() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 100, 0.5).unwrap().with_min_cluster_size(5);

        // Add only 3 entries — below min cluster size.
        mem.add_experience(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(0.95, 1.0));
        mem.add_experience(make_entry(1.05, 1.0));
        mem.consolidate();

        // Nothing should be promoted.
        assert_eq!(mem.bedrock_len(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_auto_consolidate() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 100, 0.5)
            .unwrap()
            .with_min_cluster_size(2)
            .with_auto_consolidate_at(3);

        // Add 3 entries — should trigger auto-consolidation.
        mem.add_experience(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(0.95, 1.0));
        mem.add_experience(make_entry(1.05, 1.0));

        assert_eq!(mem.fluid_len(), 0, "fluid should be drained after consolidation");
        assert!(mem.bedrock_len() > 0, "bedrock should have consolidated entries");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_infer_tools() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();

        let mut e1 = make_entry(1.0, 1.0);
        e1.tool_bitmap = 0b101; // tools 0 and 2
        let mut e2 = make_entry(0.8, 0.8);
        e2.tool_bitmap = 0b011; // tools 0 and 1

        mem.promote_to_bedrock(e1);
        mem.add_experience(e2);

        let mut query = [0.0f32; EMBEDDING_DIM];
        query[0] = 1.0;
        let matches = mem.search(&query, 5);

        let tools = mem.infer_tools(&matches);
        // Tool 0 should be voted (both have it).
        assert!(tools & 0b001 != 0, "tool 0 should be recommended");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_clear() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 10, 0.5).unwrap();

        mem.promote_to_bedrock(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(1.0, 1.0));
        assert_eq!(mem.total_count(), 2);

        mem.clear().unwrap();
        assert_eq!(mem.total_count(), 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_fluid_max_size_eviction() {
        let path = tmp_path();
        let mut mem = DualTrackMemory::open(&path, 3, 0.5)
            .unwrap()
            .with_auto_consolidate_at(usize::MAX);

        mem.add_experience(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(2.0, 1.0));
        mem.add_experience(make_entry(3.0, 1.0));
        assert_eq!(mem.fluid_len(), 3);

        // This should evict the oldest (1.0).
        mem.add_experience(make_entry(4.0, 1.0));
        assert_eq!(mem.fluid_len(), 3);

        // First fluid entry should now be the one with 2.0.
        assert!((mem.fluid.entries()[0].embedding[0] - 2.0).abs() < 1e-6);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_consolidate_reinstates_fluid_when_no_cluster_qualifies() {
        let path = tmp_path();
        // min_cluster_size=10 means no cluster of 3 entries will qualify
        let mut mem = DualTrackMemory::open(&path, 100, 0.5)
            .unwrap()
            .with_min_cluster_size(10)
            .with_auto_consolidate_at(usize::MAX);

        // Add 3 entries
        mem.add_experience(make_entry(1.0, 1.0));
        mem.add_experience(make_entry(1.05, 1.0));
        mem.add_experience(make_entry(1.1, 1.0));
        assert_eq!(mem.fluid_len(), 3);

        // Manually trigger consolidate — no cluster qualifies, so fluid is reinstated
        mem.consolidate();

        // Fixed: fluid entries are reinstated when no cluster qualifies
        assert_eq!(mem.fluid_len(), 3, "fluid is reinstated after failed consolidation");
        assert_eq!(mem.bedrock_len(), 0, "bedrock has no new entries");
        assert_eq!(mem.total_count(), 3, "no data lost on failed consolidation");
        std::fs::remove_file(&path).ok();
    }
}
