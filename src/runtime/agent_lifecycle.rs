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
            let mut g = self
                .task_graph
                .lock()
                .expect("agent_lifecycle mutex poisoned");
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

    /// Map a tool name to its bit position for the tool bitmap.
    ///
    /// Bit positions are auto-assigned by position in `TOOL_NAMES`.
    /// Index order IS the bit assignment — add new tools at the end.
    /// Deprecated tools stay in place (stored experience bitmaps reference them).
    pub(crate) fn tool_bit(name: &str) -> u64 {
        const TOOL_NAMES: &[&str] = &[
            "read_file",     // bit 0
            "write_file",    // bit 1
            "sh",            // bit 2
            "list_dir",      // bit 3
            "grep",          // bit 4
            "find_files",    // bit 5
            "move_file",     // bit 6
            "copy_file",     // bit 7
            "delete_file",   // bit 8
            "append_file",   // bit 9 (deprecated)
            "patch_file",    // bit 10 (deprecated)
            "glob",          // bit 11 (deprecated)
            "spawn_agent",   // bit 12
            "read_memo",     // bit 13
            "write_memo",    // bit 14
            "delete_memo",   // bit 15
            "list_memos",    // bit 16
            "call_agent",    // bit 17 (reserved)
            "list_agents",   // bit 18
            "send_message",  // bit 19
            "read_messages", // bit 20
            "line_edit",     // bit 21 (deprecated)
            "fetch",         // bit 22
            "search_asset",  // bit 23
            "extract_json",  // bit 24
            "diff_edit",     // bit 25
        ];
        TOOL_NAMES
            .iter()
            .position(|&n| n == name)
            .map(|i| 1u64 << i)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::MockEmbed;
    use std::sync::Arc;

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
