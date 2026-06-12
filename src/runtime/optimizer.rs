//! Prompt optimization engine — learns from experience to improve role system prompts.
//!
//! # Flow
//!
//! 1. Collect all experiences for a role via [`get_experiences_by_role`].
//! 2. Analyze: successful patterns (weight ≥ 0.7) vs low-quality patterns (weight < 0.7).
//! 3. Build an LLM prompt that asks the model to synthesize an improved system prompt.
//! 4. Return the improved prompt for user review.
//! 5. User confirms → new prompt is saved to the role template.

use std::collections::HashMap;
use std::time::Instant;

use crate::core::types::ExperienceEntry;
use crate::llm::LlmProvider;
use crate::runtime::config::RoleTemplate;

/// Minimum number of experiences required before optimization is meaningful.
pub const MIN_EXPERIENCES: usize = 5;

/// Tracks optimization frequency to prevent repeated optimization of the same role.
pub struct OptimizationTracker {
    last_optimized: HashMap<u32, Instant>,
    pub min_interval_secs: u64,
    pub min_new_experiences: usize,
    experience_snapshot: HashMap<u32, usize>,
}

impl OptimizationTracker {
    pub fn new() -> Self {
        Self {
            last_optimized: HashMap::new(),
            min_interval_secs: 3600,
            min_new_experiences: 3,
            experience_snapshot: HashMap::new(),
        }
    }

    pub fn can_optimize(&self, role_id: u32, current_count: usize) -> Option<&'static str> {
        if let Some(last) = self.last_optimized.get(&role_id) {
            let elapsed = last.elapsed().as_secs();
            if elapsed < self.min_interval_secs {
                return Some("optimized too recently");
            }
        }
        if let Some(&prev_count) = self.experience_snapshot.get(&role_id) {
            if current_count < prev_count + self.min_new_experiences {
                return Some("not enough new experiences since last optimization");
            }
        }
        None
    }

    pub fn mark_optimized(&mut self, role_id: u32, experience_count: usize) {
        self.last_optimized.insert(role_id, Instant::now());
        self.experience_snapshot.insert(role_id, experience_count);
    }
}

impl Default for OptimizationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about a role's experience pool.
#[derive(Debug, Clone)]
pub struct OptimizationStats {
    pub total: usize,
    pub successful: usize,
    pub low_quality: usize,
    pub most_used_tools: u64,
    pub avg_weight: f32,
}

/// The result of an optimization run, ready for user review.
#[derive(Debug, Clone)]
pub struct OptimizationResult {
    pub role_name: String,
    pub original_prompt: String,
    pub improved_prompt: String,
    pub summary: String,
    pub stats: OptimizationStats,
}

/// Run prompt optimization for a role.
///
/// Returns `None` if there are not enough experiences yet.
pub async fn optimize_role(
    role: &RoleTemplate,
    experiences: &[ExperienceEntry],
    llm: &LlmProvider,
    model_id: &str,
) -> Result<OptimizationResult, anyhow::Error> {
    if experiences.len() < MIN_EXPERIENCES {
        anyhow::bail!(
            "Need at least {} experiences for role '{}', have {}",
            MIN_EXPERIENCES,
            role.role,
            experiences.len()
        );
    }

    let stats = compute_stats(experiences);
    let (successful, low_quality): (Vec<&ExperienceEntry>, Vec<&ExperienceEntry>) =
        experiences.iter().partition(|e| e.weight >= 0.7);
    let analysis_prompt = build_prompt(role, &stats, &successful, &low_quality);

    let response = llm
        .chat(model_id, "", &analysis_prompt)
        .await
        .map_err(|e| anyhow::anyhow!("LLM optimization call failed: {}", e))?;

    let improved_prompt = clean_output(&response);

    let summary = format!(
        "Analyzed {} experiences ({} successful, {} low-quality). \
         Tools used across all: {:016b}. Average weight: {:.2}.",
        stats.total, stats.successful, stats.low_quality, stats.most_used_tools, stats.avg_weight,
    );

    Ok(OptimizationResult {
        role_name: role.role.clone(),
        original_prompt: role.system_prompt.clone(),
        improved_prompt,
        summary,
        stats,
    })
}

fn compute_stats(experiences: &[ExperienceEntry]) -> OptimizationStats {
    let total = experiences.len();
    let (high, low): (Vec<_>, Vec<_>) = experiences.iter().partition(|e| e.weight >= 0.7);
    let mut tools = 0u64;
    for e in experiences {
        tools |= e.tool_bitmap;
    }
    let avg = if total > 0 {
        experiences.iter().map(|e| e.weight).sum::<f32>() / total as f32
    } else {
        0.0
    };

    OptimizationStats {
        total,
        successful: high.len(),
        low_quality: low.len(),
        most_used_tools: tools,
        avg_weight: avg,
    }
}

