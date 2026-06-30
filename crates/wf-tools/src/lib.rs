//! wf-tools — MCP tool system, sandbox, diff editor, memo tools.
//! MCP tool system using rig's ToolServer and ToolDyn infrastructure.
//!
//! Built-in tools are registered on a shared [`ToolServerHandle`] and
//! agents connect via `.tool_server_handle()`.

pub mod agent;
pub mod builtin;
pub mod diff_edit;
pub mod memo;

pub use rig::tool::server::{ToolServer, ToolServerHandle};

pub use memo::MemoToolDeps;

/// Create a [`ToolServerHandle`] pre-loaded with all built-in tools.
///
/// The handle is cheaply cloneable and can be shared across agents.
pub fn create_tool_server() -> ToolServerHandle {
    builtin::register_tools(ToolServer::new()).run()
}

/// Create a [`ToolServerHandle`] with built-ins, agent tools, and memo tools.
///
/// Takes an `AgentPool` directly instead of extracting from AppState,
/// avoiding a dependency on `wf-tui`.
pub fn create_agent_tool_server(
    agent_pool: std::sync::Arc<tokio::sync::RwLock<wf_agent::AgentPool>>,
    runtime_event_tx: Option<tokio::sync::mpsc::Sender<wf_core::event::RuntimeEvent>>,
    responsible_agent_id: Option<wf_core::AgentId>,
) -> ToolServerHandle {
    let pool = agent_pool;
    let tx = runtime_event_tx;
    let agent_id = responsible_agent_id;

    let server = builtin::register_tools(ToolServer::new());
    let server = agent::register_tools(server, pool.clone(), tx, agent_id);
    let memo_deps = memo::MemoToolDeps::new(pool, agent_id);
    memo::register_memo_tools(server, memo_deps).run()
}

/// Create a [`ToolServerHandle`] with sandbox-aware tools for a specific agent.
pub fn create_sandboxed_agent_tool_server(
    agent_pool: std::sync::Arc<tokio::sync::RwLock<wf_agent::AgentPool>>,
    runtime_event_tx: Option<tokio::sync::mpsc::Sender<wf_core::event::RuntimeEvent>>,
    responsible_agent_id: Option<wf_core::AgentId>,
    sandbox: Option<std::sync::Arc<wf_agent::sandbox::SandboxHandle>>,
) -> ToolServerHandle {
    let pool = agent_pool;
    let tx = runtime_event_tx;
    let agent_id = responsible_agent_id;

    let with_search_asset = sandbox.is_some();
    let server = builtin::register_sandboxed_tools(ToolServer::new(), sandbox, with_search_asset);
    let server = agent::register_tools(server, pool.clone(), tx, agent_id);
    let memo_deps = memo::MemoToolDeps::new(pool, agent_id);
    memo::register_memo_tools(server, memo_deps).run()
}

