# Tool System Design

## Architecture Overview

The tool system exposes MCP (Model Context Protocol) tools to LLM agents through
[rig](https://docs.rs/rig/latest/rig/)'s `Tool` trait and `ToolServer` infrastructure.

```
┌──────────────────────────────────────────────────────┐
│                  ToolServerHandle                     │
│  ┌──────────────────────────────────────────────────┐ │
│  │  create_tool_server()                            │ │
│  │  create_agent_tool_server(state)                 │ │
│  │  create_sandboxed_agent_tool_server(state, sb)   │ │
│  └──────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────┤
│  Tools                                                │
│  ┌──────────────────────────────────────────────────┐ │
│  │  builtin.rs  : 15 tools (file I/O, shell, etc.)  │ │
│  │  agent.rs    :  4 tools (spawn, message, list)   │ │
│  │  memo.rs     :  4 tools (key-value scratchpad)   │ │
│  └──────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────┤
│  Isolation Layer: sandbox.rs                          │
│  - workdir/  : write zone                             │
│  - src -> ~/project/ : read-only symlink              │
│  - asset_indices : semantic chunk cache               │
└──────────────────────────────────────────────────────┘
```

**23 tools** across 3 domains, all sharing `ToolCallError` for uniform error handling.

---

## Core Trait Contract

Every tool implements `rig::tool::Tool`:

```rust
pub trait Tool: Send + Sync {
    const NAME: &'static str;         // snake_case tool name
    type Error: std::error::Error;     // always ToolCallError
    type Args: DeserializeOwned;       // JSON → struct
    type Output: Serialize;            // struct → JSON

    fn definition(&self, prompt: String) -> ToolDefinition;
    fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error>;
}
```

**Pattern every tool follows:**
1. `Args` struct with `#[derive(Deserialize)]`
2. `definition()` returns JSON Schema for LLM consumption
3. `call()` performs the operation, returns structured output
4. Errors always wrapped in `ToolCallError(String)`

---

## Tool Catalog

### Built-in Domain (`builtin.rs`)

| Tool | Purpose | Args | Output |
|------|---------|------|--------|
| `read_file` | Read file with optional line range | path, start?, end? | String (truncated at 10KB) |
| `write_file` | Write/create file | path, content, start?, end? | String (confirmation + preview) |
| `sh` | Execute shell command | command | String (stdout+stderr, 100KB cap) |
| `list_dir` | List directory with sizes | path | String (file count + sizes) |
| `grep` | Regex search, single file or directory | pattern, path, max_results? | String (line:match) |
| `find_files` | Glob pattern file finder | pattern, root?, max_results? | String (paths + sizes) |
| `move_file` | Move/rename with cross-device fallback | source, destination | String ("Moved ...") |
| `copy_file` | Copy file or directory tree | source, destination | String ("Copied ...") |
| `delete_file` | Delete with safety checks | path, recursive?, force? | String ("Deleted ...") |
| `append_file` | Append with optional newline | path, content, newline? | String (confirmation) |
| `patch_file` | Search-and-replace with count limit | path, old_text, new_text, count? | String (replacements) |
| `glob` | Glob pattern resolution | pattern, root? | String (matches + sizes) |
| `line_edit` | Structured line operations | path, dry_run?, operations[] | String (diff preview) |
| `fetch` | HTTP(S) fetch | url, max_size?, timeout? | String (status + body) |
| `search_asset` | Semantic search in indexed output | asset_id, query, top_k? | String (chunks + scores) |

**Operations**: `insert_after`, `insert_before`, `replace_range`, `delete_range`

### Agent Domain (`agent.rs`)

| Tool | Purpose | Args | Output |
|------|---------|------|--------|
| `spawn_agent` | Delegate work to a child agent | role, goal, reason, expected_output?, blocking? | `SpawnAgentOutput` (Running/Rejected) |
| `send_message` | Inter-agent communication | recipient, message | String |
| `read_messages` | Drain inbox (FIFO) | max? | String |
| `list_agents` | List pool status | (none) | String |

### Memo Domain (`memo.rs`)

| Tool | Purpose | Args | Output |
|------|---------|------|--------|
| `write_memo` | Persist key-value note | key, value | String (confirmation) |
| `read_memo` | Retrieve note by key | key | String (value + age) |
| `list_memos` | List all notes, optional prefix filter | prefix? | String |
| `delete_memo` | Delete note by key | key | String |

Memos are **per-role** key-value stores with 8KB value limit, persisted to disk.

---

## Sandbox Architecture (`sandbox.rs`)

```
~/.workflow/sandbox/{agent_id:8}/
├── work/          ← writable: all writes, compilation, shell cwd
└── src → /project/   ← read-only symlink (never written through)
```

**Path resolution rules:**

| Write type | Relative path | Absolute path |
|-----------|---------------|---------------|
| **Read** | `workdir/{path}` ✅ | Checked against workdir ∪ source_root ✅ |
| **Write** | `workdir/{path}` ✅ | Rejected (must be relative) ❌ |

**Embedded asset indexing:**
- Shell/read_file outputs >5KB → `create_embedded_asset()`
- Content hashed → `{tool}_{hash}` asset ID
- 384-d embeddings computed via fastembed ONNX engine
- Stored in `SandboxHandle.asset_indices` (in-memory HashMap)
- Retrievable via `search_asset(asset_id, query)` → AVX2+FMA cosine similarity

---

## Factory Functions (`mod.rs`)

| Function | Built-in | Agent | Memo | Sandbox |
|----------|----------|-------|------|---------|
| `create_tool_server()` | ✅ | ❌ | ❌ | ❌ |
| `create_agent_tool_server(state)` | ✅ | ✅ | ✅ | ❌ |
| `create_sandboxed_agent_tool_server(state, sb)` | ✅ | ✅ | ✅ | ✅ |
| `create_tool_server_with(extra)` | ✅ | ❌ | ❌ | ❌ |

---

## Design Patterns

### 1. Blocking I/O Offload
File operations use `spawn_blocking_fs()` to avoid starving the Tokio runtime:
```rust
async fn spawn_blocking_fs<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, ToolCallError> {
    spawn_blocking(f).await
        .map_err(|e| ToolCallError(format!("Blocking pool join failed: {}", e)))?
        .map_err(ToolCallError)
}
```

### 2. Structured Error Recovery
`line_edit` tool uses per-operation error handling with rollback:
```rust
match apply_operation(&mut lines, op, ends_with_newline) {
    Ok(desc) => stats.record(desc),
    Err(e) => {
        // Partial rollback: restore original
        let _ = std::fs::write(&args.path, &content);
        return Err(ToolCallError(err_msg));
    }
}
```

### 3. Atomic File Writes
`line_edit` and `write_file` use temp-file + rename pattern:
```rust
let tmp_path = format!("{}.tmp.{}", args.path, std::process::id());
std::fs::write(&tmp_path, &new_content)?;
std::fs::rename(&tmp_path, &args.path)?;
```

### 4. Sandbox Path Safety
Two-stage path resolution: canonicalize nearest ancestor, then check boundary:
```rust
fn canonicalise_or_reject(&self, path: &Path) -> Result<PathBuf> {
    // Fast path: existing file
    if let Ok(canon) = path.canonicalize() {
        return self.check_boundary(canon);
    }
    // Walk up ancestors until existing
    let mut ancestor = path.parent();
    while let Some(parent) = ancestor {
        if parent.exists() {
            let canon_parent = parent.canonicalize()?;
            let checked = self.check_boundary(canon_parent)?;
            // ... reconstruct path
        }
        ancestor = parent.parent();
    }
}
```

---

## Known Design Issues

### P1: `spawn_agent` blocking mode is non-functional
The `blocking` parameter exists in the schema but is never used. The tool always returns
`SpawnAgentOutput::Running` immediately regardless. Fix requires wiring the agent lifecycle
completion signal back through the runtime event loop.

### P1: `search_asset` registered in non-sandbox servers
`register_tools()` calls `register_sandboxed_tools(server, None)`, which registers
`SearchAsset { sandbox: None }`. Non-sandbox agents see the tool in their list but
every call fails with "requires a sandboxed agent context".

### P2: `tools/mod.rs` duplicate doc comments
Comment formatting issue — first line is empty, second has the real content.

### P2: No structured JSON extraction tool
Agents have no tool for parsing semi-structured output (logs, configs, CSVs) into JSON.

### P2: No decision log tool
Memos are ephemeral key-value stores; there is no versioned decision record.

---

## Adding a New Tool

### Step 1: Define Args struct
```rust
#[derive(Deserialize)]
pub struct MyToolArgs {
    pub input: String,
    pub option: Option<bool>,
}
```

### Step 2: Define tool struct + Tool impl
```rust
pub struct MyTool;

impl Tool for MyTool {
    const NAME: &'static str = "my_tool";
    type Error = ToolCallError;
    type Args = MyToolArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition { ... }
    async fn call(&self, args: Self::Args) -> Result<String, ToolCallError> { ... }
}
```

### Step 3: Register
```rust
server.tool(MyTool)
```

### Step 4: Test (add to `mod.rs` tests or `builtin.rs` `tests` module)
```rust
#[tokio::test]
async fn test_my_tool() {
    let handle = create_tool_server();
    let result = handle.call_tool("my_tool", r#"{"input":"test"}"#).await.unwrap();
    assert!(result.contains("test"));
}
```

---

## Testing Strategy

MCP agent simulation tests live in `mod.rs` — these validate the full round-trip:
1. `get_tool_defs` — discoverability
2. `call_tool` with JSON strings — LLM-style invocation
3. Concurrent calls — thread safety
4. Error cases — `old_text` not found, path escape, system path deletion refused
5. Multi-step workflows — write → read → patch → shell

```
test_simulate_agent_list_tools      → All 15 built-in tools discovered
test_simulate_agent_read_file       → Content matches, metadata intact
test_simulate_agent_workflow        → 4-step pipeline passes
test_simulate_concurrent_agent_calls → 3 concurrent agents, no races
test_simulate_sandbox_write_isolation → Files land in workdir, not real tree
test_simulate_memo_lifecycle        → Write/read/list/delete cycle
```
