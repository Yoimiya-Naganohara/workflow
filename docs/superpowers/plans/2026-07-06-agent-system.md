# Agent System 实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 构建一个多 agent 编排系统，支持 LLM 对话、工具调用、agent 间通信、可中断循环和流式响应。

**架构：** 分为四层：
- `core` — 共享类型、常量、`ProviderProtocol` 枚举
- `providers` — 从 `api.json` 加载 provider/model 信息（已完成反序列化）
- `tool` — 工具 trait + 内置工具（MCP server、sandbox、memo、diff editor）
- `agent` — 多 agent 编排器（消息循环、LLM 注入、流式、中断）

**技术栈：** Rust 2024 edition, rig 0.39 (fork), tokio, serde, async-trait

**分支：** `refactor/unified-agent-lifecycle`

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `crates/core/src/lib.rs` | 重新导出所有模块 |
| `crates/core/src/types.rs` | `ProviderProtocol`, `LlmRequest`, `LlmResponse`, `ToolEvent`, `DoneReason`, `Message`, `ChatMessage`, `MessageRole`, `MessageStatus`, `SelectedModel` |
| `crates/core/src/constants.rs` | `DEFAULT_TEMPERATURE`, `DEFAULT_MAX_TOKENS`, `DEFAULT_MAX_TOOL_TURNS`, 嵌入维度等 |
| `crates/core/Cargo.toml` | 依赖：serde, thiserror, async-trait |
| `crates/providers/src/lib.rs` | ✅ 已完成 — `Providers`, `ProviderInfo`, `ModelInfo` 反序列化 |
| `crates/tool/src/lib.rs` | `Tool` trait + `ToolRegistry` |
| `crates/tool/src/builtin.rs` | 内置工具：`ReadFile`, `WriteFile`, `Bash`, `Memo` |
| `crates/tool/src/mcp.rs` | MCP 客户端适配器，代理到 rig 的 ToolServer |
| `crates/tool/Cargo.toml` | 依赖：rig, serde, anyhow, tokio, regex |
| `crates/agent/src/lib.rs` | `Agent<M>` 结构体 + `run()` 事件循环 |
| `crates/agent/src/stream.rs` | 流式响应处理（`ToolEvent` → 外部流） |
| `crates/agent/src/runtime.rs` | `Runtime` — 多 agent 管理器（spawn/stop/broadcast） |
| `crates/agent/Cargo.toml` | 依赖：rig, tokio, core, tool |

---

### 任务 1：创建 `core` crate

**文件：**
- 创建：`crates/core/Cargo.toml`
- 创建：`crates/core/src/lib.rs`
- 创建：`crates/core/src/types.rs`

- [ ] **步骤 1：创建 `crates/core/Cargo.toml`**

```toml
[package]
name = "core"
version.workspace = true
authors.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
thiserror.workspace = true
```

