//! Agent execution — run a single agent with LLM + tools.
use super::AgentRuntime;
use crate::agent::{AgentPool, AgentStatus};
use crate::core::types::*;
use crate::llm::LlmProvider;
use std::sync::Arc;
use tokio::sync::RwLock;

impl AgentRuntime {
    pub(crate) async fn execute_agent_detached(
        runtime: Arc<RwLock<Self>>,
        agent_id: AgentId,
        agent_pool: Arc<RwLock<AgentPool>>,
        tool_server: Option<crate::tools::ToolServerHandle>,
    ) -> (String, AgentStatus) {
        // Phase 1: Extract needed data under a brief read lock
        let (provider, role_template_store, embedding_service) = {
            let rt = runtime.read().await;
            (
                rt.provider.clone(),
                Arc::clone(&rt.role_template_store),
                rt.pipeline.embedding().clone(),
            )
        };

        let (goal, role, config) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return (String::new(), AgentStatus::Failed),
            };
            (agent.goal, agent.role, agent.config.clone())
        };

        let provider: Arc<LlmProvider> = match provider {
            Some(p) => p,
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.release_budget_guard(&agent_id);
                    pool.notify_completed(&agent_id);
                }
                return (
                    "No LLM provider configured".to_string(),
                    AgentStatus::Failed,
                );
            }
        };

        // Mark planning
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Planning;
            }
        }

        // Phase 2: Build system prompt (no lock on runtime needed)
        let role_system_prompt = role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let memo_block = {
            let pool = agent_pool.read().await;
            pool.format_role_memos(&role)
        };
        let memos = memo_block.as_deref().unwrap_or("");

        // Read reasoning_effort + reasoning_options from pool (brief read lock).
        let (reasoning_effort, reasoning_options) = {
            let pool = agent_pool.read().await;
            (
                pool.reasoning_effort.clone(),
                pool.reasoning_options.clone(),
            )
        };

        // Check for pending messages and inject notification into prompt.
        let inbox_hint = {
            let pool = agent_pool.read().await;
            pool.get_agent(&agent_id)
                .map(|a| {
                    let count = a.inbox.len();
                    if count > 0 {
                        format!(
                            "\n\nYou have {} unread message(s) in your inbox. \
Use the `read_messages` tool to read them before proceeding. \
Messages may contain important context from sibling agents.",
                            count
                        )
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default()
        };

        let system_prompt = format!(
            "{}\n\n{}\n\n{}{}{}",
            role_system_prompt,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
            inbox_hint,
        );
        let leaf_goal = format!(
            "Your goal: {}\n\nWork independently and produce a concrete result. Do not request sub-agents — you are a leaf agent.\n\nTOOL DISCIPLINE: Only call tools when you truly need information you cannot infer. Prefer answering directly from your knowledge. You have up to 6 tool call rounds available, with a maximum of 12 total tool calls per session. No single tool may be called more than 6 times. If you have called several tools and still cannot answer, summarize what you found and explain what is missing. Repeated calls to the same tool with the same arguments (3+ times) will be treated as a loop and terminated.",
            goal
        );

        // Mark execution start in metrics
        {
            let mut pool = agent_pool.write().await;
            pool.mark_execution_start(&agent_id);
        }

        // Phase 3: Execute LLM call (no lock held on runtime)
        let (response, tool_bitmap) = if let Some(handle) = &tool_server {
            let additional_params = reasoning_effort
                .as_ref()
                .and_then(|effort| provider.reasoning_params(effort, &reasoning_options));
            let stream = match provider
                .chat_with_tools_stream_mcp(
                    &config.model_id,
                    &system_prompt,
                    &leaf_goal,
                    &[],
                    handle,
                    additional_params.as_ref(),
                )
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            let (text, tools_used) = Self::process_tool_stream(stream, agent_id, &agent_pool).await;
            (text, tools_used)
        } else {
            let text = match provider
                .chat(&config.model_id, &system_prompt, &leaf_goal)
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            (text, 0)
        };

        // Phase 4: Record result under brief lock
        {
            let mut pool = agent_pool.write().await;
            pool.mark_execution_complete(&agent_id);
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Completed;
                agent.result = Some(response.clone());
            }
        }

        // Phase 5: Record experience (re-acquire runtime lock briefly)
        if !response.is_empty() {
            let goal_for_emb = goal.clone();
            if let Ok(emb) = embedding_service.embed(&goal_for_emb).await {
                let rt = runtime.read().await;
                rt.pipeline
                    .add_experience(crate::core::types::ExperienceEntry {
                        embedding: emb,
                        applicability_vector: [0.0f32; 128],
                        tool_bitmap,
                        role_template_id: role_template_store
                            .get_by_role(&role)
                            .map(|t| t.template_id),
                        weight: 1.0,
                        domain_version: 0,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        l2_override_weight: 0.0,
                        l2_override_created_at: 0,
                    });
            }
        }

        // Phase 6: Log metrics and release resources
        let mut pool = agent_pool.write().await;

        // Emit structured log line for monitoring
        if let Some(log_line) =
            pool.metrics_log_line(&agent_id, &format!("{:02x}..", agent_id[0]), &role)
        {
            tracing::info!(target: "agent_metrics", "{}", log_line);
        }

        pool.release_budget_guard(&agent_id);
        pool.notify_completed(&agent_id);

        (response, AgentStatus::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::AgentRuntimeConfig;

    /// A mock embedding service that returns fixed embeddings.
    struct MockEmbed;
    #[async_trait::async_trait]
    impl crate::llm::EmbeddingService for MockEmbed {
        async fn embed(&self, _text: &str) -> anyhow::Result<[f32; 384]> {
            let mut e = [0.0f32; 384];
            e[0] = 1.0;
            Ok(e)
        }
        async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<[f32; 384]>> {
            Ok(texts
                .iter()
                .map(|_| {
                    let mut e = [0.0f32; 384];
                    e[0] = 1.0;
                    e
                })
                .collect())
        }
        fn similarity(&self, a: &[f32; 384], b: &[f32; 384]) -> f32 {
            crate::core::simd::cosine_similarity_384(a, b)
        }
        fn cache_size(&self) -> usize {
            0
        }
        fn clear_cache(&self) {}
        fn cache_hits(&self) -> u64 {
            0
        }
        fn cache_misses(&self) -> u64 {
            0
        }
    }

    fn test_config() -> AgentRuntimeConfig {
        let dir =
            std::env::temp_dir().join(format!("workflow_exec_test_{}", rand::random::<u64>()));
        AgentRuntimeConfig {
            bedrock_path: Some(dir.join("experience_a.bin")),
            role_template_path: Some(dir.join("role_templates.json")),
            ..Default::default()
        }
    }

    /// Create a pool with one agent, returning (pool, agent_id).
    fn pool_with_agent() -> (Arc<RwLock<AgentPool>>, AgentId) {
        let mut pool = AgentPool::new();
        let agent = crate::agent::Agent {
            id: rand::random(),
            name: "exec-test-agent".to_string(),
            role: "developer".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test goal".to_string(),
            config: crate::agent::AgentConfig::default(),
            status: crate::agent::AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: None,
            sandbox: None,
            retry_count: 0,
            reasoning: String::new(),
        };
        let id = agent.id;
        pool.add_agent(agent);
        (Arc::new(RwLock::new(pool)), id)
    }

    #[tokio::test]
    async fn test_agent_not_found_returns_failed() {
        let runtime = Arc::new(RwLock::new(AgentRuntime::new(
            test_config(),
            Arc::new(MockEmbed),
        )));
        let (pool, _aid) = pool_with_agent();
        let unknown_id: AgentId = rand::random();
        let (result, status) =
            AgentRuntime::execute_agent_detached(runtime, unknown_id, pool, None).await;
        assert_eq!(status, AgentStatus::Failed);
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_no_provider_returns_error() {
        let runtime = Arc::new(RwLock::new(AgentRuntime::new(
            test_config(),
            Arc::new(MockEmbed),
        )));
        let (pool, aid) = pool_with_agent();
        // Provider is None by default — execute_agent_detached should
        // mark the agent as failed and release budget guard.
        let (result, status) =
            AgentRuntime::execute_agent_detached(runtime, aid, pool.clone(), None).await;
        assert_eq!(status, AgentStatus::Failed);
        assert!(
            result.contains("No LLM provider configured"),
            "expected 'No LLM provider configured', got: {}",
            result
        );
        // Agent should be marked as Failed in the pool
        let pool_r = pool.read().await;
        let agent = pool_r.get_agent(&aid).unwrap();
        assert_eq!(agent.status, crate::agent::AgentStatus::Failed);
        assert!(
            agent
                .result
                .as_deref()
                .unwrap_or("")
                .contains("No LLM provider")
        );
    }
}
