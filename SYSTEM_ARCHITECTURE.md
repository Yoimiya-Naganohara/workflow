# 整体系统架构分析

根据 GVSD 协议，建立单一全局模型后逐层分析。

---

## 1. 全局模型 — 系统全景

```
                            main.rs
                      ┌──────────────────┐
                      │  tokio::main      │
                      │  CLI / TUI 二选一 │
                      └──────┬───────────┘
                             │
              ┌──────────────┴──────────────┐
              │                              │
              ▼                              ▼
       ┌──────────────┐             ┌──────────────────┐
       │  CLI mode    │             │    TUI mode       │
       │  process_    │             │  ┌──────────────┐ │
       │  with_text() │             │  │ ratatui 界面  │ │
       │  → 单次决策   │             │  │ event loop    │ │
       └──────┬───────┘             │  │ runtime_bridge│ │
              │                     │  └──────┬───────┘ │
              │                     └─────────┼─────────┘
              │                               │
              └───────────────┬───────────────┘
                              │
                              ▼
                    ┌─────────────────────┐
                    │   AgentRuntime       │
                    │   (src/runtime/)     │
                    └──────────┬──────────┘
                               │
              ┌────────────────┼─────────────────┐
              │                │                  │
              ▼                ▼                  ▼
     ┌────────────────┐ ┌──────────────┐ ┌──────────────────┐
     │ DecisionPipeline│ │ AgentPool    │ │ RuntimeEventLoop │
     │ (L-1/L0/L1/L2) │ │ (agent pool) │ │ (异步事件循环)   │
     └────────────────┘ └──────────────┘ └──────────────────┘
```

---

## 2. 核心架构层次

### 2.1 决策管道 (Decision Pipeline)

```
SpawnRequest
    │
    ▼
┌─────────────────────────────────────────────────┐
│  L-1: AdmissionController (src/admission.rs)     │
│  基于 tokio semaphore 的并发准入控制              │
│  限制同时运行的 agent 数量                       │
└──────────────────────┬──────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────┐
│  L0: Circuit Breaker + Budget (src/l0.rs)        │
│  CAS 原子预算分配 (AtomicU64)                     │
│  并发安全，无锁竞争                              │
│  BudgetGuard RAII 释放                           │
└──────────────────────┬──────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────┐
│  L1: Experience Retrieval (src/l1/)              │
│  SIMD 余弦相似度 + 工具位图过滤                   │
│  Fluid Vec 热数据 + mmap 冷数据                   │
│  返回 top-K 相似经验                              │
└──────────────────────┬──────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────┐
│  L2: Rule Audit + LLM Judge (src/l2/)            │
│  规则引擎 + LLM 评估                             │
│  拒绝率 ≤ 15% (CI 强制验证)                      │
│  输出 SpawnDecision::Approved | Rejected         │
└──────────────────────────────────────────────────┘
```

### 2.2 工具系统

```
ToolServerHandle
  ├── built-in (12 个)
  │   ├── read_file / write_file / sh (核心 I/O)
  │   ├── grep / find_files (搜索)
  │   ├── move_file / copy_file / delete_file (文件操作)
  │   ├── diff_edit / fetch (编辑/网络)
  │   ├── extract_json (数据处理)
  │   └── search_asset (语义检索，仅沙箱)
  ├── agent (4 个)
  │   ├── spawn_agent / list_agents / send_message / read_messages
  └── memo (4 个)
      ├── write_memo / read_memo / list_memos / delete_memo
```

### 2.3 运行时期

```
RuntimeEventLoop (src/runtime/runtime_loop.rs)
  事件驱动状态机:
  
  SpawnTask → ActivateAgent → execute_agent_detached
      ↓                                      ↓
  TaskGraph 更新                      LLM 调用(带工具)
      ↓                                      ↓
  ChildCompleted ←─────────────── process_tool_stream
      ↓
  all_done? → ReadyForAggregation → 父 agent 聚合
      ↓
  AggregationCompleted → 结果上报
```

### 2.4 持久化

```
~/.workflow/
  ├── state.json          → API keys, provider 配置
  ├── experience_a.bin    → mmap 经验池（384-d 嵌入向量）
  ├── sandbox/{id}/       → 每个 agent 的工作目录
  │   ├── work/            → 可写隔离区
  │   └── src → 项目源树   → 只读 symlink
  └── role_memos/         → 按 role 持久化的 memo
```

---

## 3. 复杂度分析

### 3.1 代码规模

```
模块              行数     文件数   职责
──────────────────────────────────────────
src/runtime/      ~10,000  20      决策管道 + agent 生命周期
src/tui/          ~8,000   17      终端 UI
src/tools/        ~6,800   7       工具系统
src/llm/          ~1,800   3       LLM 提供者抽象
src/experience/   ~1,800   5       经验池
src/l0/           ~630     1       CAS 预算
src/l1/           ~1       1       (单行模块)
src/l2/           ~1       1       (单行模块)
src/core/         ~1       1       类型 + SIMD
src/agent/        ~1       1       agent 定义 + 计划
──────────────────────────────────────────
总计              ~30,000  60+
```