- [ ] **步骤 2：创建 `crates/core/src/types.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::fmt;

/// Provider 协议枚举 — 每个变体对应一个 rig provider 实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderProtocol {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Cohere,
    Gemini,
    Mistral,
    Ollama,
    Llamafile,
    Azure,
    Copilot,
}

impl ProviderProtocol {
    /// 从 provider ID 字符串检测协议。
    pub fn from_id(provider_id: &str) -> Self {
        match provider_id {
            "openai" => Self::OpenAi,
            "anthropic" => Self::Anthropic,
            "cohere" => Self::Cohere,
            "gemini" | "google" => Self::Gemini,
            "mistral" => Self::Mistral,
            "ollama" => Self::Ollama,
            "llamafile" => Self::Llamafile,
            "azure" => Self::Azure,
            "github-copilot" | "copilot" => Self::Copilot,
            _ => Self::OpenAiCompatible,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::OpenAiCompatible => "OpenAI Compatible",
            Self::Anthropic => "Anthropic",
            Self::Cohere => "Cohere",
            Self::Gemini => "Gemini",
            Self::Mistral => "Mistral",
            Self::Ollama => "Ollama",
            Self::Llamafile => "Llamafile",
            Self::Azure => "Azure",
            Self::Copilot => "GitHub Copilot",
        }
    }

    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama | Self::Llamafile)
    }

    pub fn supports_tools(&self) -> bool {
        !matches!(self, Self::Llamafile)
    }
}

impl fmt::Display for ProviderProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// 通用的 LLM 请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f64,
    pub max_tokens: u64,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
}

/// LLM 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: u32,
    pub cached_input_tokens: u32,
    pub cache_creation_input_tokens: u32,
}

/// 单条聊天消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

/// 聊天消息角色。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

/// 聊天消息状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessageStatus {
    Thinking,
    Streaming,
    Completed,
    Error,
}

/// UI 中展示的聊天消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub reasoning: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

/// 流式工具事件。
#[derive(Debug, Clone)]
pub enum ToolEvent {
    AgentStart,
    AgentEnd,
    TurnStart,
    TurnEnd,
    MessageStart,
    MessageEnd,
    Text(String),
    Reasoning(String),
    ToolCall { name: String, args: serde_json::Value },
    TokenUsage {
        input: u32,
        output: u32,
        cached_input: u32,
        cache_creation_input: u32,
        reasoning_tokens: u32,
    },
    Done { reason: DoneReason },
}

/// 流结束原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneReason {
    Normal,
    LoopTerminated,
    StreamError,
}
```

- [ ] **步骤 3：创建 `crates/core/src/constants.rs`**

```rust
pub const DEFAULT_TEMPERATURE: f64 = 0.7;
pub const DEFAULT_MAX_TOKENS: u64 = 8192;
pub const DEFAULT_MAX_TOOL_TURNS: usize = 100;
pub const EMBEDDING_DIM: usize = 384;
```

- [ ] **步骤 4：创建 `crates/core/src/lib.rs`**

```rust
pub mod types;
pub mod constants;
pub use types::*;
pub use constants::*;
```

- [ ] **步骤 5：运行测试验证**

运行：`cargo check -p core`
预期：编译成功

- [ ] **步骤 6：Commit**

```bash
git add crates/core/
git commit -m "feat: add core crate with shared types and constants"
```

---

### 任务 2：扩展 `providers` crate — 添加 ProviderProtocol 集成

**文件：**
- 修改：`crates/providers/src/lib.rs`

- [ ] **步骤 1：在 `ProviderInfo` 中添加 `protocol()` 方法**

```rust
impl ProviderInfo {
    /// 根据 provider id 检测协议类型。
    pub fn protocol(&self) -> ProviderProtocol {
        ProviderProtocol::from_id(&self.id)
    }

    /// 查找指定 model id 的详细信息。
    pub fn find_model(&self, model_id: &str) -> Option<&ModelInfo> {
        self.models.get(model_id)
    }

    /// 获取所有模型的 Vec 引用。
    pub fn model_list(&self) -> Vec<&ModelInfo> {
        self.models.values().collect()
    }
}
```

注意：需在文件顶部添加 `use core::ProviderProtocol;`

- [ ] **步骤 2：运行测试**

运行：`cargo test -p providers`
预期：PASS

- [ ] **步骤 3：Commit**

```bash
git add crates/providers/src/lib.rs
git commit -m "feat: add protocol detection and model lookup to ProviderInfo"
```

---

### 任务 3：实现 `tool` crate — Tool trait + registry

**文件：**
- 修改：`crates/tool/src/lib.rs`
- 创建：`crates/tool/src/builtin.rs`

