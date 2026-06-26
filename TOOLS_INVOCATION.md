# Tool Invocation Architecture

完整调用链 — 从 LLM 决定调用工具到工具结果返回的全流程。

## 总体调用链

```
LLM Agent (rig)
     │
     │ ① LLM 返回 tool_call (function name + JSON args)
     ▼
rig::agent::Agent::stream_chat()
     │
     │ ② 多轮循环 multi_turn()
     │    每次 LLM 返回 tool_call → 执行 → 结果喂回 LLM
     ▼
ToolServerHandle::call_tool(tool_name, args_json)
     │
     │ ③ 按名称查找已注册的 Tool
     ▼
Tool::call(args)         ← 具体的工具实现
     │                       (read_file, sh, grep, diff_edit, ...)
     │ ④ 执行操作 (文件 I/O / shell / 网络)
     ▼
Result<Output, ToolCallError>  ← 返回给 rig 框架
     │
     │ ⑤ rig 将结果构造为 tool_result 消息
     │    再次调用 LLM (含 tool_call + tool_result)
     ▼
LLM 继续生成或产生 FinalResponse
     │
     │ ⑥ multi_turn 流式输出事件
     ▼
wrap_tool_stream()  ← 在 src/llm/chat.rs
     │
     │ ⑦ 将 rig MultiTurnStreamItem 转换为 ToolEvent 流
     │    · Text → 文本块
     │    · ToolCall → 通知（已执行完毕）
     │    · Reasoning → 推理过程
     │    · TokenUsage → token 计数
     │    · FinalResponse → Done
     │ ⑧ 循环检测：相同 tool+args 3次 → LoopTerminated
     │    单工具超限 → LoopTerminated
     │    总调用超限 → LoopTerminated
     ▼
process_tool_stream()  ← 在 src/runtime/agent_stream.rs
     │
     │ ⑨ 消费 ToolEvent 流
     │    · Text → 拼接响应
     │    · Reasoning → 写入 agent.reasoning
     │    · ToolCall → 记录到 agent.tool_trace
     │    · TokenUsage → 累加到 agent.tokens_input/output
     │    · Done → 结束
     │ ⑩ 启发式错误检测：在文本中扫描 error 关键词
     ▼
(text, tool_bitmap)  → 存储到 agent result + experience
```

## 文件级调用链

```
src/runtime/agent_exec.rs
  AgentRuntime::execute_agent_detached()
    │
    │ 1. 获取 provider / role_template / goal
    │ 2. 构造 system_prompt (含 memo + inbox_hint)
    │
    ├─ [有 tool_server] ──→
    │   provider.chat_with_tools_stream_mcp(
    │       model, system, goal, history, tool_server_handle
    │   )
    │       │
    │       ▼ src/llm/chat.rs
    │   chat_with_tools_stream_mcp()
    │       → mcp_stream_arm! 宏
    │       → client.agent(model)
    │           .preamble(system)
    │           .tool_server_handle(handle)  ← rig MCP 绑定
    │           .build()
    │       → agent.stream_chat(message, history)
    │           .multi_turn(MAX_TOOL_TURNS)  ← rig 多轮框架
    │       → wrap_tool_stream()  → ToolChatStream
    │
    ├─ [无 tool_server] ──→
    │   provider.chat(model, system, goal)  ← 纯文本
    │
    ▼
  process_tool_stream(stream, agent_id, agent_pool)
    → (text, tools_used_bitmap)
```

## rig 内部多轮执行流程

```
Agent::stream_chat(msg, history)
  │
  │──→ LLM 调用 (含 tool 定义)
  │     │
  │     ├─ LLM 返回 tool_call
  │     │   │
  │     │   ├─ ToolServerHandle::call_tool(name, args)
  │     │   │   └─ 在注册表中查找 name → Tool::call(args)
  │     │   │       └─ 返回 Result<Output, ToolError>
  │     │   │
  │     │   └─ 构造 tool_result 消息
  │     │       └─ 继续循环 → 再次调用 LLM
  │     │
  │     └─ LLM 返回文本内容 (FinalResponse)
  │         └─ 生成流事件 → 结束
  │
  └──→ 流式输出 MultiTurnStreamItem:
       ├─ StreamAssistantItem::Text       → ToolEvent::Text
       ├─ StreamAssistantItem::ToolCall   → ToolEvent::ToolCall
       ├─ StreamAssistantItem::Reasoning  → ToolEvent::Reasoning
       ├─ CompletionCall                  → ToolEvent::TokenUsage
       └─ FinalResponse                   → ToolEvent::Done
```

## 安全边界（循环检测）

`src/llm/chat.rs` 的 `wrap_tool_stream()` 实现了三层防护：

| 边界 | 阈值 | 效果 |
|------|------|------|
| 总工具调用次数 | `MAX_TOOL_CALLS_PER_STREAM` (常数) | 超限 → "Tool call limit reached" |
| 单工具调用次数 | `MAX_CALLS_PER_TOOL` (常数) | 超限 → "Tool loop detected: called X times" |
| 相同 tool+args | 连续 3 次相同 | 超限 → "called 3 times with identical arguments" |

