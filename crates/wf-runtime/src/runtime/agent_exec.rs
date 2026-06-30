//! Agent execution — run a single agent with LLM + tools.
use super::AgentRuntime;
use std::sync::Arc;
use tokio::sync::RwLock;
use wf_agent::{AgentPool, AgentStatus};
use wf_core::*;
use wf_llm::LlmProvider;

impl AgentRuntime {
    pub(crate) async fn execute_agent_detached(
        runtime: Arc<RwLock<Self>>,
        agent_id: AgentId,
        agent_pool: Arc<RwLock<AgentPool>>,
        tool_server: Option<wf_tools::ToolServerHandle>,
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
            wf_core::MEMO_INSTRUCTIONS,
            wf_core::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
            inbox_hint,
        );
        let leaf_goal = format!(
            "Your goal: {}\n\nWork independently and produce a concrete result. Do not request sub-agents — you are a leaf agent.\n\nRULES:\n1. Use tools freely — there is no limit on how many times you can call them.\n2. Keep working until you have a complete, well-researched answer.\n3. When you have enough information, stop and provide your final answer.\n4. If you detect you are calling the same tool with the same arguments 3+ times, you are looping — stop and summarize what you have.\n\nYou are a capable engineer. Do not stop working until the goal is achieved.",
            goal
        );

        // Mark execution start in metrics
        {
            let mut pool = agent_pool.write().await;
            pool.mark_execution_start(&agent_id);
        }

        // Phase 3: Execute LLM call with our custom loop (no lock held)
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
            let (mut text, tools_used) =
                Self::process_tool_stream(stream, agent_id, &agent_pool).await;

            // If the tool loop was terminated (tool call limit hit), make a
            // follow-up LLM call without tools so the agent can produce a
            // proper summary response instead of cutting off abruptly.
            let was_terminated = agent_pool
                .read()
                .await
                .get_agent(&agent_id)
                .map(|a| a.loop_terminated)
                .unwrap_or(false);
            if was_terminated {
                // Reset the flag so agent reuse doesn't re-trigger.
                if let Some(agent) = agent_pool.write().await.get_agent_mut(&agent_id) {
                    agent.loop_terminated = false;
                }
                // The follow-up call has no tools — it can only summarise.
                let summary_prompt = format!(
                    "您的工具调用次数已达到上限。根据您已完成的工作，请提供一份全面的总结。\n\n\
                     以下是您的部分工作输出：\n{}",
                    text,
                );
                if let Ok(summary) = provider
                    .chat(&config.model_id, &system_prompt, &summary_prompt)
                    .await
                {
                    text = summary;
                }
            }

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
                rt.pipeline.add_experience(wf_core::ExperienceEntry {
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
    use crate::test_utils::MockEmbed;

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
        let agent = wf_agent::Agent {
            id: rand::random(),
            name: "exec-test-agent".to_string(),
            role: "developer".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test goal".to_string(),
            config: wf_agent::AgentConfig::default(),
            status: wf_agent::AgentStatus::Idle,
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
            loop_terminated: false,
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
        let (pool, _) = pool_with_agent();
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
        assert_eq!(agent.status, wf_agent::AgentStatus::Failed);
        assert!(
            agent
                .result
                .as_deref()
                .unwrap_or("")
                .contains("No LLM provider")
        );
    }
}