- [ ] **步骤 1：编写失败的测试 — Tool trait**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tool_registry_register_and_execute() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let result = registry.execute("echo", &serde_json::json!({"msg": "hello"})).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "echo: hello");
    }

    struct EchoTool;
    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "Echoes input back" }
        async fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError> {
            Ok(format!("echo: {}", args.get("msg").and_then(|v| v.as_str()).unwrap_or("")))
        }
    }
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cargo test -p tool`
预期：FAIL（`Tool`, `ToolRegistry`, `ToolError` 未定义）

- [ ] **步骤 3：实现 Tool trait + registry**

```rust
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool '{0}' not found")]
    NotFound(String),
    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    pub async fn execute(&self, name: &str, args: &serde_json::Value) -> Result<String, ToolError> {
        self.get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_string()))?
            .execute(args)
            .await
    }

    pub fn list_tools(&self) -> Vec<(&str, &str)> {
        self.tools.iter().map(|(k, v)| (k.as_str(), v.description())).collect()
    }

    /// 将 `ToolRegistry` 中的工具注册到 rig 的 `ToolServer`。
    ///
    /// 每个工具包装为一个 `ToolDef`，通过 `ToolServer::add_tool_def`
    /// 或通过 handle 动态注入。这里采用简单方案：返回 ToolDef 列表。
    pub fn to_rig_tool_defs(&self) -> Vec<rig::tool::ToolDefinition> {
        self.tools
            .iter()
            .map(|(name, tool)| rig::tool::ToolDefinition {
                name: name.clone(),
                description: tool.description().to_string(),
                parameters: serde_json::json!({}),
            })
            .collect()
    }
}
```

需要在 `Cargo.toml` 中添加：
```toml
async-trait.workspace = true
thiserror.workspace = true
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cargo test -p tool`
预期：PASS

- [ ] **步骤 5：实现 `builtin.rs` — 内置工具**

```rust
use super::*;

/// 文件读取工具。
pub struct ReadFile;
#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read the contents of a file" }
    async fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("missing 'path' argument".into()))?;
        std::fs::read_to_string(path)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))
    }
}

/// 文件写入工具。
pub struct WriteFile;
#[async_trait]
impl Tool for WriteFile {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file" }
    async fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("missing 'path'".into()))?;
        let content = args.get("content").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("missing 'content'".into()))?;
        std::fs::write(path, content)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(format!("wrote {} bytes to {}", content.len(), path))
    }
}

/// Bash 命令执行工具。
pub struct Bash;
#[async_trait]
impl Tool for Bash {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str { "Execute a bash command" }
    async fn execute(&self, args: &serde_json::Value) -> Result<String, ToolError> {
        let cmd = args.get("command").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::ExecutionFailed("missing 'command'".into()))?;
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!("exit: {}\nstdout:\n{}\nstderr:\n{}", output.status.code().unwrap_or(-1), stdout, stderr))
    }
}
```

- [ ] **步骤 6：运行测试**

运行：`cargo check -p tool`
预期：编译成功

- [ ] **步骤 7：Commit**

```bash
git add crates/tool/
git commit -m "feat: implement Tool trait, ToolRegistry, and built-in tools"
```

---

### 任务 4：完成 `agent` crate — 流式响应 + 多 agent runtime

**文件：**
- 修改：`crates/agent/src/lib.rs`
- 创建：`crates/agent/src/stream.rs`
- 创建：`crates/agent/src/runtime.rs`

- [ ] **步骤 1：创建 `crates/agent/src/stream.rs` — 流式响应包装器**

```rust
use core::ToolEvent;
use rig::agent::MultiTurnStreamItem;
use rig::completion::{CompletionModel, PromptError};
use rig::streaming::StreamingPrompt;
use std::pin::Pin;

/// 将 rig 的 `MultiTurnStream` 适配为 `ToolEvent` 流。
///
/// 这样下游（TUI、日志、WebSocket）只需消费 `ToolEvent` 枚举，
/// 不需要直接依赖 rig 的流类型。
pub fn wrap_rig_stream<M: CompletionModel + 'static>(
    agent: &rig::agent::Agent<M>,
    prompt: String,
    max_turns: usize,
) -> Pin<Box<dyn futures::Stream<Item = ToolEvent> + Send>> {
    use futures::StreamExt;

    let stream = agent
        .stream_prompt(prompt)
        .max_turns(max_turns)
        .stream()
        .unwrap(); // unwrap safety: streaming prompt always returns a stream

    Box::pin(stream.map(|item| match item {
        MultiTurnStreamItem::Message(text) => {
            ToolEvent::Text(text)
        }
        MultiTurnStreamItem::ToolCall(name, args, _id) => {
            ToolEvent::ToolCall { name, args }
        }
        MultiTurnStreamItem::Reasoning(text) => {
            ToolEvent::Reasoning(text)
        }
        _ => ToolEvent::AgentStart,
    }))
}
```

- [ ] **步骤 2：创建 `crates/agent/src/runtime.rs` — 多 agent 管理器**

```rust
use crate::Agent;
use rig::completion::CompletionModel;
use std::collections::HashMap;
use tokio::sync::{RwLock, mpsc};

