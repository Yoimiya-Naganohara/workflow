# Workflow 架构文档

## 一句话

LLM 驱动的多 Agent 协作系统：分层决策管线 + 自动容错 + 持久化检查点 + TUI 终端界面。

## 系统分层

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 1: TUI 界面  (17 文件, ~9,900 行)                        │
│  用户交互、消息渲染、诊断树、命令系统                              │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: 运行时引擎  (17 文件, ~9,400 行)                       │
│  Agent 生命周期、事件循环、任务调度、策略进化                      │
├─────────────────────────────────────────────────────────────────┤
│  Layer 3: Agent 管理层  (4 文件, ~2,000 行)                      │
│  Agent 结构体、池管理、任务分解、暂停队列                          │
├─────────────────────────────────────────────────────────────────┤
│  Layer 4: LLM 接口  (6 文件, ~2,100 行)                          │
│  9 个 Provider、流式聊天、三层循环保护、本地嵌入                   │
├─────────────────────────────────────────────────────────────────┤
│  Layer 5: 决策管线  (7 文件, ~1,700 行)                          │
│  L-1 准入 / L0 熔断 / L1 经验 / L2 审计                         │
├─────────────────────────────────────────────────────────────────┤
│  Layer 6: 工具系统  (5 文件, ~5,600 行)                          │
│  15+ 内置工具、MCP Server、文件沙箱                               │
├─────────────────────────────────────────────────────────────────┤
│  基础模块  (22 文件, ~6,700 行)                                  │
│  core/ types + constants + simd                                 │
│  experience/ 双轨记忆 + 聚类 + 检索                              │
│  persistence/ state.json + session.json                          │
│  checkpoint/ AgentPool + TaskGraph 快照                          │
│  models/ 模型注册表 + provider 配置                              │
│  config/ 统一配置层                                              │
│  reflection/ 自省系统                                            │
└─────────────────────────────────────────────────────────────────┘
```

## 核心数据结构

```
AppState (tui/state.rs)
├── CoreState
│   ├── messages: Vec<ChatMessage>         ←── 对话历史
│   ├── agent_pool: Arc<RwLock<AgentPool>> ←── Agent 池
│   ├── runtime: Option<AgentRuntime>      ←── 运行时
│   ├── tool_server: ToolServerHandle      ←── MCP 工具服务
│   └── runtime_event_tx: Sender           ←── 事件通道
│
├── UiState
│   ├── input, focus, mode                ←── 输入状态
│   ├── budget_used/total                 ←── 预算显示
│   ├── agent_tree_version                ←── 诊断树版本
│   └── think_level, reasoning_effort     ←── 推理控制
│
└── effects: Vec<Effect>                   ←── 异步效果队列

Agent (agent/agent.rs)
├── id, name, role, status, depth          ←── 标识 + 层级
├── context: Vec<Message>                  ←── 对话历史
├── inbox: VecDeque<AgentMessage>          ←── 消息队列
├── tool_trace: VecDeque<ToolCallRecord>   ←── 工具追踪 (✓✗⏳)
├── reasoning: String                      ←── 推理链
├── tokens_input/output                    ←── Token 消耗
├── retry_count                            ←── 重试计数
├── result: Option<String>                 ←── 执行结果
└── sandbox: Option<SandboxHandle>         ←── 文件沙箱

AgentPool (agent/agent.rs)
├── agents: Vec<Agent>                     ←── 所有 Agent
├── role_memos: HashMap<role, memos>       ←── 角色备忘录
├── max_retries, checkpoint_interval       ←── 配置
└── ttl_secs, max_agents                   ←── 清理策略

TaskGraph (runtime/task_graph.rs)
├── nodes: HashMap<TaskId, TaskNode>       ←── DAG 节点
│   └── TaskNode { parent, children,
│         dependencies, status, goal, role,
│         assigned_agent, result }
├── roots: Vec<TaskId>                     ←── 根节点
└── failure_policy: FailurePolicy          ←── 失败传播策略