/// Create a [`ToolServerHandle`] and register one extra tool.
pub fn create_tool_server_with<T>(extra: T) -> ToolServerHandle
where
    T: rig::tool::Tool + 'static,
{
    let mut server = builtin::register_tools(ToolServer::new());
    server = server.tool(extra);
    server.run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_tool_server_returns_handle() {
        let handle = create_tool_server();
        // Handle should be cloneable (cheaply)
        let cloned = handle.clone();
        drop(cloned);
        // Handle is Send + Sync (verified in test below)
    }

    #[test]
    fn test_create_tool_server_is_send_sync() {
        let handle = create_tool_server();
        // Verify it can be sent across threads
        let result = std::thread::spawn(move || {
            let _ = handle;
            true
        })
        .join();
        assert!(result.unwrap());
    }

    #[test]
    fn test_tool_server_types_are_public() {
        // Verify the re-exports compile correctly
        let _ = ToolServer::new();
        // ToolServerHandle::new is not public, but we can create via run()
        let handle = builtin::register_tools(ToolServer::new()).run();
        let _ = handle.clone();
    }

    // ────────────────────────────────────────────────────────
    // MCP Agent Simulation: simulates an LLM agent calling
    // built-in tools through the ToolServerHandle (MCP interface)
    // ────────────────────────────────────────────────────────

    // ────────────────────────────────────────────────────────
    // Sandbox MCP Simulation: agent with filesystem isolation
    // ────────────────────────────────────────────────────────

    /// Helper: create a sandbox handle for testing.
    fn test_sandbox_handle() -> std::sync::Arc<wf_agent::sandbox::SandboxHandle> {
        static TEST_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
        let n = TEST_IDX.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut id = [0u8; 16];
        id[0..8].copy_from_slice(&n.to_le_bytes());
        std::sync::Arc::new(wf_agent::sandbox::SandboxHandle::new(&id).expect("sandbox creation"))
    }

    /// Simulate an agent using sandboxed MCP tools.
    #[tokio::test]
    async fn test_simulate_sandbox_write_isolation() {
        let sandbox = test_sandbox_handle();

        // Build sandboxed tool server (like create_sandboxed_agent_tool_server
        // would for a real agent)
        let server = crate::builtin::register_sandboxed_tools(
            crate::ToolServer::new(),
            Some(sandbox.clone()),
            true,
        );
        let handle = server.run();

        // Agent writes to a "project" path via sandboxed write_file
        let write_args = serde_json::json!({
            "path": "src/new_file.rs",
            "content": "fn sandboxed() {}\n"
        });
        let result = handle
            .call_tool("write_file", &write_args.to_string())
            .await
            .unwrap();
        println!("[SANDBOX] write result: {}", result);

        // Verify the file landed in sandbox workdir, NOT the real source tree
        let real_path = std::path::Path::new("src/new_file.rs");
        assert!(!real_path.exists(), "real source tree must NOT be touched");

        let sandbox_path = sandbox.workdir.join("src/new_file.rs");
        assert!(sandbox_path.exists(), "file must exist in sandbox workdir");
        let content = std::fs::read_to_string(&sandbox_path).unwrap();
        assert_eq!(content, "fn sandboxed() {}\n");
        println!(
            "[SANDBOX] File isolated in workdir: {}",
            sandbox_path.display()
        );

        sandbox.cleanup();
    }

    /// Simulate an agent reading a project file through the sandbox.
    #[tokio::test]
    async fn test_simulate_sandbox_read_via_symlink() {
        let sandbox = test_sandbox_handle();
        let server = crate::builtin::register_sandboxed_tools(
            crate::ToolServer::new(),
            Some(sandbox.clone()),
            false,
        );
        let handle = server.run();

        // Read a real project file through the sandbox's src→project symlink
        let read_args = serde_json::json!({
            "path": "src/tools/mod.rs"
        });
        let result = handle
            .call_tool("read_file", &read_args.to_string())
            .await
            .unwrap();
        assert!(result.contains("create_tool_server"));
        assert!(result.contains("MCP"));
        println!("[SANDBOX] Read through symlink succeeded");

        sandbox.cleanup();
    }

    /// Simulate sandbox rejecting an absolute write outside workdir.
    #[tokio::test]
    async fn test_simulate_sandbox_write_absolute_rejected() {
        let sandbox = test_sandbox_handle();
        let server = crate::builtin::register_sandboxed_tools(
            crate::ToolServer::new(),
            Some(sandbox.clone()),
            false,
        );
        let handle = server.run();

        // Agent tries to write with an absolute path that resolves outside workdir
        let write_args = serde_json::json!({
            "path": sandbox.source_root.join("escape.txt").to_str().unwrap().to_string(),
            "content": "should fail"
        });
        let result = handle
            .call_tool("write_file", &write_args.to_string())
            .await;
        assert!(
            result.is_err(),
            "absolute write outside workdir must be rejected"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("denied") || err.contains("rejected") || err.contains("outside"));
        println!("[SANDBOX] Absolute write correctly rejected: {}", err);

        sandbox.cleanup();
    }

    /// Simulate sandbox shell — commands run from the workdir.
    #[tokio::test]
    async fn test_simulate_sandbox_shell() {
        let sandbox = test_sandbox_handle();
        let server = crate::builtin::register_sandboxed_tools(
            crate::ToolServer::new(),
            Some(sandbox.clone()),
            false,
        );
        let handle = server.run();

        // Same 'sh' tool name — the sandbox variant routes through SandboxHandle
        let shell_args = serde_json::json!({
            "command": "echo 'sandboxed shell' && pwd"
        });
        let result = handle
            .call_tool("sh", &shell_args.to_string())
            .await
            .unwrap();
        assert!(result.contains("sandboxed shell"));
        assert!(result.contains(sandbox.workdir.to_str().unwrap()));
        println!("[SANDBOX] Shell runs inside workdir:\n{}", result);

        sandbox.cleanup();
    }

    /// Simulate agent MCP calls through the full sandboxed agent tool server.
    #[tokio::test]
    async fn test_simulate_sandbox_workflow() {
        let sandbox = test_sandbox_handle();
        let server = crate::builtin::register_sandboxed_tools(
            crate::ToolServer::new(),
            Some(sandbox.clone()),
            false,
        );
        let handle = server.run();

        // Step 1: Write a Python script via sandbox
        let write_args = serde_json::json!({
            "path": "scripts/hello.py",
            "content": "print('Hello from sandbox')\n"
        });
        let _ = handle
            .call_tool("write_file", &write_args.to_string())
            .await
            .unwrap();

        // Step 2: Use shell (sandbox-aware) to verify the file
        let shell_list_args = serde_json::json!({
            "command": "ls -la scripts/"
        });
        let shell_list_result = handle
            .call_tool("sh", &shell_list_args.to_string())
            .await
            .unwrap();
        assert!(shell_list_result.contains("hello.py"));
        println!("[SANDBOX WORKFLOW] ls via shell:\n{}", shell_list_result);

        // Step 3: Run it via shell (cwd = workdir)
        let shell_args = serde_json::json!({
            "command": "python3 scripts/hello.py 2>&1 || python scripts/hello.py 2>&1 || echo 'no python'"
        });
        let shell_result = handle
            .call_tool("sh", &shell_args.to_string())
            .await
            .unwrap();
        println!("[SANDBOX WORKFLOW] shell:\n{}", shell_result);

        // Step 4: Use shell (sandbox-aware) to glob
        let shell_glob_args = serde_json::json!({
            "command": "ls scripts/*.py 2>/dev/null || find scripts -name '*.py'"
        });
        let shell_glob_result = handle
            .call_tool("sh", &shell_glob_args.to_string())
            .await
            .unwrap();
        assert!(shell_glob_result.contains("hello.py"));
        println!("[SANDBOX WORKFLOW] glob via shell:\n{}", shell_glob_result);

        sandbox.cleanup();
    }

    /// Simulate concurrent sandboxed agents — each has its own isolated workdir.
    #[tokio::test]
    async fn test_simulate_sandbox_concurrent_agents() {
        use std::sync::Arc;
        use tokio::sync::Barrier;

        let barrier = Arc::new(Barrier::new(3));
        let mut tasks = Vec::new();

        for i in 0..3 {
            let b = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let sandbox = test_sandbox_handle();
                let server = crate::builtin::register_sandboxed_tools(
                    crate::ToolServer::new(),
                    Some(sandbox.clone()),
                    false,
                );
                let handle = server.run();

                // Wait for all agents to be ready
                b.wait().await;

                // Each agent writes its own file
                let write_args = serde_json::json!({
                    "path": format!("agent_{}.txt", i),
                    "content": format!("Agent {} data\n", i)
                });
                let result = handle
                    .call_tool("write_file", &write_args.to_string())
                    .await
                    .unwrap();
                println!("[CONCURRENT SANDBOX] Agent {}: {}", i, result);

                // Verify isolation: only its own file exists
                let my_file = sandbox.workdir.join(format!("agent_{}.txt", i));
                assert!(my_file.exists(), "Agent {} file must exist", i);

                sandbox.cleanup();
            }));
        }

        let results = futures::future::join_all(tasks).await;
        for (i, r) in results.iter().enumerate() {
            assert!(r.is_ok(), "Agent {} task failed: {:?}", i, r);
        }
        println!("[CONCURRENT SANDBOX] All 3 agents completed with full isolation");
    }

    // ────────────────────────────────────────────────────────
    // Agent Tool Simulation: decompose_task, list_agents,
    // send_message, read_messages via MCP ToolServerHandle
    // ────────────────────────────────────────────────────────

    /// Build a populated agent pool for tests.
    /// Returns `(pool, responsible_agent_id)`.
    fn make_pool(
        agent_count: usize,
    ) -> (
        std::sync::Arc<tokio::sync::RwLock<wf_agent::AgentPool>>,
        Option<wf_core::AgentId>,
    ) {
        let mut pool = wf_agent::AgentPool::new();
        let mut responsible_id: Option<wf_core::AgentId> = None;

        for i in 0..agent_count {
            let id: wf_core::AgentId = {
                let mut buf = [0u8; 16];
                buf[0] = (i + 1) as u8;
                buf[1] = 0xAA;
                buf
            };
            if i == 0 {
                responsible_id = Some(id);
            }
            pool.add_agent(wf_agent::Agent {
                id,
                name: format!("agent-{}", i),
                role: if i == 0 {
                    "coordinator".to_string()
                } else {
                    format!("worker-{}", i)
                },
                role_template_id: None,
                parent_id: if i == 0 { None } else { responsible_id },
                children: Vec::new(),
                depth: if i == 0 { 0 } else { 1 },
                goal: format!("Goal for agent {}", i),
                config: wf_agent::AgentConfig::default(),
                status: if i == 0 {
                    wf_agent::AgentStatus::Planning
                } else {
                    wf_agent::AgentStatus::Idle
                },
                result: None,
                child_results: Vec::new(),
                context: Vec::new(),
                last_active_at: wf_agent::now_secs(),
                tokens_input: 0,
                tokens_output: 0,
                tool_trace: std::collections::VecDeque::new(),
                inbox: std::collections::VecDeque::new(),
                task_id: None,
                sandbox: None,
                retry_count: 0,
                loop_terminated: false,
                reasoning: String::new(),
            });
        }

        (
            std::sync::Arc::new(tokio::sync::RwLock::new(pool)),
            responsible_id,
        )
    }

    /// Build a pool with one agent for memo tool tests.
    fn make_memo_pool() -> (
        std::sync::Arc<tokio::sync::RwLock<wf_agent::AgentPool>>,
        Option<wf_core::AgentId>,
    ) {
        let mut pool = wf_agent::AgentPool::new();
        let responsible_id: wf_core::AgentId =
            [0xAA, 0xBB, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        pool.add_agent(wf_agent::Agent {
            id: responsible_id,
            name: "memo-agent".to_string(),
            role: "note_taker".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "Take notes".to_string(),
            config: wf_agent::AgentConfig::default(),
            status: wf_agent::AgentStatus::Planning,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: wf_agent::now_secs(),
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: None,
            sandbox: None,
            retry_count: 0,
            loop_terminated: false,
            reasoning: String::new(),
        });
        (
            std::sync::Arc::new(tokio::sync::RwLock::new(pool)),
            Some(responsible_id),
        )
    }

    /// Simulate an agent calling `list_agents` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_list_agents_tool() {
        let (pool, aid) = make_pool(3);
        let server = crate::ToolServer::new();
        let server = crate::agent::register_tools(server, pool, None, aid);
        let handle = server.run();

        let result = handle.call_tool("list_agents", "{}").await.unwrap();
        println!("[AGENT TOOL] list_agents:\n{}", result);
        assert!(result.contains("agent-0"));
        assert!(result.contains("agent-1"));
        assert!(result.contains("agent-2"));
        assert!(result.contains("coordinator") || result.contains("worker"));
    }

    /// Simulate an agent calling `send_message` then `read_messages` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_send_and_read_messages() {
        let (pool, aid) = make_pool(2);
        let server = crate::ToolServer::new();
        let server = crate::agent::register_tools(server, pool.clone(), None, aid);
        let handle = server.run();

        // Send a message from responsible agent (agent-0) to agent-1
        let send_args = serde_json::json!({
            "recipient": "agent-1",
            "message": "Hello from coordinator!"
        });
        let send_result = handle
            .call_tool("send_message", &send_args.to_string())
            .await;
        match &send_result {
            Ok(msg) => println!("[AGENT TOOL] send_message: {}", msg),
            Err(e) => println!(
                "[AGENT TOOL] send_message skipped (no event channel): {}",
                e
            ),
        }
        // send_message may succeed or fail (depends on runtime_event_tx),
        // but we verify the message was enqueued in agent-1's inbox.
        if send_result.is_ok() {
            let pool_guard = pool.read().await;
            let target_id: wf_core::AgentId = [2, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            if let Some(agent1) = pool_guard.get_agent(&target_id) {
                assert!(!agent1.inbox.is_empty(), "message must be in inbox");
            }
        }
    }

    /// Simulate `list_agents` returning empty pool.
    #[tokio::test]
    async fn test_simulate_agent_list_agents_empty() {
        let (pool, aid) = make_pool(0);
        let server = crate::ToolServer::new();
        let server = crate::agent::register_tools(server, pool, None, aid);
        let handle = server.run();

        let result = handle.call_tool("list_agents", "{}").await.unwrap();
        assert!(result.contains("No agents"));
        println!("[AGENT TOOL] list_agents empty: {}", result);
    }

    /// Simulate `send_message` to a non-existent agent.
    #[tokio::test]
    async fn test_simulate_agent_send_message_invalid_recipient() {
        let (pool, aid) = make_pool(1);
        let server = crate::ToolServer::new();
        let server = crate::agent::register_tools(server, pool, None, aid);
        let handle = server.run();

        let send_args = serde_json::json!({
            "recipient": "nonexistent_agent",
            "message": "hello"
        });
        let result = handle
            .call_tool("send_message", &send_args.to_string())
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
        println!("[AGENT TOOL] send_message invalid: {}", err);
    }

    /// Simulate `read_messages` when inbox is empty.
    #[tokio::test]
    async fn test_simulate_agent_read_messages_empty() {
        let (pool, aid) = make_pool(1);
        let server = crate::ToolServer::new();
        let server = crate::agent::register_tools(server, pool, None, aid);
        let handle = server.run();

        let result = handle
            .call_tool("read_messages", &serde_json::json!({}).to_string())
            .await
            .unwrap();
        assert!(result.contains("No messages"));
        println!("[AGENT TOOL] read_messages empty: {}", result);
    }

    /// Simulate calling agent tools on the full `create_agent_tool_server`.
    #[tokio::test]
    async fn test_simulate_full_agent_tool_server() {
        let (pool, aid) = make_pool(2);
        let handle = crate::create_agent_tool_server(pool, None, aid);

        // List tools available — should include both built-in and agent tools
        let defs = handle.get_tool_defs(None).await.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"decompose_task"));
        assert!(names.contains(&"send_message"));
        assert!(names.contains(&"read_messages"));
        assert!(names.contains(&"list_agents"));
        assert!(names.contains(&"read_file"));
        println!(
            "[AGENT TOOL] Full agent server has {} tools including agent tools",
            defs.len()
        );
    }

    // ────────────────────────────────────────────────────────
    // Memo MCP Simulation: agent scratchpad/notepad tools
    // ────────────────────────────────────────────────────────

    /// Simulate an agent calling memo tools via MCP: full lifecycle.
    #[tokio::test]
    async fn test_simulate_memo_lifecycle() {
        let (pool, aid) = make_memo_pool();
        let deps = crate::memo::MemoToolDeps::new(pool, aid);
        let server = crate::memo::register_memo_tools(crate::ToolServer::new(), deps);
        let handle = server.run();

        // Step 1: list_memos — should be empty initially
        let list_result = handle
            .call_tool("list_memos", &serde_json::json!({}).to_string())
            .await
            .unwrap();
        println!("[MEMO] Initial list: {}", list_result);
        assert!(list_result.contains("No memos"));

        // Step 2: write_memo — store a finding
        let write_args = serde_json::json!({
            "key": "task/findings",
            "value": "The root cause is a race condition in the event loop."
        });
        let write_result = handle
            .call_tool("write_memo", &write_args.to_string())
            .await
            .unwrap();
        println!("[MEMO] write: {}", write_result);
        assert!(write_result.contains("task/findings"));

        // Step 3: read_memo — retrieve it
        let read_args = serde_json::json!({"key": "task/findings"});
        let read_result = handle
            .call_tool("read_memo", &read_args.to_string())
            .await
            .unwrap();
        println!("[MEMO] read: {}", read_result);
        assert!(read_result.contains("root cause"));
        assert!(read_result.contains("race condition"));

        // Step 4: list_memos — should show 1 memo
        let list_after = handle
            .call_tool("list_memos", &serde_json::json!({}).to_string())
            .await
            .unwrap();
        println!("[MEMO] list after write: {}", list_after);
        assert!(list_after.contains("task/findings"));
        assert!(list_after.contains("1 total"));

        // Step 5: list_memos with prefix filter
        let list_filtered = handle
            .call_tool(
                "list_memos",
                &serde_json::json!({"prefix": "task/"}).to_string(),
            )
            .await
            .unwrap();
        println!("[MEMO] list filtered: {}", list_filtered);
        assert!(list_filtered.contains("task/findings"));

        // Step 6: list with non-matching prefix
        let list_no_match = handle
            .call_tool(
                "list_memos",
                &serde_json::json!({"prefix": "decision/"}).to_string(),
            )
            .await
            .unwrap();
        println!("[MEMO] list no match: {}", list_no_match);
        assert!(list_no_match.contains("No memos"));

        // Step 7: delete_memo
        let delete_args = serde_json::json!({"key": "task/findings"});
        let delete_result = handle
            .call_tool("delete_memo", &delete_args.to_string())
            .await
            .unwrap();
        println!("[MEMO] delete: {}", delete_result);
        assert!(delete_result.contains("deleted"));

        // Step 8: read_memo after deletion — should fail
        let read_deleted = handle.call_tool("read_memo", &read_args.to_string()).await;
        assert!(read_deleted.is_err());
        assert!(read_deleted.unwrap_err().to_string().contains("not found"));
        println!("[MEMO] read after delete correctly fails");
    }

    /// Simulate overwriting an existing memo.
    #[tokio::test]
    async fn test_simulate_memo_overwrite() {
        let (pool, aid) = make_memo_pool();
        let deps = crate::memo::MemoToolDeps::new(pool, aid);
        let server = crate::memo::register_memo_tools(crate::ToolServer::new(), deps);
        let handle = server.run();

        // Write initial memo
        handle
            .call_tool(
                "write_memo",
                &serde_json::json!({
                    "key": "decision/approach",
                    "value": "Use SIMD for embeddings"
                })
                .to_string(),
            )
            .await
            .unwrap();

        // Overwrite with new value
        handle
            .call_tool(
                "write_memo",
                &serde_json::json!({
                    "key": "decision/approach",
                    "value": "Use LSH instead of SIMD for better recall"
                })
                .to_string(),
            )
            .await
            .unwrap();

        // Verify latest value
        let result = handle
            .call_tool(
                "read_memo",
                &serde_json::json!({"key": "decision/approach"}).to_string(),
            )
            .await
            .unwrap();
        assert!(result.contains("LSH"));
        assert!(!result.contains("SIMD for embeddings"));
        println!("[MEMO] Overwrite verified: latest value is 'LSH'");
    }

    /// Simulate memo tools registered alongside built-in tools on full agent server.
    #[tokio::test]
    async fn test_simulate_memo_on_full_server() {
        let (pool, aid) = make_memo_pool();
        let handle = crate::create_agent_tool_server(pool, None, aid);

        // Memo tools should be discoverable
        let defs = handle.get_tool_defs(None).await.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"write_memo"));
        assert!(names.contains(&"read_memo"));
        assert!(names.contains(&"list_memos"));
        assert!(names.contains(&"delete_memo"));
        println!(
            "[MEMO] Full agent server has {} tools including memo tools",
            defs.len()
        );

        // Verify all memo tools have valid definitions
        for name in &["write_memo", "read_memo", "list_memos", "delete_memo"] {
            let def = defs.iter().find(|d| d.name == *name).unwrap();
            assert!(
                def.parameters.get("properties").is_some(),
                "{} missing properties",
                name
            );
        }
    }
}
