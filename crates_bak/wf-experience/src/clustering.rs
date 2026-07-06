//! Threshold-based Leader Clustering with Welford's online algorithm.
//!
//! Groups similar experiences into clusters using a cosine-similarity
//! threshold.  Within each cluster, the representative embedding is
//! the Welford running mean, tracking variance for quality estimation.

use wf_core::simd::cosine_similarity_384;
use wf_core::{EMBEDDING_DIM, ExperienceEntry};

// ---------------------------------------------------------------------------
//  Cluster
// ---------------------------------------------------------------------------

/// A cluster of similar experiences with an online Welford centroid.
#[derive(Clone, Debug)]
pub struct Cluster {
    /// Running mean embedding (centroid).
    pub centroid: [f32; EMBEDDING_DIM],
    /// Number of points in this cluster.
    pub count: u64,
    /// Sum of experience weights (for weighted centroid).
    pub sum_weights: f64,
    /// Welford M2 accumulator (sum of squared distances from mean).
    /// Useful for estimating cluster cohesion.
    pub m2: f64,
    /// Aggregated tool bitmap (union of all tools seen).
    pub tool_bitmap: u64,
    /// Most common domain version.
    pub domain_version: u64,
    /// Latest timestamp across all points.
    pub latest_timestamp: u64,
    /// Max L2 override weight seen.
    pub max_l2_weight: f32,
    /// Role template IDs seen in this cluster (deduplicated).
    pub role_template_ids: Vec<u32>,
}

impl Cluster {
    pub fn new(entry: &ExperienceEntry) -> Self {
        Self {
            centroid: entry.embedding,
            count: 1,
            sum_weights: entry.weight as f64,
            m2: 0.0,
            tool_bitmap: entry.tool_bitmap,
            domain_version: entry.domain_version,
            latest_timestamp: entry.timestamp,
            max_l2_weight: entry.l2_override_weight,
            role_template_ids: entry
                .role_template_id
                .map(|id| vec![id])
                .unwrap_or_default(),
        }
    }

    /// Distance between the centroid and `embedding` (1 - cosine similarity).
    pub fn distance_to(&self, embedding: &[f32; EMBEDDING_DIM]) -> f64 {
        (1.0 - cosine_similarity_384(&self.centroid, embedding) as f64).max(0.0)
    }

    /// Update with Welford's online algorithm.
    ///
    /// This updates the running mean in a numerically stable way
    /// without storing all data points.
    pub fn update(&mut self, entry: &ExperienceEntry) {
        self.count += 1;
        // Use a minimum epsilon weight to ensure numerical stability.
        // When all entries have weight 0, the centroid won't update at all,
        // giving a misleading cluster with zero variance.
        let effective_weight = if entry.weight <= 0.0 {
            f32::MIN_POSITIVE
        } else {
            entry.weight
        };
        let old_weight = self.sum_weights;
        let new_weight = old_weight + effective_weight as f64;
        self.sum_weights = new_weight;

        // Weighted Welford update for the centroid.
        // Guard against division by zero when all weights are 0.
        if new_weight > 0.0_f64 {
            let weight_ratio = effective_weight as f64 / new_weight;
            for i in 0..EMBEDDING_DIM {
                let delta = entry.embedding[i] as f64 - self.centroid[i] as f64;
                self.centroid[i] += (delta * weight_ratio) as f32;

                // Weighted Welford: M2 += w_new * delta * delta2
                // where delta = x - mean_old, delta2 = x - mean_new.
                // This simplifies to w_old * w_new / (w_old + w_new) * delta^2.
                let delta2 = entry.embedding[i] as f64 - self.centroid[i] as f64;
                self.m2 += effective_weight as f64 * delta * delta2;
            }
        }

        self.tool_bitmap |= entry.tool_bitmap;
        self.domain_version = self.domain_version.max(entry.domain_version);
        self.latest_timestamp = self.latest_timestamp.max(entry.timestamp);
        self.max_l2_weight = self.max_l2_weight.max(entry.l2_override_weight);

        // Track role template IDs (deduplicated).
        if let Some(id) = entry.role_template_id
            && !self.role_template_ids.contains(&id)
        {
            self.role_template_ids.push(id);
        }
    }