RuntimeEvent (runtime/event.rs)
├── ActivateAgent { agent_id, parent_id }
├── InboxMessage { agent_id, from, preview, count }
├── SpawnTask { goal, role, parent_agent }
├── TaskCompleted { task_id, result }
├── TaskFailed { task_id, error }
├── ChildCompleted { parent_id, child_id, result }
├── AgentFailed { agent_id, error }
├── AggregationCompleted { agent_id, result }
└── ReadyForAggregation, EscalateTask, MergeTaskResult
```

## 执行流程

### 聊天路径

```
用户输入 → handler.rs
  → Effect::StartChat
  → provider.chat_with_tools_stream_mcp()
  → wrap_tool_stream() (三层循环保护)
  → ToolEvent(Text/Reasoning/ToolCall/TokenUsage/Done)
  → AppEvent(ChatToken/ChatReasoning/...)
  → state.handle_event()
  → TUI 实时渲染
```

### Agent 执行路径

```
事件循环收到 ActivateAgent
  → tokio::spawn(handle_activate_inner)
  → execute_agent_detached()
  → process_tool_stream() (共享方法)
  → retry 检查 (≤3 次)
  → Completed/Failed → parent 通知
```

### 调度路径

```
scheduler.dispatch() 收到新任务
  → task_graph.ready_tasks()
  → StrategyGraph.select_strategy()
  → Decomposition.decompose()
  → RoleSelector.select_role()
  → Pipeline.process_request() (L0→L1→L2)
  → Approved: spawn_agent()
  → Rejected: mark_rejected()
  → Escalation.should_escalate()
  → StrategyGraph.record_trace()
```

### 检查点路径

```
每 50 事件:
  Phase 1 (锁内): serialize_snapshot(&pool, &graph) → bytes
  Phase 2 (锁外): write_snapshot(bytes) → 磁盘

启动时:
  Checkpoint::restore_snapshot()
  → rehydrate_pool() (Running→Idle)
  → task_graph: Running→Ready, Dispatching→Created
```

## 核心不变量

1. Agent 不永久停止: LoopTerminated/StreamError → Complete
2. 重试走完整 Pipeline: 新预算、新 L1/L2 评估
3. 检查点两阶段: 锁内 serialize (μs), 锁外 write (ms)
4. await_agent loop 防止 retry 提前唤醒
5. task_graph 可恢复: Running→Created, 可重新调度
6. tool_trace 使用 try_write: 高频率可丢弃操作
7. RuntimeEvent → mpsc channel: 单向异步不阻塞

## 状态机

```
AgentStatus:
  Idle → Planning → AwaitingChildren / Aggregating → Completed
                                                 → Failed
                                            → Idle (retry)

TaskStatus:
  Created → Ready → Dispatching → Running → Completed
                                        → Failed
                   → Decomposed → (children done) → Completed
  Dispatching → Created (retry)
  Running → Created (retry)

MessageStatus:
  Thinking → Streaming → Completed
                      → Error (RED)
```

## 模块依赖

```
                    runtime
                   /  |  |  \
                  /   |  |   \
                 /    |  |    \
                ▼     ▼  ▼     ▼
             agent  llm  tools  experience
               |     |              |
               ▼     ▼              ▼
               l0   l1/l2         persistence
               |                   |
               ▼                   ▼
           admission         checkpoint + core
```

## 关键文件

| 文件 | 行数 | 职责 |
|------|:----:|------|
| `runtime/runtime_loop.rs` | 1,029 | 事件循环: ActivateAgent/checkpoint/eviction |
| `runtime/runtime.rs` | 1,779 | AgentRuntime: execute/tool_stream/checkpoint |
| `runtime/task_graph.rs` | 1,464 | DAG 调度: spawn/mark/query |
| `tui/state.rs` | 1,408 | AppState: messages/metrics/panels |
| `tui/chat_lines.rs` | 1,645 | Markdown 渲染 + 工具行 + 错误竖线 |
| `tui/popup.rs` | 783 | Agent Detail 弹窗 (reasoning + tool_trace) |
| `llm/chat.rs` | 611 | wrap_tool_stream + 三层循环保护 |
| `checkpoint.rs` | 355 | AgentPool + TaskGraph 持久化 |
| `agent/agent.rs` | 804 | Agent 结构体 + AgentPool |
