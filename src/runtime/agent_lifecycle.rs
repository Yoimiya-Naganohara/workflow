//! Agent lifecycle — spawn, execute, synthesize.
//!
//! Methods on [`AgentRuntime`] for creating agents, executing them,
//! and aggregating results.  Extracted from `runtime.rs`.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use super::AgentRuntime;
use super::config::RoleTemplate;

use crate::agent::plan::{PlanEntity, PlanStatus, Task, TaskStatus};
use crate::agent::{Agent, AgentPool, AgentStatus};
use crate::core::types::*;

impl AgentRuntime {
    pub fn bootstrap_root_agent(
        &self,
        goal: &str,
        role: &str,
        agent_pool: &mut AgentPool,
    ) -> AgentId {
        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        // Phase 2B: Create a root task in the graph so Agent ↔ Task mapping
        // is always consistent — no more "agent exists but task doesn't" window.
        let root_task_id: crate::core::types::TaskId = {
            let mut g = self.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            let id = g.spawn_root(goal);
            // Mark as Decomposed so it can receive children.
            g.mark_decomposed(id).ok();
            id
        };

        let agent_id: AgentId = rand::random();
        // Create sandbox (best-effort — failure means no filesystem isolation).
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .ok()
            .map(std::sync::Arc::new);
        let agent = Agent {
            id: agent_id,
            name: format!(
                "{}-{:04x}",
                role,
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            role: role.to_string(),
            role_template_id: Some(role_tpl.template_id),
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: goal.to_string(),
            config: crate::agent::AgentConfig {
                model_id: self.model_id.clone(),
                ..Default::default()
            },
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: crate::agent::now_secs(),
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: Some(root_task_id),
            sandbox,
            retry_count: 0,
            reasoning: String::new(),
        };
        agent_pool.add_agent(agent);
        agent_id
    }

    pub async fn spawn_root_agent(
        &self,
        goal: &str,
        role: &str,
        value_statement: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_emb = self.pipeline.embedding().embed(role).await?;
        let task_emb = self.pipeline.embedding().embed(goal).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();

        // Run the decision pipeline
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: 0,
            responsibility_chain: vec![agent_id],
            raw_text_ref: None,
        };

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the agent (ownership transferred).
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: role_tpl_id,
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
                        allowed_tools: config.allowed_tools,
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                    context: Vec::new(),
                    last_active_at: crate::agent::now_secs(),
                    tokens_input: 0,
                    tokens_output: 0,
                    tool_trace: std::collections::VecDeque::new(),
                    inbox: std::collections::VecDeque::new(),
                    sandbox: sandbox.clone(),
                    task_id: None,
                    retry_count: 0,
                    reasoning: String::new(),
                };
                agent_pool.add_agent(agent);
                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_child(
        &self,
        parent_id: AgentId,
        parent_depth: u32,
        role: &str,
        goal: &str,
        value_statement: &str,
        responsibility_chain: &[AgentId],
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let max_depth = crate::core::constants::DEFAULT_MAX_DEPTH;
        if parent_depth + 1 >= max_depth {
            return Err(anyhow::anyhow!(
                "Agent depth limit ({}) reached — cannot spawn '{}' at depth {}",
                max_depth,
                role,
                parent_depth + 1
            ));
        }
        let role_emb = self.pipeline.embedding().embed(role).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();
        let task_emb = self.pipeline.embedding().embed(goal).await?;
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

        let mut chain = responsibility_chain.to_vec();
        chain.push(agent_id);

        // Derive parent_span_id from the first agent in the responsibility chain.
        let parent_span_id: u64 = responsibility_chain
            .first()
            .and_then(|id| Some(u64::from_le_bytes(id[0..8].try_into().ok()?)))
            .unwrap_or(0);
        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: parent_depth + 1,
            responsibility_chain: chain,
            raw_text_ref: None,
        };

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the child agent.
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: Some(role_tpl.template_id),
                    parent_id: Some(parent_id),
                    children: Vec::new(),
                    depth: parent_depth + 1,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
                        allowed_tools: config.allowed_tools,
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                    context: Vec::new(),
                    last_active_at: crate::agent::now_secs(),
                    tokens_input: 0,
                    tokens_output: 0,
                    tool_trace: std::collections::VecDeque::new(),
                    inbox: std::collections::VecDeque::new(),
                    task_id: None,
                    sandbox: sandbox.clone(),
                    retry_count: 0,
                    reasoning: String::new(),
                };
                agent_pool.add_agent(agent);
                // Register plan entity
                let plan_entity = PlanEntity {
                    plan_name: format!(
                        "{}-{}-{:04x}",
                        role,
                        goal.chars().take(16).collect::<String>(),
                        agent_id[0] as u16
                    ),
                    agent_id,
                    parent_plan: None,
                    goal: goal.to_string(),
                    tasks: vec![Task {
                        id: 0,
                        description: goal.to_string(),
                        status: TaskStatus::Pending,
                        result: None,
                    }],
                    status: PlanStatus::Draft,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                {
                    let mut reg = self.pipeline.plans().write().await;
                    reg.insert(plan_entity);
                }

                // Link parent → child
                if let Some(parent) = agent_pool.get_agent_mut(&parent_id) {
                    parent.children.push(agent_id);
                }

                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    pub async fn spawn_plan_task_agent(
        &self,
        owner_id: AgentId,
        role: &str,
        goal: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let (parent_depth, responsibility_chain) = agent_pool
            .get_agent(&owner_id)
            .map(|agent| (agent.depth, vec![owner_id]))
            .ok_or_else(|| anyhow::anyhow!("Responsible agent not found"))?;

        self.spawn_child(
            owner_id,
            parent_depth,
            role,
            goal,
            "default",
            &responsibility_chain,
            agent_pool,
        )
        .await
    }

    pub async fn synthesize_plan_result(
        &self,
        owner_id: AgentId,
        plan_goal: &str,
        task_results: &[(usize, String)],
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> String {
        let (config, role, provider) = {
            let mut pool = agent_pool.write().await;
            let (config, role) = match pool.get_agent_mut(&owner_id) {
                Some(agent) => {
                    agent.status = AgentStatus::Aggregating;
                    (agent.config.clone(), agent.role.clone())
                }
                None => return "Responsible agent not found".to_string(),
            };
            (config, role, self.provider.clone())
        };

        let result = if let Some(provider) = provider {
            let role_system_prompt = self
                .role_template_store
                .get_by_role(&role)
                .map(|t| t.system_prompt)
                .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));
            let task_summary = task_results
                .iter()
                .map(|(id, result)| format!("Task {}:\n{}", id, result))
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt = format!(
                "You own this approved plan.\n\nPlan goal: {}\n\nCompleted task results:\n{}\n\nSynthesize the final result for the user.",
                plan_goal, task_summary
            );
            provider
                .chat(&config.model_id, &role_system_prompt, &prompt)
                .await
                .unwrap_or_else(|e| format!("Plan synthesis failed: {}", e))
        } else {
            "No LLM provider configured".to_string()
        };

        let mut pool = agent_pool.write().await;
        if let Some(agent) = pool.get_agent_mut(&owner_id) {
            agent.result = Some(result.clone());
            agent.status = AgentStatus::Completed;
            pool.release_budget_guard(&owner_id);
            pool.notify_completed(&owner_id);
        }
        result
    }

    /// Aggregate child results into a final synthesis by calling
    /// `provider.chat()` (pure text-in-text-out, no tools, no role
    /// alternation constraints).
    ///
    /// Reads `child_results` from the pool, builds a structured
    /// prompt, and stores the LLM response as the parent's `result`.
    pub async fn synthesize_aggregation(
        &self,
        owner_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Result<String> {
        // Phase 1: Drain inbox + child_results under a single write lock.
        // Collect raw data; closures must NOT borrow `pool` to avoid conflicts.
        let (config, role, provider, all_summaries, goal): (
            crate::agent::AgentConfig,
            String,
            Option<std::sync::Arc<crate::llm::LlmProvider>>,
            Vec<String>,
            String,
        ) = {
            let mut pool = agent_pool.write().await;
            let agent = pool
                .get_agent_mut(&owner_id)
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

            // Drain both inbox (structured handoff) and child_results (legacy).
            let inbox_msgs: Vec<crate::agent::AgentMessage> = agent.inbox.drain(..).collect();
            let cr_raw: Vec<(crate::core::types::AgentId, String)> =
                agent.child_results.drain(..).collect();

            let cfg = agent.config.clone();
            let rl = agent.role.clone();
            let gl = agent.goal.clone();

            // Build inbox summaries inline (no pool access needed).
            let inbox_summaries: Vec<String> = inbox_msgs
                .iter()
                .map(|msg| {
                    let hint = match &msg.payload {
                        Some(crate::agent::MessagePayload::AssetPointer {
                            asset_id, hint, ..
                        }) => format!(" (asset: {}, hint: {})", asset_id, hint),
                        Some(crate::agent::MessagePayload::StateSummary { summary, .. }) => {
                            format!(" (summary: {})", summary)
                        }
                        None => String::new(),
                    };
                    format!("[{}]{}[{}]", msg.from_name, hint, msg.content)
                })
                .collect();

            // Build legacy summaries (no pool access — use raw IDs).
            let cr_summaries: Vec<String> = cr_raw
                .iter()
                .map(|(_, result)| format!("[agent]\n{}", result))
                .collect();

            let all_summaries: Vec<String> = inbox_summaries
                .into_iter()
                .chain(cr_summaries.into_iter())
                .collect();

            (cfg, rl, self.provider.clone(), all_summaries, gl)
        };

        let provider = provider.ok_or_else(|| anyhow::anyhow!("No LLM provider configured"))?;

        let role_system_prompt = self
            .role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let child_count = all_summaries.len();
        let task_summary = all_summaries.join("\n\n---\n\n");

        // Include a note about SearchAsset when there are asset pointers.
        let has_assets = all_summaries.iter().any(|s| s.contains("(asset:"));
        let asset_note = if has_assets {
            concat!(
                "\n\n(NOTE) Some sub-tasks produced large outputs that are stored as assets. ",
                "If you need details, use `search_asset(asset_id, query)`. ",
                "Your current context only contains compact summaries. ",
                "Do not ask for the full raw output unless you truly need it."
            )
        } else {
            ""
        };

        let prompt = format!(
            "You delegated this goal to {} sub-agent(s).\n\nOriginal goal: {}\n\nCompleted sub-task results:\n{}{}\n\nSynthesize the final result for the user.",
            child_count, goal, task_summary, asset_note
        );

        let result = provider
            .chat(&config.model_id, &role_system_prompt, &prompt)
            .await
            .map_err(|e| anyhow::anyhow!("Synthesis LLM call failed: {}", e))?;

        Ok(result)
    }

    /// Map a tool name to a bit position for the tool bitmap.
    pub(crate) fn tool_bit(name: &str) -> u64 {
        match name {
            "read_file" => 1 << 0,
            "write_file" => 1 << 1,
            "sh" => 1 << 2,
            "list_dir" => 1 << 3,
            "grep" => 1 << 4,
            "find_files" => 1 << 5,
            "move_file" => 1 << 6,
            "copy_file" => 1 << 7,
            "delete_file" => 1 << 8,
            "append_file" => 1 << 9,
            "patch_file" => 1 << 10,
            "glob" => 1 << 11,
            "spawn_agent" => 1 << 12,
            "read_memo" => 1 << 13,
            "write_memo" => 1 << 14,
            "delete_memo" => 1 << 15,
            "list_memos" => 1 << 16,
            "call_agent" => 1 << 17,
            "list_agents" => 1 << 18,
            "send_message" => 1 << 19,
            "read_messages" => 1 << 20,
            "line_edit" => 1 << 21,
            "fetch" => 1 << 22,
            "search_asset" => 1 << 23,
            "extract_json" => 1 << 24,
            "diff_edit" => 1 << 25,
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── tool_bit ──

    #[test]
    fn test_tool_bit_read_file() {
        assert_eq!(AgentRuntime::tool_bit("read_file"), 1 << 0);
    }

    #[test]
    fn test_tool_bit_write_file() {
        assert_eq!(AgentRuntime::tool_bit("write_file"), 1 << 1);
    }

    #[test]
    fn test_tool_bit_sh() {
        assert_eq!(AgentRuntime::tool_bit("sh"), 1 << 2);
    }

    #[test]
    fn test_tool_bit_list_dir() {
        assert_eq!(AgentRuntime::tool_bit("list_dir"), 1 << 3);
    }

    #[test]
    fn test_tool_bit_grep() {
        assert_eq!(AgentRuntime::tool_bit("grep"), 1 << 4);
    }

    #[test]
    fn test_tool_bit_find_files() {
        assert_eq!(AgentRuntime::tool_bit("find_files"), 1 << 5);
    }

    #[test]
    fn test_tool_bit_move_file() {
        assert_eq!(AgentRuntime::tool_bit("move_file"), 1 << 6);
    }

    #[test]
    fn test_tool_bit_copy_file() {
        assert_eq!(AgentRuntime::tool_bit("copy_file"), 1 << 7);
    }

    #[test]
    fn test_tool_bit_delete_file() {
        assert_eq!(AgentRuntime::tool_bit("delete_file"), 1 << 8);
    }

    #[test]
    fn test_tool_bit_append_file() {
        assert_eq!(AgentRuntime::tool_bit("append_file"), 1 << 9);
    }

    #[test]
    fn test_tool_bit_patch_file() {
        assert_eq!(AgentRuntime::tool_bit("patch_file"), 1 << 10);
    }

    #[test]
    fn test_tool_bit_glob() {
        assert_eq!(AgentRuntime::tool_bit("glob"), 1 << 11);
    }

    #[test]
    fn test_tool_bit_spawn_agent() {
        assert_eq!(AgentRuntime::tool_bit("spawn_agent"), 1 << 12);
    }

    #[test]
    fn test_tool_bit_read_memo() {
        assert_eq!(AgentRuntime::tool_bit("read_memo"), 1 << 13);
    }

    #[test]
    fn test_tool_bit_write_memo() {
        assert_eq!(AgentRuntime::tool_bit("write_memo"), 1 << 14);
    }

    #[test]
    fn test_tool_bit_delete_memo() {
        assert_eq!(AgentRuntime::tool_bit("delete_memo"), 1 << 15);
    }

    #[test]
    fn test_tool_bit_list_memos() {
        assert_eq!(AgentRuntime::tool_bit("list_memos"), 1 << 16);
    }

    #[test]
    fn test_tool_bit_call_agent() {
        assert_eq!(AgentRuntime::tool_bit("call_agent"), 1 << 17);
    }

    #[test]
    fn test_tool_bit_list_agents() {
        assert_eq!(AgentRuntime::tool_bit("list_agents"), 1 << 18);
    }

    #[test]
    fn test_tool_bit_send_message() {
        assert_eq!(AgentRuntime::tool_bit("send_message"), 1 << 19);
    }

    #[test]
    fn test_tool_bit_read_messages() {
        assert_eq!(AgentRuntime::tool_bit("read_messages"), 1 << 20);
    }

    #[test]
    fn test_tool_bit_line_edit() {
        assert_eq!(AgentRuntime::tool_bit("line_edit"), 1 << 21);
    }

    #[test]
    fn test_tool_bit_fetch() {
        assert_eq!(AgentRuntime::tool_bit("fetch"), 1 << 22);
    }

    #[test]
    fn test_tool_bit_search_asset() {
        assert_eq!(AgentRuntime::tool_bit("search_asset"), 1 << 23);
    }

    #[test]
    fn test_tool_bit_extract_json() {
        assert_eq!(AgentRuntime::tool_bit("extract_json"), 1 << 24);
    }

    #[test]
    fn test_tool_bit_diff_edit() {
        assert_eq!(AgentRuntime::tool_bit("diff_edit"), 1 << 25);
    }

    // Combined bitmap from multiple tools
    #[test]
    fn test_tool_bit_combined_or() {
        let bits = AgentRuntime::tool_bit("read_file")
            | AgentRuntime::tool_bit("sh")
            | AgentRuntime::tool_bit("glob");
        assert!(bits & (1 << 0) != 0, "read_file");
        assert!(bits & (1 << 2) != 0, "sh");
        assert!(bits & (1 << 11) != 0, "glob");
        assert_eq!(bits.count_ones(), 3);
    }

    #[test]
    fn test_tool_bit_unknown_returns_zero() {
        assert_eq!(AgentRuntime::tool_bit("nonexistent_tool"), 0);
        assert_eq!(AgentRuntime::tool_bit(""), 0);
    }

    #[test]
    fn test_tool_bit_no_overlap() {
        // Every defined tool bit must be unique — no two tools share a bit.
        let tool_names = [
            "read_file",
            "write_file",
            "sh",
            "list_dir",
            "grep",
            "find_files",
            "move_file",
            "copy_file",
            "delete_file",
            "append_file",
            "patch_file",
            "glob",
            "spawn_agent",
            "read_memo",
            "write_memo",
            "delete_memo",
            "list_memos",
            "call_agent",
            "list_agents",
            "send_message",
            "read_messages",
            "line_edit",
            "fetch",
            "search_asset",
            "extract_json",
            "diff_edit",
        ];
        let bits: Vec<u64> = tool_names
            .iter()
            .map(|n| AgentRuntime::tool_bit(n))
            .collect();
        for (i, &a) in bits.iter().enumerate() {
            for (j, &b) in bits.iter().enumerate() {
                if i != j {
                    assert_eq!(
                        a & b,
                        0,
                        "bit {} ({}: bit {}) and bit {} ({}: bit {}) overlap at mask {}",
                        i,
                        tool_names[i],
                        a,
                        j,
                        tool_names[j],
                        b,
                        a & b
                    );
                }
            }
        }
    }

    // ── bootstrap_root_agent (requires tokio runtime for AgentRuntime::new) ──

    /// Mock embedding service returning normalized unit vectors.
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

    fn test_config() -> crate::runtime::AgentRuntimeConfig {
        let dir =
            std::env::temp_dir().join(format!("workflow_lifecycle_test_{}", rand::random::<u64>()));
        crate::runtime::AgentRuntimeConfig {
            bedrock_path: Some(dir.join("experience_a.bin")),
            role_template_path: Some(dir.join("role_templates.json")),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_bootstrap_root_agent_creates_agent() {
        let runtime = AgentRuntime::new(test_config(), Arc::new(MockEmbed));
        let mut pool = AgentPool::new();

        let agent_id = runtime.bootstrap_root_agent("Build a REST API", "developer", &mut pool);

        let agent = pool.get_agent(&agent_id).unwrap();
        assert_eq!(agent.goal, "Build a REST API");
        assert_eq!(agent.role, "developer");
        assert_eq!(agent.depth, 0);
        assert!(agent.parent_id.is_none());
        assert!(agent.children.is_empty());
        assert!(agent.task_id.is_some(), "root agent should have a task_id");
        assert_eq!(agent.status, AgentStatus::Idle);
        // Name should contain role prefix
        assert!(agent.name.starts_with("developer-"));
        // Model ID should match runtime
        assert_eq!(agent.config.model_id, runtime.model_id);
    }

    #[tokio::test]
    async fn test_bootstrap_root_agent_creates_task_graph_entry() {
        let runtime = AgentRuntime::new(test_config(), Arc::new(MockEmbed));
        let mut pool = AgentPool::new();

        let agent_id = runtime.bootstrap_root_agent("test goal", "planner", &mut pool);
        let agent = pool.get_agent(&agent_id).unwrap();
        let task_id = agent.task_id.unwrap();

        // Verify the task graph has this root task
        let graph = runtime.task_graph.lock().unwrap();
        let node = graph.get(&task_id).unwrap();
        assert_eq!(node.goal, "test goal");
        assert!(node.children.is_empty(), "root should have no children yet");
    }

    #[tokio::test]
    async fn test_bootstrap_root_agent_role_without_template_still_works() {
        let runtime = AgentRuntime::new(test_config(), Arc::new(MockEmbed));
        let mut pool = AgentPool::new();

        // Role "nonexistent-role" doesn't exist in the role template store
        let agent_id = runtime.bootstrap_root_agent("goal", "nonexistent-role", &mut pool);
        let agent = pool.get_agent(&agent_id).unwrap();
        assert_eq!(agent.role, "nonexistent-role");
        assert_eq!(agent.goal, "goal");
    }

    #[tokio::test]
    async fn test_bootstrap_root_agent_unique_ids() {
        let runtime = AgentRuntime::new(test_config(), Arc::new(MockEmbed));
        let mut pool = AgentPool::new();

        let id1 = runtime.bootstrap_root_agent("goal 1", "dev", &mut pool);
        let id2 = runtime.bootstrap_root_agent("goal 2", "dev", &mut pool);

        assert_ne!(id1, id2, "each bootstrap should generate a unique AgentId");
        assert_eq!(pool.agents().len(), 2);
    }
}
