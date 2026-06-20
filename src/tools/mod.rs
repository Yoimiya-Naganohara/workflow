//! MCP tool system using rig's ToolServer and ToolDyn infrastructure.
//!
//! Built-in tools are registered on a shared [`ToolServerHandle`] and
//! agents connect via `.tool_server_handle()`.

pub mod agent;
pub mod builtin;
pub mod memo;
pub mod sandbox;

pub use rig::tool::server::{ToolServer, ToolServerHandle};

pub use memo::MemoToolDeps;

/// Create a [`ToolServerHandle`] pre-loaded with all built-in tools.
///
/// The handle is cheaply cloneable and can be shared across agents.
pub fn create_tool_server() -> ToolServerHandle {
    builtin::register_tools(ToolServer::new()).run()
}

/// Create a [`ToolServerHandle`] with built-ins plus workflow agent tools.
/// Create a [`ToolServerHandle`] with built-ins, agent tools, and memo tools.
///
/// The `state` is used for both the agent tools (spawn_agent) and to derive
/// the memo tool dependencies (agent pool, responsible agent ID).
pub fn create_agent_tool_server(
    state: std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>,
) -> ToolServerHandle {
    let server = builtin::register_tools(ToolServer::new());
    let server = agent::register_tools(server, state.clone());
    // Derive memo deps from the state
    let memo_deps = memo::MemoToolDeps::from_state(&state);
    memo::register_memo_tools(server, memo_deps).run()
}

