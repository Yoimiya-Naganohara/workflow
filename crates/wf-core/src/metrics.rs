//! Agent execution metrics for monitoring and observability.

use std::collections::HashMap;

/// Per-tool execution metrics.
#[derive(Debug, Clone, Default)]
pub struct ToolMetrics {
    pub call_count: u64,
    pub error_count: u64,
    pub total_duration_ms: u64,
}

impl ToolMetrics {
    pub fn success_rate(&self) -> f64 {
        if self.call_count == 0 {
            1.0
        } else {
            (self.call_count - self.error_count) as f64 / self.call_count as f64
        }
    }
    pub fn avg_duration_ms(&self) -> f64 {
        if self.call_count == 0 {
            0.0
        } else {
            self.total_duration_ms as f64 / self.call_count as f64
        }
    }
}

/// Aggregate agent execution metrics.
#[derive(Debug, Clone, Default)]
pub struct AgentMetrics {
    pub tools: HashMap<String, ToolMetrics>,
    pub total_calls: u64,
    pub total_errors: u64,
    pub started_at: u64,
    pub completed_at: u64,
}

impl AgentMetrics {
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            1.0
        } else {
            (self.total_calls - self.total_errors) as f64 / self.total_calls as f64
        }
    }

    pub fn to_log_line(&self, agent_name: &str, role: &str) -> String {
        let duration = self.completed_at.saturating_sub(self.started_at);
        let tools: Vec<String> = self
            .tools
            .iter()
            .map(|(n, m)| {
                format!(
                    "{}:{}c/{}e/{:.0}ms",
                    n,
                    m.call_count,
                    m.error_count,
                    m.avg_duration_ms()
                )
            })
            .collect();
        format!(
            "agent={} role={} calls={} errors={} success={:.0}% duration={}s tools=[{}]",
            agent_name,
            role,
            self.total_calls,
            self.total_errors,
            self.success_rate() * 100.0,
            duration,
            tools.join(" ")
        )
    }
}