### 3.2 架构复杂度热点

#### 🔴 P0: 20 个子模块的 runtime

`src/runtime/` 有 **20 个文件**，职责分散：

```
runtime.rs              AgentRuntime 主结构
pipeline.rs             决策管道构建
runtime_loop.rs         事件循环状态机
task_graph.rs           DAG 任务图
strategy_graph.rs       策略竞争图
graph_analytics.rs      图分析 + 模板演化
scheduler.rs            任务调度器
dispatch.rs             分发决策
decomposition.rs        任务分解
capability.rs           能力注册 + 角色选择
embedding_analyzer.rs   嵌入分析器
escalation.rs           升级策略
validation.rs           验证引擎
optimizer.rs            优化器
agent_exec.rs           agent 执行
agent_stream.rs         工具流处理
agent_lifecycle.rs      agent 生命周期 + 工具位图
event.rs                事件类型
config.rs               配置
```

**问题：** 每个子模块都很薄（有的 < 100 行），但数量多导致理解成本高。

#### 🟡 P1: 分层过度

```
SpawnRequest
  → L-1 (admission)        ← 4 个 impl 文件
  → L0 (budget + breaker)  ← 1 个文件
  → L1 (experience)        ← 5 个 experience 文件
  → L2 (audit + judge)     ← 1 个文件
```

实际运行中：
- L-1 Admission: 纯 tokio semaphore 封装，4 个函数
- L0 Budget: 纯 CAS 原子操作，3 个函数  
- L1 Experience: SIMD 搜索 + 双轨存储，但检索接口只用 `search_experience()`
- L2 Audit: 规则引擎 + LLM judge，但 LLM judge 可能永远不被配置

**问题：** 架构重量超过实际逻辑复杂度。L-1 和 L0 可以合并。

#### 🟡 P2: 沙箱实现偏重

`sandbox.rs` (556 行) 承担了：
- 路径解析 (resolve_path / resolve_write_path)
- 语义索引 (AssetIndex / create_embedded_asset)
- 嵌入引擎注入
- cleanup 生命周期

其中语义索引部分（`create_embedded_asset`）依赖嵌入引擎，但这个能力只在沙箱存在时才可用。这部分逻辑复杂且使用率低。

#### 🟢 P3: 工具位图维护成本

`tool_bit()` 函数手工分配 26 个 bit 位，需要与工具注册保持同步。已有历史遗漏（line_edit, fetch 等曾丢失）。

---

## 4. 改进建议

### 建议 1: 合并 L-1 和 L0

```
现在:                    建议:
L-1 Admission            GuardLayer {
L0 Budget                  semaphore: tokio::sync::Semaphore
                           budget: AtomicI64
L1 Experience            }
L2 Audit                 
                         ExperienceLayer { retrieval, clustering }
                         AuditLayer { rules, llm }
```

**收益：** 从 4 层减到 3 层，减少文件数量。L-1 Admission 和 L0 Budget 共享同一个 `BudgetGuard` RAII —— 它们本就是同一个并发控制语义的两个面。

### 建议 2: runtime 模块重组

```
现在:                   建议:
runtime/               runtime/
  20 文件                pipeline.rs (合并决策层)
                        runtime.rs (AgentRuntime)
                        runtime_loop.rs (事件循环, 精简)
                        task_graph.rs (DAG)
                        lifecycle.rs (agent 生命周期)
                        agent_exec.rs (执行 + 流)
                          → 从 20 → 6 文件
```

### 建议 3: 评估是否需要多策略支持

`strategy_graph.rs` (938 行) + `graph_analytics.rs` (637 行) 实现了竞争协议和模板演化，但在当前架构中：

- 只有一个 `CompetitionProtocol::default()`
- 模板演化仅在调度时触发
- 没有切换策略的运行时路径

如果这个功能暂时未使用，可以考虑简化或推迟实现。

### 建议 4: 评估 TUI 模块

`src/tui/` 有 **17 个文件 (~8000 行)**，包括：
- 自定义 tokenizer
- 命令树解析器
- 聊天渲染系统

如果主要使用 CLI 模式或 API 调用，TUI 这 8000 行是纯维护负担。可以考虑将 `--cli` 模式作为默认，TUI 作为可选功能（feature flag）。

---

## 5. 总结

| 层次 | 当前复杂度 | 简化方向 | 收益 |
|------|-----------|---------|------|
| 决策管道 (L-1/L0) | 2 个模块 3 个文件 | 合并为 GuardLayer | -2 文件 |
| runtime 模块 | 20 个文件 | 合并为 6-8 个核心文件 | -12 文件 |
| 策略图 + 图分析 | 2 个文件 ~1600 行 | 评估是否当前需求 | 可能移除大量代码 |
| TUI | 17 文件 ~8000 行 | 移到 feature flag | 非 UI 开发者不受影响 |
| **总计** | **60+ 文件 ~30,000 行** | **可削减 ~30% 文件数** | |