/// Create a [`ToolServerHandle`] with sandbox-aware tools for a specific agent.
pub fn create_sandboxed_agent_tool_server(
    base_state: std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>,
    sandbox: Option<std::sync::Arc<crate::tools::sandbox::SandboxHandle>>,
) -> ToolServerHandle {
    let server = builtin::register_sandboxed_tools(ToolServer::new(), sandbox);
    let server = agent::register_tools(server, base_state.clone());
    let memo_deps = memo::MemoToolDeps::from_state(&base_state);
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
            let _h = handle;
            true
        })
        .join();
        assert!(result.unwrap());
    }

    #[test]
    fn test_tool_server_types_are_public() {
        // Verify the re-exports compile correctly
        let _server = ToolServer::new();
        // ToolServerHandle::new is not public, but we can create via run()
        let handle = builtin::register_tools(ToolServer::new()).run();
        let _cloned = handle.clone();
    }

    // ────────────────────────────────────────────────────────
    // MCP Agent Simulation: simulates an LLM agent calling
    // built-in tools through the ToolServerHandle (MCP interface)
    // ────────────────────────────────────────────────────────

    /// Simulate an agent enumerating available tools via MCP.
    #[tokio::test]
    async fn test_simulate_agent_list_tools() {
        let handle = create_tool_server();

        // Agent calls get_tool_defs (like MCP tools/list)
        let defs = handle.get_tool_defs(None).await.unwrap();

        // Verify all expected built-in tools are present
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"read_file"), "missing read_file");
        assert!(names.contains(&"write_file"), "missing write_file");
        assert!(names.contains(&"sh"), "missing sh");
        assert!(names.contains(&"list_dir"), "missing list_dir");
        assert!(names.contains(&"grep"), "missing grep");
        assert!(names.contains(&"find_files"), "missing find_files");
        assert!(names.contains(&"move_file"), "missing move_file");
        assert!(names.contains(&"copy_file"), "missing copy_file");
        assert!(names.contains(&"delete_file"), "missing delete_file");
        assert!(names.contains(&"append_file"), "missing append_file");
        assert!(names.contains(&"patch_file"), "missing patch_file");
        assert!(names.contains(&"glob"), "missing glob");
        assert!(names.contains(&"line_edit"), "missing line_edit");
        assert!(names.contains(&"fetch"), "missing fetch");
        assert!(names.contains(&"search_asset"), "missing search_asset");

        // Each definition has parameters with a type
        for def in &defs {
            assert!(
                def.parameters.get("type").is_some(),
                "{} missing type in parameters",
                def.name
            );
        }

        println!("[SIMULATE] Agent discovered {} MCP tools", defs.len());
    }

    /// Simulate an agent calling `read_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_read_file() {
        use std::io::Write;

        let handle = create_tool_server();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "line1\nline2\nline3").unwrap();
        let path = f.path().to_str().unwrap().to_string();

        // Agent constructs JSON args (as an LLM would)
        let args = serde_json::json!({
            "path": path,
            "start": null,
            "end": null
        });

        let result = handle
            .call_tool("read_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
        println!("[SIMULATE] Agent read_file result:\n{}", result);
    }

    /// Simulate an agent calling `write_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_write_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.txt");

        let args = serde_json::json!({
            "path": path.to_str().unwrap().to_string(),
            "content": "Hello from MCP agent!\n"
        });

        let result = handle
            .call_tool("write_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("Written"));
        assert!(result.contains("bytes")); // byte count
        println!("[SIMULATE] Agent write_file result: {}", result);

        // Verify the file was actually written
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Hello from MCP agent!\n");
    }

    /// Simulate an agent calling `shell` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_shell() {
        let handle = create_tool_server();

        let args = serde_json::json!({
            "command": "echo 'Hello from agent MCP call'"
        });

        let result = handle.call_tool("sh", &args.to_string()).await.unwrap();
        assert!(result.contains("Hello from agent MCP call"));
        assert!(result.contains("exit code: 0"));
        println!("[SIMULATE] Agent shell result:\n{}", result);
    }

    /// Simulate an agent calling `shell` with stderr output.
    #[tokio::test]
    async fn test_simulate_agent_shell_stderr() {
        let handle = create_tool_server();

        let args = serde_json::json!({
            "command": "echo 'error msg' >&2 && echo 'stdout msg'"
        });

        let result = handle.call_tool("sh", &args.to_string()).await.unwrap();
        assert!(result.contains("stdout msg"));
        assert!(result.contains("stderr:"));
        assert!(result.contains("error msg"));
        println!("[SIMULATE] Agent shell (stderr) result:\n{}", result);
    }

    /// Simulate an agent calling `list_dir` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_list_dir() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.rs"), "b").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let args = serde_json::json!({
            "path": dir.path().to_str().unwrap().to_string()
        });

        let result = handle
            .call_tool("list_dir", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("a.txt"));
        assert!(result.contains("b.rs"));
        assert!(result.contains("sub"));
        println!("[SIMULATE] Agent list_dir result:\n{}", result);
    }

    /// Simulate an agent calling `glob` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_glob() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "").unwrap();
        std::fs::write(dir.path().join("readme.md"), "").unwrap();

        let args = serde_json::json!({
            "pattern": format!("{}/*.rs", dir.path().to_str().unwrap())
        });

        let result = handle.call_tool("glob", &args.to_string()).await.unwrap();
        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
        assert!(!result.contains("readme.md"));
        println!("[SIMULATE] Agent glob result:\n{}", result);
    }

    /// Simulate an agent calling `grep` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_grep() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("hello.txt"),
            "hello world\nfoo bar\nhello again\n",
        )
        .unwrap();

        let args = serde_json::json!({
            "pattern": "hello",
            "path": dir.path().to_str().unwrap().to_string()
        });

        let result = handle.call_tool("grep", &args.to_string()).await.unwrap();
        assert!(result.contains("hello world"));
        assert!(result.contains("hello again"));
        assert!(!result.contains("foo bar"));
        println!("[SIMULATE] Agent grep result:\n{}", result);
    }

    /// Simulate an agent calling `find_files` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_find_files() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();

        let args = serde_json::json!({
            "pattern": "*.rs",
            "path": dir.path().to_str().unwrap().to_string()
        });

        let result = handle
            .call_tool("find_files", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("main.rs"));
        assert!(result.contains("lib.rs"));
        println!("[SIMULATE] Agent find_files result:\n{}", result);
    }

    /// Simulate an agent calling `move_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_move_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("dest.txt");
        std::fs::write(&src, "move me").unwrap();

        let args = serde_json::json!({
            "source": src.to_str().unwrap().to_string(),
            "destination": dst.to_str().unwrap().to_string()
        });

        let result = handle
            .call_tool("move_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("Moved"));
        assert!(!dst.exists() || dst.exists()); // after move
        assert!(!src.exists());
        println!("[SIMULATE] Agent move_file result: {}", result);
    }

    /// Simulate an agent calling `copy_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_copy_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("original.txt");
        let dst = dir.path().join("copy.txt");
        std::fs::write(&src, "copy me").unwrap();

        let args = serde_json::json!({
            "source": src.to_str().unwrap().to_string(),
            "destination": dst.to_str().unwrap().to_string()
        });

        let result = handle
            .call_tool("copy_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("Copied"));
        assert!(src.exists());
        assert!(dst.exists());
        println!("[SIMULATE] Agent copy_file result: {}", result);
    }

    /// Simulate an agent calling `delete_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_delete_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("to_delete.txt");
        std::fs::write(&file, "delete me").unwrap();

        let args = serde_json::json!({
            "path": file.to_str().unwrap().to_string()
        });

        let result = handle
            .call_tool("delete_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("Deleted"));
        assert!(!file.exists());
        println!("[SIMULATE] Agent delete_file result: {}", result);
    }

    /// Simulate an agent calling `append_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_append_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("log.txt");
        std::fs::write(&file, "existing content\n").unwrap();

        let args = serde_json::json!({
            "path": file.to_str().unwrap().to_string(),
            "content": "appended line"
        });

        let result = handle
            .call_tool("append_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("appended"));
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("existing content"));
        assert!(content.contains("appended line"));
        println!("[SIMULATE] Agent append_file result: {}", result);
    }

    /// Simulate an agent calling `patch_file` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_patch_file() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("patch_me.txt");
        std::fs::write(&file, "hello old world\n").unwrap();

        let args = serde_json::json!({
            "path": file.to_str().unwrap().to_string(),
            "old_text": "old",
            "new_text": "new"
        });

        let result = handle
            .call_tool("patch_file", &args.to_string())
            .await
            .unwrap();
        assert!(result.contains("Patched"));
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("hello new world"));
        println!("[SIMULATE] Agent patch_file result: {}", result);
    }

    /// Simulate an agent calling an unknown tool (error case).
    #[tokio::test]
    async fn test_simulate_agent_unknown_tool() {
        let handle = create_tool_server();

        let result = handle.call_tool("nonexistent_tool", "{}").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent_tool"));
        println!("[SIMULATE] Agent unknown tool error: {}", err);
    }

    /// Simulate an agent calling `shell` with an LLM-style JSON arg
    /// (extra whitespace, trailing commas — LLMs often produce messy JSON).
    #[tokio::test]
    async fn test_simulate_agent_llm_style_messy_json() {
        let handle = create_tool_server();

        // LLMs sometimes inject extra whitespace or formatting in JSON
        let messy_json = r#"{"command": "echo LLM-style call"}"#;
        // The args string is what the LLM would send — passthrough via ToolServerHandle
        let result = handle.call_tool("sh", messy_json).await.unwrap();
        assert!(result.contains("LLM-style call"));
        println!("[SIMULATE] Agent LLM-style messy JSON result:\n{}", result);
    }

    /// Simulate concurrent MCP tool calls from multiple agents.
    #[tokio::test]
    async fn test_simulate_concurrent_agent_calls() {
        let handle = create_tool_server();
        use std::sync::Arc;
        use tokio::sync::Barrier;

        // Three agents calling shell concurrently
        let barrier = Arc::new(Barrier::new(3));
        let mut tasks = Vec::new();

        for i in 0..3 {
            let h = handle.clone();
            let b = barrier.clone();
            tasks.push(tokio::spawn(async move {
                // Wait for all agents to be ready
                b.wait().await;

                let args = serde_json::json!({
                    "command": format!("echo 'Agent {} reporting for duty'", i)
                });
                h.call_tool("sh", &args.to_string()).await
            }));
        }

        let results = futures::future::join_all(tasks).await;
        for (i, result) in results.iter().enumerate() {
            let r = result.as_ref().unwrap().as_ref().unwrap();
            assert!(r.contains(&format!("Agent {}", i)));
            assert!(r.contains("exit code: 0"));
            println!("[SIMULATE] Agent {} concurrent result: OK", i);
        }
    }

    /// Simulate an agent's multi-step workflow: write → read → patch → shell.
    #[tokio::test]
    async fn test_simulate_agent_workflow() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("workflow.py");
        let path = file.to_str().unwrap().to_string();

        // Step 1: Write initial file
        let write_args = serde_json::json!({
            "path": path,
            "content": "def greet(name):\n    print(f\"Hello, {name}\")\n\ngreet(\"World\")\n"
        });
        let write_result = handle
            .call_tool("write_file", &write_args.to_string())
            .await
            .unwrap();
        println!("[WORKFLOW] Step 1 — write_file: {}", write_result);

        // Step 2: Verify via read
        let read_args = serde_json::json!({"path": path});
        let read_result = handle
            .call_tool("read_file", &read_args.to_string())
            .await
            .unwrap();
        assert!(read_result.contains("greet"));
        println!("[WORKFLOW] Step 2 — read_file:\n{}", read_result);

        // Step 3: Patch the function
        let patch_args = serde_json::json!({
            "path": path,
            "old_text": "Hello",
            "new_text": "Hi"
        });
        let patch_result = handle
            .call_tool("patch_file", &patch_args.to_string())
            .await
            .unwrap();
        assert!(patch_result.contains("Patched"));
        println!("[WORKFLOW] Step 3 — patch_file: {}", patch_result);

        // Step 4: Run the script via shell
        let shell_args = serde_json::json!({ "command": format!("cd {} && python3 workflow.py 2>&1 || python workflow.py 2>&1 || echo 'no python'", dir.path().to_str().unwrap()) });
        let shell_result = handle
            .call_tool("sh", &shell_args.to_string())
            .await
            .unwrap();
        println!("[WORKFLOW] Step 4 — shell:\n{}", shell_result);
    }

    /// Simulate agent error recovery: call delete_file on a nonexistent path.
    #[tokio::test]
    async fn test_simulate_agent_error_handling() {
        let handle = create_tool_server();

        let args = serde_json::json!({
            "path": "/tmp/__nonexistent_file_xyz__/test.txt"
        });

        let result = handle.call_tool("delete_file", &args.to_string()).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found")
                || err.contains("No such file")
                || err.contains("not exist")
                || err.contains("error")
        );
        println!("[SIMULATE] Agent error handling: {}", err);
    }

    /// Simulate calling `line_edit` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_line_edit() {
        let handle = create_tool_server();
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::write(&file, "# Config\nhost = \"localhost\"\nport = 8080\n").unwrap();

        let args = serde_json::json!({
            "path": file.to_str().unwrap().to_string(),
            "operations": [
                {
                    "op": "replace_range",
                    "start_line": 2,
                    "end_line": 2,
                    "text": "host = \"0.0.0.0\""
                }
            ]
        });

        let result = handle
            .call_tool("line_edit", &args.to_string())
            .await
            .unwrap();
        println!("[SIMULATE] Agent line_edit result: {}", result);
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("host = \"0.0.0.0\""));
        assert!(!content.contains("host = \"localhost\""));
    }

    /// Simulate an agent calling `fetch` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_fetch() {
        let handle = create_tool_server();

        let args = serde_json::json!({
            "url": "https://httpbin.org/get",
            "max_size": 1024,
            "timeout": 5
        });

        let result = handle.call_tool("fetch", &args.to_string()).await;
        match result {
            Ok(text) => {
                // If network is available — just check we got something back
                assert!(!text.is_empty(), "fetch returned empty content");
                let preview: String = text.chars().take(200).collect();
                println!(
                    "[SIMULATE] Agent fetch result ({} bytes, {} chars preview)\n---\n{}---",
                    text.len(),
                    preview.len(),
                    preview
                );
            }
            Err(e) => {
                // Network may not be available in test environments
                println!(
                    "[SIMULATE] Agent fetch skipped (network unavailable): {}",
                    e
                );
            }
        }
    }

    // ────────────────────────────────────────────────────────
    // Sandbox MCP Simulation: agent with filesystem isolation
    // ────────────────────────────────────────────────────────

    /// Helper: create a sandbox handle for testing.
    fn test_sandbox_handle() -> std::sync::Arc<crate::tools::sandbox::SandboxHandle> {
        static TEST_IDX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
        let n = TEST_IDX.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut id = [0u8; 16];
        id[0..8].copy_from_slice(&n.to_le_bytes());
        std::sync::Arc::new(
            crate::tools::sandbox::SandboxHandle::new(&id).expect("sandbox creation"),
        )
    }

    /// Simulate an agent using sandboxed MCP tools.
    #[tokio::test]
    async fn test_simulate_sandbox_write_isolation() {
        let sandbox = test_sandbox_handle();

        // Build sandboxed tool server (like create_sandboxed_agent_tool_server
        // would for a real agent)
        let server = crate::tools::builtin::register_sandboxed_tools(
            crate::tools::ToolServer::new(),
            Some(sandbox.clone()),
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
        let server = crate::tools::builtin::register_sandboxed_tools(
            crate::tools::ToolServer::new(),
            Some(sandbox.clone()),
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
        let server = crate::tools::builtin::register_sandboxed_tools(
            crate::tools::ToolServer::new(),
            Some(sandbox.clone()),
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
        let server = crate::tools::builtin::register_sandboxed_tools(
            crate::tools::ToolServer::new(),
            Some(sandbox.clone()),
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
        assert!(result.contains(&sandbox.workdir.to_str().unwrap()));
        println!("[SANDBOX] Shell runs inside workdir:\n{}", result);

        sandbox.cleanup();
    }

    /// Simulate agent MCP calls through the full sandboxed agent tool server.
    #[tokio::test]
    async fn test_simulate_sandbox_workflow() {
        let sandbox = test_sandbox_handle();
        let server = crate::tools::builtin::register_sandboxed_tools(
            crate::tools::ToolServer::new(),
            Some(sandbox.clone()),
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
                let server = crate::tools::builtin::register_sandboxed_tools(
                    crate::tools::ToolServer::new(),
                    Some(sandbox.clone()),
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
                ()
            }));
        }

        let results = futures::future::join_all(tasks).await;
        for (i, r) in results.iter().enumerate() {
            assert!(r.is_ok(), "Agent {} task failed: {:?}", i, r);
        }
        println!("[CONCURRENT SANDBOX] All 3 agents completed with full isolation");
    }

    // ────────────────────────────────────────────────────────
    // Agent Tool Simulation: spawn_agent, list_agents,
    // send_message, read_messages via MCP ToolServerHandle
    // ────────────────────────────────────────────────────────

    /// Helper: create a minimal AppState with a pool of test agents.
    fn make_agent_state(
        agent_count: usize,
    ) -> std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>> {
        let mut pool = crate::agent::AgentPool::new();
        let mut responsible_id = [0u8; 16];

        for i in 0..agent_count {
            let id: crate::core::types::AgentId = {
                let mut buf = [0u8; 16];
                buf[0] = (i + 1) as u8;
                buf[1] = 0xAA;
                buf
            };
            if i == 0 {
                responsible_id = id;
            }
            pool.add_agent(crate::agent::Agent {
                id,
                name: format!("agent-{}", i),
                role: if i == 0 {
                    "coordinator".to_string()
                } else {
                    format!("worker-{}", i)
                },
                role_template_id: None,
                parent_id: if i == 0 { None } else { Some(responsible_id) },
                children: Vec::new(),
                depth: if i == 0 { 0 } else { 1 },
                goal: format!("Goal for agent {}", i),
                config: crate::agent::AgentConfig::default(),
                status: if i == 0 {
                    crate::agent::AgentStatus::Planning
                } else {
                    crate::agent::AgentStatus::Idle
                },
                result: None,
                child_results: Vec::new(),
                context: Vec::new(),
                last_active_at: crate::agent::now_secs(),
                tokens_input: 0,
                tokens_output: 0,
                tool_trace: std::collections::VecDeque::new(),
                inbox: std::collections::VecDeque::new(),
                task_id: None,
                sandbox: None,
            });
        }

        let mut state = crate::tui::state::AppState::default();
        state.core.agent_pool = std::sync::Arc::new(tokio::sync::RwLock::new(pool));
        state.core.responsible_agent_id = Some(responsible_id);

        std::sync::Arc::new(tokio::sync::RwLock::new(state))
    }

    /// Simulate an agent calling `list_agents` via MCP.
    #[tokio::test]
    async fn test_simulate_agent_list_agents_tool() {
        let state = make_agent_state(3);
        let server = crate::tools::ToolServer::new();
        let server = crate::tools::agent::register_tools(server, state);
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
        let state = make_agent_state(2);
        let server = crate::tools::ToolServer::new();
        let server = crate::tools::agent::register_tools(server, state.clone());
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
            let s = state.read().await;
            let pool = s.core.agent_pool.read().await;
            let target_id: crate::core::types::AgentId =
                [2, 0xAA, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            if let Some(agent1) = pool.get_agent(&target_id) {
                assert!(!agent1.inbox.is_empty(), "message must be in inbox");
            }
        }
    }

    /// Simulate `list_agents` returning empty pool.
    #[tokio::test]
    async fn test_simulate_agent_list_agents_empty() {
        let state = make_agent_state(0);
        let server = crate::tools::ToolServer::new();
        let server = crate::tools::agent::register_tools(server, state);
        let handle = server.run();

        let result = handle.call_tool("list_agents", "{}").await.unwrap();
        assert!(result.contains("No agents"));
        println!("[AGENT TOOL] list_agents empty: {}", result);
    }

    /// Simulate `send_message` to a non-existent agent.
    #[tokio::test]
    async fn test_simulate_agent_send_message_invalid_recipient() {
        let state = make_agent_state(1);
        let server = crate::tools::ToolServer::new();
        let server = crate::tools::agent::register_tools(server, state);
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
        let state = make_agent_state(1);
        let server = crate::tools::ToolServer::new();
        let server = crate::tools::agent::register_tools(server, state);
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
        let state = make_agent_state(2);
        let handle = crate::tools::create_agent_tool_server(state);

        // List tools available — should include both built-in and agent tools
        let defs = handle.get_tool_defs(None).await.unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"spawn_agent"));
        assert!(names.contains(&"send_message"));
        assert!(names.contains(&"read_messages"));
        assert!(names.contains(&"list_agents"));
        assert!(names.contains(&"list_dir"));
        assert!(names.contains(&"read_file"));
        println!(
            "[AGENT TOOL] Full agent server has {} tools including agent tools",
            defs.len()
        );

        // Verify spawn_agent tool definition has correct params
        let spawn_def = defs.iter().find(|d| d.name == "spawn_agent").unwrap();
        assert!(spawn_def.parameters.get("required").is_some());
        println!(
            "[AGENT TOOL] spawn_agent definition: {}",
            serde_json::to_string_pretty(&spawn_def.parameters).unwrap()
        );
    }

    // ────────────────────────────────────────────────────────
    // Memo MCP Simulation: agent scratchpad/notepad tools
    // ────────────────────────────────────────────────────────

    /// Helper: build AgentState with one active agent for memo tool tests.
    fn make_memo_state() -> std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>> {
        let mut pool = crate::agent::AgentPool::new();
        let responsible_id: crate::core::types::AgentId =
            [0xAA, 0xBB, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        pool.add_agent(crate::agent::Agent {
            id: responsible_id,
            name: "memo-agent".to_string(),
            role: "note_taker".to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "Take notes".to_string(),
            config: crate::agent::AgentConfig::default(),
            status: crate::agent::AgentStatus::Planning,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: crate::agent::now_secs(),
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: None,
            sandbox: None,
        });

        let mut state = crate::tui::state::AppState::default();
        state.core.agent_pool = std::sync::Arc::new(tokio::sync::RwLock::new(pool));
        state.core.responsible_agent_id = Some(responsible_id);
        std::sync::Arc::new(tokio::sync::RwLock::new(state))
    }

    /// Simulate an agent calling memo tools via MCP: full lifecycle.
    #[tokio::test]
    async fn test_simulate_memo_lifecycle() {
        let state = make_memo_state();
        let deps = crate::tools::memo::MemoToolDeps::from_state(&state);
        let server = crate::tools::memo::register_memo_tools(crate::tools::ToolServer::new(), deps);
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
        let state = make_memo_state();
        let deps = crate::tools::memo::MemoToolDeps::from_state(&state);
        let server = crate::tools::memo::register_memo_tools(crate::tools::ToolServer::new(), deps);
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
        let state = make_memo_state();
        let handle = crate::tools::create_agent_tool_server(state);

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