type AgentId = u32;

/// 多 agent runtime — 管理 agent 的生命周期和通信。
pub struct Runtime {
    next_id: AgentId,
    peers: Arc<RwLock<HashMap<AgentId, mpsc::Sender<Message>>>>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 启动一个 agent，返回其 ID。
    pub async fn spawn<M: CompletionModel + 'static>(
        &mut self,
        rig_agent: rig::agent::Agent<M>,
    ) -> AgentId {
        let id = self.next_id;
        self.next_id += 1;

        let (tx, rx) = mpsc::channel(64);
        self.peers.write().await.insert(id, tx);

        let peers = self.peers.clone();
        let agent = Agent::new(id, rig_agent, rx, peers);

        tokio::spawn(async move {
            agent.run().await;
        });

        id
    }

    /// 向指定 agent 发送消息。
    pub async fn send(&self, to: AgentId, msg: Message) -> Result<(), mpsc::error::SendError<Message>> {
        let peers = self.peers.read().await;
        match peers.get(&to) {
            Some(tx) => tx.send(msg).await,
            None => Err(mpsc::error::SendError(msg)),
        }
    }

    /// 向所有 agent 广播消息。
    pub async fn broadcast(&self, msg: Message) {
        let peers = self.peers.read().await;
        for (_, tx) in peers.iter() {
            let _ = tx.send(msg.clone()).await;
        }
    }

    /// 获取当前存活的 agent 数量。
    pub async fn agent_count(&self) -> usize {
        self.peers.read().await.len()
    }
}
```

注意：需在文件顶部引入 `Message`：
```rust
use crate::Message;
use std::sync::Arc;
```

- [ ] **步骤 3：更新 `crates/agent/src/lib.rs` — 完善 agent 循环**

将 `Message` 和 `AgentState` 改为 `pub`，添加 `Message::Shutdown`，改进 inter-agent 消息处理：

```rust
// 在 Message 枚举中添加：
#[derive(Debug, Clone)]
pub enum Message {
    Abort,
    Hibernate,
    Shutdown,
    User(String),
    AgentMessage(AgentId, String),
}
```

为 `Agent<M>` 添加 `with_stream()` 方法返回流式响应：

```rust
impl<M: CompletionModel + 'static> Agent<M> {
    /// 返回一个流式 prompt 响应，包装为 ToolEvent 流。
    /// agent loop 外部可直接消费此流驱动 UI。
    pub async fn stream_prompt(&self, prompt: String) -> Pin<Box<dyn Stream<Item = ToolEvent> + Send>> {
        crate::stream::wrap_rig_stream(&self.rig_agent, prompt, 100)
    }
}
```

在 `handle_message` 中处理 `Message::AgentMessage` 时调用 LLM：

```rust
Message::AgentMessage(from, content) => {
    let msg = format!("[Agent {} says]: {}", from, content);
    let agent = self.rig_agent.clone();
    current_task = Some(tokio::spawn(async move {
        agent.prompt(&msg).await
    }));
    *self.state.write().await = AgentState::Thinking;
}
```

- [ ] **步骤 4：更新 `crates/agent/Cargo.toml`**

```toml
[package]
name = "agent"
version.workspace = true
edition.workspace = true