所有边界触发后都注入 system 消息：
```
<system>Tool call limit reached: {summary}. Tool calls stopped.
Summarize what you have found so far.</system>
```

## 工具注册查找机制

```
ToolServer 内部是一个 HashMap<String, Arc<dyn Tool>>:

register_tools(server):
  server.tool(ReadFile)         →  insert "read_file"
  server.tool(WriteFile)        →  insert "write_file"
  server.tool(Shell)            →  insert "sh"
  server.tool(ExtractJson)      →  insert "extract_json"
  server.tool(DiffEdit)         →  insert "diff_edit"
  ...

  server.run() → ToolServerHandle {
      tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>
  }

call_tool(name, args):
  let tool = tools.get(name)?  ← O(1) 查找
  let args: T = serde_json::from_str(args)?  ← JSON 反序列化
  tool.call(args).await  ← 执行
```

## 三种 ToolServer 构建路径

```
create_tool_server()
  → builtin::register_tools()    ← 无 agent/memo 工具
  → 用于 CLI 模式 / 纯工具调用

create_agent_tool_server(state)
  → builtin::register_tools()    ← 内置工具
  → agent::register_tools()      ← agent 工具
  → memo::register_memo_tools()  ← memo 工具
  → 用于有 agent pool 的主服务器

create_sandboxed_agent_tool_server(state, sandbox)
  → builtin::register_sandboxed_tools(sandbox, true)  ← 隔离工具
  → agent::register_tools()                            ← agent 工具
  → memo::register_memo_tools()                        ← memo 工具
  → 用于子 agent 的沙箱执行环境
```

## 运行时状态流

```
ToolCall 事件流过以下状态：

┌──────────────┐     ┌──────────────────┐     ┌──────────────────┐
│ agent_stream │────►│  agent.tool_trace │────►│  tool_bitmap     │
│ .rs          │     │  VecDeque<Record> │     │  u64 bitmask     │
│              │     │                   │     │                  │
│ 计数工具调用 │     │ 记录: name, args, │     │ 编译时按工具名  │
│ 记录错误     │     │ status, timestamp │     │ 分配 bit 位置    │
│              │     │ MAX_TOOL_TRACE=50 │     │ 用作 experience  │
└──────────────┘     └──────────────────┘     │ 检索加速         │
                                             └──────────────────┘
```

## 关键设计决策

### 1. MCP 模式优于原生工具绑定

```
// 使用 tool_server_handle (MCP 模式):
client.agent(model).tool_server_handle(handle)

// 而非静态绑定:
client.agent(model).tool(ReadFile).tool(Shell).tool(Grep)
```

**优势：** 运行时动态增删工具，无需重编译；不同 agent 可以有不同工具集；沙箱隔离可插拔。

### 2. 事件流 vs 直接调用

工具执行结果不直接返回给调用者，而是通过事件流 `ToolEvent` 传递：

- ** producer:** `wrap_tool_stream()` → `ToolChatStream`
- **consumer:** `process_tool_stream()` → `(text, bitmap)`

这种设计支持：
- 非阻塞处理（工具调用是并发的）
- 循环检测（在事件流中拦截）
- UI 更新（TUI 可以订阅 ToolEvent）

### 3. 工具位图 (bitmap)

每个工具在编译时分配一个 bit 位置：

```rust
fn tool_bit(name: &str) -> u64 {
    match name {
        "read_file"    => 1 << 0,
        "write_file"   => 1 << 1,
        "sh"           => 1 << 2,
        "grep"         => 1 << 3,
        "glob"         => 1 << 4,
        "fetch"        => 1 << 5,
        "patch_file"   => 1 << 6,
        "line_edit"    => 1 << 7,
        "list_dir"     => 1 << 8,
        "find_files"   => 1 << 9,
        "move_file"    => 1 << 10,
        "copy_file"    => 1 << 11,
        "delete_file"  => 1 << 12,
        "append_file"  => 1 << 13,
        "search_asset" => 1 << 14,
        "extract_json" => 1 << 15,
        "diff_edit"    => 1 << 16,
        "spawn_agent"  => 1 << 28,
        "send_message" => 1 << 29,
        "read_messages"=> 1 << 30,
        "list_agents"  => 1 << 31,
        _ => 0,
    }
}
```

工具位图用于 `experience` 检索加速：只需 bitwise AND 即可过滤相关经验。

## 发现的问题

### P3: ToolEvent::ToolCall 的 result 字段为空

在 `wrap_tool_stream()` 中：
```rust
yield ToolEvent::ToolCall {
    name: tool_name,
    args,
    result: String::new(),   // ← 永远为空
};
```

`ToolCall` 事件已经是工具执行完成后的通知，但 result 被丢弃了。如果 future 需要记录工具输出（如审计），没有直接途径。

### P3: 工具位图 (tool_bit) 未自动更新

新增工具（`extract_json`、`diff_edit`）可能没有对应的 bit 分配。如果 `tool_bit` 返回 0，该工具调用不会影响 experience 检索位图。