    /// Variance estimate (mean squared distance from centroid).
    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            0.0
        } else {
            self.m2 / (self.count as f64 - 1.0)
        }
    }

    /// Convert to a single `ExperienceEntry` (the representative).
    pub fn to_experience_entry(&self, default_weight: f32) -> ExperienceEntry {
        let weight = if self.sum_weights > 0.0 {
            (self.sum_weights / self.count as f64) as f32
        } else {
            default_weight
        };

        ExperienceEntry {
            embedding: self.centroid,
            applicability_vector: [0.0f32; 128],
            tool_bitmap: self.tool_bitmap,
            role_template_id: self.most_common_role(),
            weight,
            domain_version: self.domain_version,
            timestamp: self.latest_timestamp,
            l2_override_weight: self.max_l2_weight,
            l2_override_created_at: 0,
        }
    }

    /// If the cluster has exactly one role, return it.
    /// Mixed-role clusters return None.
    fn most_common_role(&self) -> Option<u32> {
        if self.role_template_ids.len() == 1 {
            Some(self.role_template_ids[0])
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
//  ClusterConsolidator
// ---------------------------------------------------------------------------

/// Drives leader clustering over a set of experiences, producing
/// consolidated representatives that can be moved from B-track to A-track.
pub struct ClusterConsolidator {
    /// Cosine-similarity threshold for belonging to an existing cluster.
    /// Higher values = more clusters (finer granularity).
    pub threshold: f32,
}

impl ClusterConsolidator {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }

    /// Run leader clustering on `entries`.
    ///
    /// Returns a list of clusters sorted by count (largest first).
    pub fn cluster(&self, entries: &[ExperienceEntry]) -> Vec<Cluster> {
        let mut clusters: Vec<Cluster> = Vec::new();

        for entry in entries {
            let mut assigned = false;
            for cluster in &mut clusters {
                let dist = cluster.distance_to(&entry.embedding);
                if dist <= (1.0 - self.threshold as f64) {
                    cluster.update(entry);
                    assigned = true;
                    break;
                }
            }
            if !assigned {
                clusters.push(Cluster::new(entry));
            }
        }

        // Sort by count descending.
        clusters.sort_by(|a, b| b.count.cmp(&a.count));
        clusters
    }

    /// Consolidate B-track entries into representatives suitable for
    /// the A-track.  Only clusters with sufficient mass are returned.
    ///
    /// * `min_cluster_size` — minimum points for a cluster to be included.
    /// * `default_weight` — weight assigned to cluster representatives.
    pub fn consolidate(
        &self,
        fluid_entries: &[ExperienceEntry],
        min_cluster_size: usize,
        default_weight: f32,
    ) -> Vec<ExperienceEntry> {
        if fluid_entries.is_empty() {
            return Vec::new();
        }

        let clusters = self.cluster(fluid_entries);
        clusters
            .into_iter()
            .filter(|c| c.count >= min_cluster_size as u64)
            .map(|c| c.to_experience_entry(default_weight))
            .collect()
    }
}

impl Default for ClusterConsolidator {
    /// Default threshold: 0.85 cosine similarity.
    fn default() -> Self {
        Self { threshold: 0.85 }
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

    #[test]
    fn test_single_cluster() {
        let consolidator = ClusterConsolidator::new(0.5);
        let entries = vec![
            make_entry(1.0, 1.0),
            make_entry(0.9, 1.0),
            make_entry(0.95, 1.0),
        ];

        let clusters = consolidator.cluster(&entries);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].count, 3);
    }

    #[test]
    fn test_two_clusters() {
        let consolidator = ClusterConsolidator::new(0.5);
        let entries = vec![
            make_entry(1.0, 1.0),
            make_entry(0.9, 1.0),
            make_entry(-1.0, 1.0),
            make_entry(-0.9, 1.0),
        ];

        let clusters = consolidator.cluster(&entries);
        // We may get 2 clusters if the threshold separates them.
        assert!(clusters.len() >= 2);
    }

    #[test]
    fn test_welford_update() {
        let e1 = make_entry(1.0, 1.0);
        let e2 = make_entry(0.5, 1.0);
        let mut cluster = Cluster::new(&e1);
        cluster.update(&e2);

        // Centroid should be the mean.
        assert!((cluster.centroid[0] - 0.75).abs() < 1e-6);
        assert_eq!(cluster.count, 2);
    }

    #[test]
    fn test_consolidate_filters_small_clusters() {
        let consolidator = ClusterConsolidator::new(0.5);
        let entries = vec![
            make_entry(1.0, 1.0),
            make_entry(1.1, 1.0),
            make_entry(-1.0, 1.0),
        ];

        let reps = consolidator.consolidate(&entries, 2, 0.5);
        // Only the cluster with 2 entries (threshold 0.5) should survive.
        assert_eq!(reps.len(), 1);
        assert!((reps[0].embedding[0] - 1.05).abs() < 1e-5);
    }

    #[test]
    fn test_empty_consolidate() {
        let consolidator = ClusterConsolidator::default();
        let reps = consolidator.consolidate(&[], 2, 1.0);
        assert!(reps.is_empty());
    }
}