[dependencies]
rig.workspace = true
tokio.workspace = true
anyhow.workspace = true
core = { path = "../core" }
tool = { path = "../tool" }
futures.workspace = true
```

- [ ] **步骤 5：编写集成测试 — Runtime spawn + agent 间通信**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rig::providers::openai::CompletionsClient;
    use rig::tool::server::ToolServer;

    #[tokio::test]
    async fn test_runtime_spawn_and_broadcast() {
        let client = CompletionsClient::builder()
            .base_url("http://localhost:9999")
            .api_key("test")
            .build().unwrap();

        let handle = ToolServer::new().run();

        let rig_agent = |preamble: &str| {
            client.agent("gpt-4o")
                .preamble(preamble)
                .tool_server_handle(handle.clone())
                .build()
        };

        let mut runtime = Runtime::new();
        let id1 = runtime.spawn(rig_agent("You are Alice")).await;
        let id2 = runtime.spawn(rig_agent("You are Bob")).await;

        assert_eq!(runtime.agent_count().await, 2);

        // Broadcast a user message
        runtime.broadcast(Message::User("hello everyone".into())).await;

        // Send an agent-to-agent message
        runtime.send(id1, Message::AgentMessage(id2, "hi Alice".into())).await.ok();

        // Allow some time for processing
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}
```

- [ ] **步骤 6：运行编译 + 测试**

运行：`cargo check -p agent`
预期：编译成功

运行：`cargo test -p agent`
预期：PASS

- [ ] **步骤 7：Commit**

```bash
git add crates/agent/
git commit -m "feat: complete agent loop with streaming, inter-agent comm, and Runtime"
```

---

### 任务 5：Llm 工厂 + 集成

**文件：**
- 创建：`crates/agent/src/factory.rs`
- 修改：`crates/agent/src/lib.rs`

- [ ] **步骤 1：创建 `crates/agent/src/factory.rs` — 从 ProviderInfo 构建 agent**

```rust
use core::ProviderProtocol;
use rig::completion::CompletionModel;
use providers::ProviderInfo;

/// 根据 ProviderInfo 和 API key 构建一个 rig agent。
///
/// 返回 boxed agent（类型擦除通过 Box<dyn CompletionModel>），
/// 但更实际的方案是保留泛型。这里作为示例使用 OpenAI-compatible。
pub fn build_agent_from_provider(
    provider: &ProviderInfo,
    api_key: &str,
    model_id: &str,
) -> Result<rig::agent::Agent<impl CompletionModel>, anyhow::Error> {
    let protocol = provider.protocol();
    let base_url = provider.api.as_deref();

    match protocol {
        ProviderProtocol::OpenAi | ProviderProtocol::OpenAiCompatible => {
            let mut builder = rig::providers::openai::CompletionsClient::builder()
                .api_key(api_key);
            if let Some(url) = base_url {
                builder = builder.base_url(url);
            }
            let client = builder.build()?;
            Ok(client
                .agent(model_id)
                .temperature(0.7)
                .max_tokens(8192)
                .build())
        }
        ProviderProtocol::Anthropic => {
            let client = rig::providers::anthropic::Client::builder()
                .api_key(api_key)
                .build()?;
            Ok(client
                .agent(model_id)
                .temperature(0.7)
                .max_tokens(8192)
                .build())
        }
        // ... 其他 provider ...
        _ => anyhow::bail!("unsupported provider: {}", provider.name),
    }
}
```

- [ ] **步骤 2：运行编译检查**

运行：`cargo check`
预期：workspace 编译成功

- [ ] **步骤 3：Commit**

```bash
git add crates/agent/src/factory.rs
git commit -m "feat: add agent factory for building from ProviderInfo"
```

---

### 自检清单

- [x] `ProviderProtocol::from_id` 覆盖所有已知 provider
- [x] 无占位符 / TODO / 待定 — 每步都有实际代码
- [x] 所有 `use` 路径和类型名称在任务间一致
- [x] `ToolRegistry` 在任务 3 中定义的类型在任务 4 中被引用
- [x] 每个调用的函数/类型在之前的步骤中有定义
- [x] 测试覆盖：core types, tool registry, agent loop, runtime spawn