fn build_prompt(
    role: &RoleTemplate,
    stats: &OptimizationStats,
    successful: &[&ExperienceEntry],
    low_quality: &[&ExperienceEntry],
) -> String {
    let mut successes = Vec::new();
    for e in successful.iter().take(8) {
        successes.push(format!("  weight={:.2}  tools={:016b}", e.weight, e.tool_bitmap));
    }
    let mut failures = Vec::new();
    for e in low_quality.iter().take(4) {
        failures.push(format!("  weight={:.2}  tools={:016b}", e.weight, e.tool_bitmap));
    }

    format!(
        r#"You are optimizing an AI agent's role system prompt based on real execution data.

## Current Role
**{role_name}** — {label}

## Current System Prompt
```
{current_prompt}
```

## Performance Data
- Total experiences: {total}
- Successful (weight >= 0.7): {success_count}
- Low-quality (weight < 0.7): {fail_count}
- Most used tool bitmap across all: {tools:016b}
- Average experience weight: {avg:.2}

## Successful Experience Patterns (Top {n_ok})
{successes}

## Low-Quality Experience Patterns (Top {n_bad})
{failures}

## Task
Analyze the patterns above and produce an **improved system prompt** for the role "{role_name}".

**Guidelines:**
1. Keep what works from the current prompt — don't discard valuable guidance.
2. Add concrete, actionable guidance based on SUCCESSFUL patterns.
3. Add explicit warnings and anti-pattern guidance based on LOW-QUALITY patterns.
4. Be specific — prefer "Always use thiserror for error types" over "Handle errors properly".
5. If tool usage patterns are clear, include tool guidance.
6. Output ONLY the new system prompt — no explanations, no markdown fences, no commentary.

Improved system prompt for role "{role_name}":
"#,
        role_name = role.role,
        label = role.label,
        current_prompt = role.system_prompt,
        total = stats.total,
        success_count = stats.successful,
        fail_count = stats.low_quality,
        tools = stats.most_used_tools,
        avg = stats.avg_weight,
        n_ok = successful.len().min(8),
        n_bad = low_quality.len().min(4),
        successes = successes.join("\n"),
        failures = failures.join("\n"),
    )
}

fn clean_output(output: &str) -> String {
    let s = output.trim();
    if s.starts_with("```") {
        let after = s
            .lines()
            .skip(1)
            .take_while(|l| !l.trim().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n");
        if after.is_empty() {
            s.to_string()
        } else {
            after.trim().to_string()
        }
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(weight: f32, tools: u64) -> ExperienceEntry {
        ExperienceEntry {
            embedding: [0.0f32; 384],
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
    fn test_compute_stats_empty() {
        let stats = compute_stats(&[]);
        assert_eq!(stats.total, 0);
        assert_eq!(stats.avg_weight, 0.0);
    }

    #[test]
    fn test_compute_stats_partition() {
        let entries = vec![make_entry(0.9, 0b101), make_entry(0.3, 0b010), make_entry(0.8, 0b111)];
        let stats = compute_stats(&entries);
        assert_eq!(stats.total, 3);
        assert_eq!(stats.successful, 2);
        assert_eq!(stats.low_quality, 1);
        assert_eq!(stats.most_used_tools, 0b111);
        assert!((stats.avg_weight - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_clean_output_no_fence() {
        let out = clean_output("You are a developer. Write code.");
        assert_eq!(out, "You are a developer. Write code.");
    }

    #[test]
    fn test_clean_output_with_fence() {
        let out = clean_output("```\nYou are a developer.\nWrite code.\n```");
        assert_eq!(out, "You are a developer.\nWrite code.");
    }

    #[test]
    fn test_clean_output_with_lang_fence() {
        let out = clean_output("```markdown\nYou are a developer.\n```");
        assert_eq!(out, "You are a developer.");
    }

    #[test]
    fn test_min_experiences_constant() {
        assert!(MIN_EXPERIENCES >= 3, "Need at least 3 for meaningful analysis");
    }

    #[test]
    fn test_tracker_new_allows_optimization() {
        let tracker = OptimizationTracker::new();
        assert!(tracker.can_optimize(1, 5).is_none());
    }

    #[test]
    fn test_tracker_blocks_immediate_reoptimize() {
        let mut tracker = OptimizationTracker::new();
        tracker.mark_optimized(1, 5);
        assert!(tracker.can_optimize(1, 5).is_some());
        assert!(tracker.can_optimize(1, 10).is_some(), "should still block due to time");
    }

    #[test]
    fn test_tracker_blocks_insufficient_new_experiences() {
        let mut tracker = OptimizationTracker::new();
        tracker.min_interval_secs = 0;
        tracker.mark_optimized(1, 10);
        assert!(tracker.can_optimize(1, 11).is_some(), "need 3 new, got 1");
        assert!(tracker.can_optimize(1, 12).is_some(), "need 3 new, got 2");
        assert!(tracker.can_optimize(1, 13).is_none(), "got 3 new, should be allowed");
    }
}
