# 经验驱动角色自进化系统 —— 实现方案

> **核心理念**: 角色本省也应该是经验的一部分。角色通过执行产生经验，经验反过来优化角色——形成系统自进化的完整闭环。

---

## 当前进度

| 阶段 | 状态 | Commit |
|------|------|--------|
| P0-1 Agent `role_template_id` 字段 | ✅ 已完成 | `b71cccd` |
| P0-2 经验记录传入 `role_template_id` | ✅ 已完成 | `cac7527` |
| P0-3 按角色搜索 (`search_by_role` / `get_experiences_by_role`) | ✅ 已完成 | `b704e02` |
| P0-4 Cluster 跟踪 `role_template_ids` | ✅ 已完成 | `d0fc9c9` |
| P0-5 `/role` TUI 命令 (list/show/create/edit/delete) | ✅ 已完成 | `f4c27e3` |
| P1 角色 embedding 自动计算 | ⏳ 待实现 | -
| P2 Prompt 优化引擎 | ✅ 已完成 | `7e80a40` |
| P3 副作用与反馈 | ✅ 已完成 | `afcf857` |

---

## 总体架构

```
┌─────────────────────────────────────────────────────────────────────┐
│                      ExperiencePool                                │
│  ┌──────────────────────┐  ┌──────────────────────────────────────┐│
│  │  FluidTrack (B-track)│  │  ExperiencePool (A-track, mmap)     ││
│  │  Vec<ExperienceEntry>│  │  ~/.workflow/experience_a.bin       ││
│  │      ↑               │  │      ↑                              ││
│  │  role_template_id     │  │  role_template_id                   ││
│  │  = Some(2)           │  │  = Some(2)                          ││
│  └──────────┬───────────┘  └──────────────┬───────────────────────┘│
│             │                              │                        │
│             └──────────┬───────────────────┘                        │
│                        ▼                                           │
│             DualTrackMemory.search_by_role(role_id)                │
└─────────────────────────────────────────────────────────────────────┘
         ↑                           ↓
         │                    ┌──────────────┐
         │ 写入               │ Prompt 优化  │
         │ 经验               │ 引擎 (LLM)   │
         │                    └──────┬───────┘
         │                           ↓
┌────────┴──────────────┐   ┌────────┴──────────────┐
│  Agent 执行           │   │  RoleTemplateStore    │
│  • system_prompt      │←──│  • system_prompt ↑   │
│  • role_template_id   │   │  • embedding ✓       │
│  • goal → experience  │   │  • role_template_id  │
└───────────────────────┘   └───────────────────────┘
         ↑                           ↑
         │                           │
┌────────┴───────────────┐  ┌────────┴──────────────┐
│  TUI (/role)           │  │  TUI (/role optimize) │
│  • list / show         │  │  • 收集经验 → LLM 分析│
│  • create / edit       │  │  • 生成改进提示词     │
│  • delete              │  │  • 用户确认 → 更新    │
│  • embed               │  │  • 自动计算 embedding │
└────────────────────────┘  └───────────────────────┘
```

---

## 阶段 1：TUI 角色管理（3-4 小时）

### 1.1 `/role` 命令集

**新增文件**: `src/tui/dialogs/role.rs` — 角色 Wizard 对话框

**修改文件**: `src/tui/commands.rs`

```rust
// 新增命令
"/role"                  → 显示子命令帮助
"/role list"             → 列出所有角色模板（表格展示）
"/role show <name>"      → 查看角色详情
"/role create"           → 启动创建 Wizard
"/role edit <name>"      → 启动编辑 Wizard
"/role delete <name>"    → 删除角色（带确认）
"/role embed <name>"     → 重新计算角色 embedding
```

### 1.2 RoleList 渲染

```
╭─ Role Templates ───────────────────────────────╮
│                                                │
│  ID  Name               Label         Embedded │
│   0  general_business…  General Busin… ✓      │
│   1  tester             QA Engineer    ✓      │
│   2  developer          Developer      ✓      │
│   3  reviewer           Code Reviewer  ✓      │
│   4  security_auditor   Security Audi… ✓      │
│                                                │
│  j/k navigate  Enter detail  Esc close        │
╰────────────────────────────────────────────────╯
```

### 1.3 RoleWizard (创建/编辑)

```
╭─ Role Template — Step 1/3 ─────────────────────╮
│                                                │
│  ● Role Name ─ ○ Label ─ ○ Prompt             │
│                                                │
│  Enter a role name:                            │
│  ┌──────────────────────────────────────────┐  │
│  │ developer                               │  │
│  └──────────────────────────────────────────┘  │
│                                                │
│  Enter to continue  ·  Esc to cancel           │
╰────────────────────────────────────────────────╯
```

### 1.4 修改清单

| 文件 | 修改内容 |
|------|---------|
| `src/tui/dialogs/mod.rs` | 新增 `Role(RoleWizard)` 和 `RoleList` 变体 |
| `src/tui/dialogs/role.rs` | **新建** — RoleWizard 和 RoleList 对话框 |
| `src/tui/commands.rs` | 解析 `/role` 及其子命令 |
| `src/tui/render.rs` | 渲染 overlay 对话框 |

---

## 阶段 2：角色与经验连接（2-3 小时）

### 2.1 Agent 携带 role_template_id

**修改文件**: `src/agent/agent.rs`

```rust
pub struct Agent {
    pub role: String,
    pub role_template_id: Option<u32>,  // ← 新增
    // ... 原有字段
}
```

### 2.2 创建 Agent 时设置 role_template_id

**修改文件**: `src/runtime/runtime.rs`

三个 spawn 方法都需要修改：

- `bootstrap_root_agent` — 查找角色模板并传 ID
- `spawn_root_agent` — 同上
- `spawn_child` — 同上

```rust
let role_tpl = self.role_template_store.get_by_role(role)
    .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85));

let agent = Agent {
    role: role.to_string(),
    role_template_id: role_tpl.as_ref().map(|t| t.template_id),  // ← 设置
    config: AgentConfig {
        system_prompt: role_tpl.as_ref()
            .map(|t| t.system_prompt.clone())
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role)),
        ...
    },
    ...
};
```

### 2.3 记录经验时传入 role_template_id

**修改文件**: `src/runtime/runtime.rs`

```rust
// execute_agent_inner — Agent 完成后
let role_tpl_id = agent_pool.read().await
    .get_agent(&agent_id)
    .and_then(|a| a.role_template_id);

self.pipeline.record_experience(ExperienceEntry {
    embedding: emb,
    role_template_id: role_tpl_id,  // ← 不再是 None!
    ...
});
```

**修改文件**: `src/tui/effect.rs`

```rust
// TUI Chat 完成后
let role_tpl_id = state.read().await
    .core
    .responsible_agent_id
    .and_then(|id| agent_pool.read().await.get_agent(&id))
    .and_then(|a| a.role_template_id);

rt.record_experience(ExperienceEntry {
    embedding: emb,
    role_template_id: role_tpl_id,  // ← 不再是 None!
    ...
});
```

### 2.4 聚类合并时聚合角色 ID

**修改文件**: `src/experience/clustering.rs`

```rust
pub struct Cluster {
    // ... 原有字段
    pub role_template_ids: Vec<u32>,  // ← 新增：跟踪该簇包含的角色
}

impl Cluster {
    pub fn new(entry: &ExperienceEntry) -> Self {
        Self {
            // ...
            role_template_ids: entry.role_template_id
                .map(|id| vec![id])
                .unwrap_or_default(),
        }
    }

    pub fn update(&mut self, entry: &ExperienceEntry) {
        // ...
        if let Some(id) = entry.role_template_id {
            if !self.role_template_ids.contains(&id) {
                self.role_template_ids.push(id);
            }
        }
    }

    pub fn to_experience_entry(&self, default_weight: f32) -> ExperienceEntry {
        ExperienceEntry {
            role_template_id: self.most_common_role_id(),  // ← 聚合
            // ...
        }
    }

    fn most_common_role_id(&self) -> Option<u32> {
        if self.role_template_ids.is_empty() {
            None
        } else if self.role_template_ids.len() == 1 {
            Some(self.role_template_ids[0])
        } else {
            // 多角色混合 → 选出现频率最高的
            // (需要 Cluster 额外存储频率，或简单返回 None)
            None
        }
    }
}
```

### 2.5 按角色搜索

**修改文件**: `src/experience/dual_track.rs`

```rust
impl DualTrackMemory {
    /// 按角色 ID 过滤搜索
    pub fn search_by_role(
        &self,
        query: &[f32; EMBEDDING_DIM],
        role_id: u32,
        k: usize,
    ) -> Vec<(ExperienceEntry, f32)> {
        let mut results = Vec::new();

        for (entry, score) in self.bedrock.search(query, k * 2) {
            if entry.role_template_id == Some(role_id) {
                results.push((entry, score * self.bedrock_credibility));
            }
        }

        for (entry, score) in self.fluid.search(query, k * 2) {
            if entry.role_template_id == Some(role_id) {
                results.push((entry, score * self.fluid_credibility));
            }
        }

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        results.truncate(k);
        results
    }

    /// 获取某角色的所有经验（不分排名）
    pub fn get_experiences_by_role(&self, role_id: u32) -> Vec<ExperienceEntry> {
        let mut results = Vec::new();

        for entry in self.bedrock.entries() {
            if entry.role_template_id == Some(role_id) {
                results.push(entry.clone());
            }
        }

        for entry in self.fluid.entries() {
            if entry.role_template_id == Some(role_id) {
                results.push(entry.clone());
            }
        }

        results
    }
}
```

### 2.6 L1 信心检查按角色加权

**修改文件**: `src/experience/dual_track.rs`

```rust
pub fn check_confidence(
    &self,
    task_embedding: &[f32; EMBEDDING_DIM],
    role_embedding: &[f32; EMBEDDING_DIM],
    role_template_id: Option<u32>,  // ← 新增参数
) -> Result<L1Assessment, SpawnRejection> {
    if self.is_empty() {
        return Err(SpawnRejection::L1Rejected { ... });
    }

    let task_matches = self.search(task_embedding, 5);
    let role_matches = if let Some(role_id) = role_template_id {
        // 有角色 ID → 同角色经验加权
        self.search_by_role(role_embedding, role_id, 5)
    } else {
        self.search(role_embedding, 5)
    };

    let task_score = task_matches.first().map(|(_, s)| *s).unwrap_or(0.0);
    let role_score = role_matches.first().map(|(_, s)| *s).unwrap_or(0.0);
    let combined = (task_score + role_score) / 2.0;

    if combined >= self.confidence_threshold {
        let recommended_tools = self.infer_tools(&task_matches);
        Ok(L1Assessment {
            confidence: combined,
            recommended_tools,
            matched_experiences: task_matches.len() + role_matches.len(),
        })
    } else {
        Err(SpawnRejection::L1Rejected { ... })
    }
}
```

---

## 阶段 3：角色 embedding 自动计算（1 小时）

### 3.1 启动时计算

**修改文件**: `src/runtime/runtime.rs`

```rust
// from_pipeline 末尾
let store = self.role_template_store.clone();
let embedding = self.pipeline.embedding().clone();
let mut updated = false;
for t in store.all() {
    if t.embedding.is_none() {
        if let Ok(emb) = embedding.embed(&t.system_prompt).await {
            let mut new_t = t.clone();
            new_t.embedding = Some(emb);
            store.upsert(new_t);
            updated = true;
        }
    }
}
if updated {
    store.persist();
}
```

### 3.2 `/role embed` 命令

```rust
"/role embed" => {
    // 异步计算所有无 embedding 的模板
    state.effects.push(Effect::ComputeRoleEmbeddings);
    true
}
```

```rust
Effect::ComputeRoleEmbeddings {
    // 在 effect.rs 中执行
    let store = runtime.role_template_store.clone();
    let embedding = runtime.pipeline.embedding().clone();
    for t in store.all() {
        if t.embedding.is_none() {
            if let Ok(emb) = embedding.embed(&t.system_prompt).await {
                let mut new_t = t.clone();
                new_t.embedding = Some(emb);
                store.upsert(new_t);
            }
        }
    }
    store.persist();
}
```

### 3.3 创建/编辑角色时自动计算

```rust
// RoleWizard 保存时
Effect::SaveRoleTemplate {
    role: RoleTemplate { ... },
    compute_embedding: true,  // 自动计算
}
```

---

## 阶段 4：Prompt 优化引擎（4-6 小时）

### 4.1 Pipeline 接口

**修改文件**: `src/runtime/pipeline.rs`

```rust
impl DecisionPipeline {
    /// 获取某角色的所有经验
    pub fn get_experiences_by_role(&self, role_id: u32) -> Vec<ExperienceEntry> {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .get_experiences_by_role(role_id)
    }
}
```

### 4.2 ExperienceRetrieval trait 扩展

**修改文件**: `src/l1/mod.rs`

```rust
pub trait ExperienceRetrieval: Send + Sync {
    // ... 原有方法

    /// 按角色 ID 过滤搜索
    fn search_by_role(
        &self,
        query: &[f32; EMBEDDING_DIM],
        role_id: u32,
        k: usize,
    ) -> Vec<(ExperienceEntry, f32)>;

    /// 获取某角色的所有经验
    fn get_experiences_by_role(&self, role_id: u32) -> Vec<ExperienceEntry>;
}
```

### 4.3 优化引擎

**新增文件**: `src/runtime/optimizer.rs`

```rust
//! Prompt优化引擎 — 从经验中学习并改进角色系统提示词。

use crate::core::types::{EMBEDDING_DIM, ExperienceEntry};
use crate::runtime::config::RoleTemplate;
use crate::llm::LlmProvider;

/// 触发优化所需的最少经验数
pub const MIN_EXPERIENCES_FOR_OPTIMIZATION: usize = 10;

/// 优化结果
pub struct OptimizationResult {
    pub role_name: String,
    pub original_prompt: String,
    pub improved_prompt: String,
    pub analysis_summary: String,
    pub stats: OptimizationStats,
}

pub struct OptimizationStats {
    pub total_experiences: usize,
    pub successful_count: usize,
    pub low_quality_count: usize,
    pub most_used_tools: u64,
    pub avg_success_weight: f32,
}

/// 运行 prompt 优化
pub async fn optimize_role(
    role: &RoleTemplate,
    experiences: &[ExperienceEntry],
    llm: &LlmProvider,
    model_id: &str,
) -> Result<OptimizationResult, anyhow::Error> {
    if experiences.len() < MIN_EXPERIENCES_FOR_OPTIMIZATION {
        return Err(anyhow::anyhow!(
            "Need at least {} experiences, got {}",
            MIN_EXPERIENCES_FOR_OPTIMIZATION,
            experiences.len()
        ));
    }

    // 1. 统计分析
    let stats = compute_stats(experiences);

    // 2. 提取成功和失败案例
    let (successful, low_quality): (Vec<_>, Vec<_>) = experiences
        .iter()
        .partition(|e| e.weight >= 0.7);

    // 3. 构建分析 prompt
    let analysis_prompt = build_analysis_prompt(
        &role.role,
        &role.system_prompt,
        &stats,
        &successful,
        &low_quality,
    );

    // 4. 调用 LLM 生成改进提示词
    let improved_prompt = llm
        .chat(model_id, "", &analysis_prompt)
        .await
        .map_err(|e| anyhow::anyhow!("LLM optimization failed: {}", e))?;

    // 5. 提取纯文本（去掉可能的 markdown 包装）
    let cleaned = clean_prompt_output(&improved_prompt);

    let summary = format!(
        "Analyzed {} experiences ({} successful, {} low-quality). \
         Most used tools: {:016b}. Average success weight: {:.2}.",
        stats.total_experiences,
        stats.successful_count,
        stats.low_quality_count,
        stats.most_used_tools,
        stats.avg_success_weight,
    );

    Ok(OptimizationResult {
        role_name: role.role.clone(),
        original_prompt: role.system_prompt.clone(),
        improved_prompt: cleaned,
        analysis_summary: summary,
        stats,
    })
}

/// 统计分析
fn compute_stats(experiences: &[ExperienceEntry]) -> OptimizationStats {
    let total = experiences.len();
    let (high, low): (Vec<_>, Vec<_>) = experiences.iter().partition(|e| e.weight >= 0.7);
    let mut tools = 0u64;
    for e in experiences {
        tools |= e.tool_bitmap;
    }
    let avg_w = experiences.iter().map(|e| e.weight).sum::<f32>() / total as f32;

    OptimizationStats {
        total_experiences: total,
        successful_count: high.len(),
        low_quality_count: low.len(),
        most_used_tools: tools,
        avg_success_weight: avg_w,
    }
}

/// 构建 LLM 分析 prompt
fn build_analysis_prompt(
    role_name: &str,
    current_prompt: &str,
    stats: &OptimizationStats,
    successful: &[&ExperienceEntry],
    low_quality: &[&ExperienceEntry],
) -> String {
    // 工具位图转文字描述
    let tool_names = describe_tools(stats.most_used_tools);

    // 成功案例的任务描述
    let success_tasks: Vec<String> = successful
        .iter()
        .take(10)  // 最多取 10 条
        .map(|e| format!("- (weight={:.2}, tools={:016b})", e.weight, e.tool_bitmap))
        .collect();

    let fail_tasks: Vec<String> = low_quality
        .iter()
        .take(5)
        .map(|e| format!("- (weight={:.2}, tools={:016b})", e.weight, e.tool_bitmap))
        .collect();

    format!(
        r#"You are optimizing an AI agent role system prompt.

## Current Role: "{role_name}"
## Current System Prompt:
```
{current_prompt}
```

## Performance Statistics
- Total experiences: {total}
- Successful (weight >= 0.7): {success_count}
- Low-quality (weight < 0.7): {fail_count}
- Most used tools across all experiences: {tools}
- Average weight: {avg_weight:.2}

## Successful experience patterns:
{success_tasks}

## Low-quality experience patterns:
{fail_tasks}

## Optimization Instructions
Analyze the patterns above and generate an IMPROVED system prompt for the "{role_name}" role.

**Rules:**
1. Keep what works from the current prompt — don't discard valuable guidance
2. Add specific, actionable guidance based on SUCCESSFUL patterns (what works)
3. Add explicit anti-patterns and warnings based on LOW-QUALITY patterns (what doesn't work)
4. Be concrete — prefer "Always use thiserror for error types" over "Handle errors properly"
5. Include tool usage guidance where patterns are clear
6. Output ONLY the new system prompt text, no explanations, no markdown

Improved system prompt for role "{role_name}":
"#,
        role_name = role_name,
        current_prompt = current_prompt,
        total = stats.total_experiences,
        success_count = stats.successful_count,
        fail_count = stats.low_quality_count,
        tools = format!("{:016b}", stats.most_used_tools),
        avg_weight = stats.avg_success_weight,
        success_tasks = success_tasks.join("\n"),
        fail_tasks = fail_tasks.join("\n"),
    )
}

/// 清理 LLM 输出（去除可能的 markdown 包裹）
fn clean_prompt_output(output: &str) -> String {
    let output = output.trim();
    // 去除 ``` 代码块
    if output.starts_with("```") {
        let without_fence = output.trim_start_matches("```")
            .trim_start_matches("rust")
            .trim_start_matches("text")
            .trim_start_matches("markdown")
            .trim();
        if let Some(end) = without_fence.rfind("```") {
            without_fence[..end].trim().to_string()
        } else {
            without_fence.to_string()
        }
    } else {
        output.to_string()
    }
}

/// 工具位图转描述（占位，后续可扩展）
fn describe_tools(_bitmap: u64) -> String {
    // 可维护一个工具 ID → 名称的映射表
    "various tools".to_string()
}
```

### 4.4 TUI 触发优化

**修改文件**: `src/tui/commands.rs`

```rust
"/role optimize <name>" => {
    let role_name = parts.get(2).unwrap_or("");
    if role_name.is_empty() {
        core.messages.push(ChatMessage::system(
            "Usage: /role optimize <role_name>"
        ));
    } else {
        state.effects.push(Effect::OptimizeRole {
            role_name: role_name.to_string(),
        });
    }
    true
}
```

**修改文件**: `src/tui/effect.rs`

```rust
Effect::OptimizeRole { role_name } => {
    let rt = runtime.read().await;
    let role = rt.get_role_template(&role_name)
        .ok_or("Role not found")?;

    let experiences = rt.get_experiences_by_role(role.template_id);
    // ... 调用 optimizer::optimize_role
    // ... 发送结果到 event channel

    let provider = rt.provider.clone()
        .ok_or("No LLM provider")?;

    match optimize_role(&role, &experiences, &provider, &rt.model_id).await {
        Ok(result) => {
            // 显示 diff 给用户确认
            tx.send(AppEvent::OptimizationResult {
                role_name: role_name.clone(),
                original: result.original_prompt,
                improved: result.improved_prompt,
                summary: result.analysis_summary,
                stats: result.stats,
            });
        }
        Err(e) => {
            tx.send(AppEvent::OptimizationError {
                role_name: role_name.clone(),
                error: e.to_string(),
            });
        }
    }
}
```

### 4.5 用户确认界面

```
╭─ Role Optimization: developer ──────────────────────────╮
│                                                          │
│  Analyzed 47 experiences (35 successful, 12 low-quality)│
│  Most used tools: 0000000000000111 (read, write, shell) │
│                                                          │
│  ┌─ Changes ───────────────────────────────────────────┐│
│  │  @@ -1,5 +1,8 @@                                    ││
│  │   You are a developer.                               ││
│  │  +                                                 ││
│  │  +## Key Patterns from Experience                    ││
│  │  +                                                 ││
│  │  +### Do                                           ││
│  │  +- Always write tests before implementation       ││
│  │  +- Use thiserror for error types                   ││
│  │  +- Read existing files before writing              ││
│  │  +                                                 ││
│  │  +### Avoid                                        ││
│  │  +- Using unwrap() in library code                  ││
│  │  +- Skipping tests for "simple" changes            ││
│  └────────────────────────────────────────────────────┘│
│                                                          │
│  [y] Apply  [d] Show diff  [Esc] Cancel                  │
╰──────────────────────────────────────────────────────────╯
```

---

## 阶段 5：副作用与经验质量反馈（2-3 小时）

### 5.1 L2 覆盖写入经验

L2 审计引擎可以标记经验是否需要覆盖：

```rust
// l2/llm.rs 中
pub struct L2LlmConfig {
    pub override_weight: f32,      // L2 给出的覆盖权重
    pub override_reason: String,   // 覆盖原因
}

// L2 审计通过后，反馈到经验
let override_entry = ExperienceEntry {
    embedding: request.task_description_embedding,
    role_template_id: ...,
    weight: l2_result.override_weight,  // L2 动态权重
    l2_override_weight: l2_result.override_weight,
    l2_override_created_at: now(),
    ...
};
```

### 5.2 工具使用记录

当前 `tool_bitmap` 始终为 0：

```rust
// execute_agent_inner 中
// 记录 Agent 实际使用的工具
let used_tools = extract_used_tools(&agent_result);
let role_tpl_id = agent_pool.read().await
    .get_agent(&agent_id)
    .and_then(|a| a.role_template_id);

self.pipeline.record_experience(ExperienceEntry {
    embedding: emb,
    role_template_id: role_tpl_id,
    tool_bitmap: used_tools,  // ← 不再是 0!
    weight: 0.8,
    ...
});
```

### 5.3 优化频率控制

```rust
// 防止频繁优化同一角色
pub struct OptimizationTracker {
    last_optimized: HashMap<u32, Instant>,  // role_id → 上次优化时间
    min_interval: Duration,                  // 最小间隔 24h
    min_new_experiences: usize,              // 最少新增经验 20
}

impl OptimizationTracker {
    pub fn can_optimize(&self, role_id: u32, current_count: usize) -> bool {
        let last = self.last_optimized.get(&role_id);
        let enough_time = last.map_or(true, |t| t.elapsed() >= self.min_interval);
        let enough_new = last.map_or(true, |_| {
            // 需要记录上次优化时的经验计数
            true  // 简化逻辑
        });
        enough_time && enough_new
    }
}
```

---

## 完整修改清单

### 新增文件

| 文件 | 说明 | 预估行数 |
|------|------|---------|
| `src/tui/dialogs/role.rs` | RoleWizard + RoleList 对话框 | ~300 |
| `src/runtime/optimizer.rs` | Prompt 优化引擎 | ~250 |

### 修改文件

| 文件 | 修改内容 | 预估行数 |
|------|---------|---------|
| `src/tui/dialogs/mod.rs` | 新增 `ActiveDialog::Role(RoleWizard)` 和 `ActiveDialog::RoleList` | +20 |
| `src/tui/commands.rs` | 解析 `/role` 及其子命令 | +80 |
| `src/tui/effect.rs` | `Effect::ComputeRoleEmbeddings` + `Effect::OptimizeRole` | +120 |
| `src/tui/state.rs` | 新增消息变体 `MessageRole::Optimization` | +10 |
| `src/tui/render.rs` | 渲染优化结果 diff（可选） | +50 |
| `src/agent/agent.rs` | Agent 新增 `role_template_id` | +5 |
| `src/runtime/runtime.rs` | spawn 方法传 ID + 经验记录传 ID + 启动时计算 embedding | +60 |
| `src/runtime/pipeline.rs` | 新增 `get_experiences_by_role()` 接口 | +10 |
| `src/experience/dual_track.rs` | 新增 `search_by_role()` + `get_experiences_by_role()` | +50 |
| `src/experience/clustering.rs` | Cluster 聚合 `role_template_ids` | +25 |
| `src/l1/mod.rs` | `ExperienceRetrieval` trait 扩展 | +10 |

### 总计

| 阶段 | 新增文件 | 修改文件 | 预估时间 |
|------|---------|---------|---------|
| 1. TUI 角色管理 | 1 | 3 | 3-4h |
| 2. 角色与经验连接 | 0 | 5 | 2-3h |
| 3. 角色 embedding 计算 | 0 | 2 | 1h |
| 4. Prompt 优化引擎 | 1 | 3 | 4-6h |
| 5. 副作用与反馈 | 0 | 3 | 2-3h |
| **合计** | **2** | **~10** | **12-17h** |

---

## 依赖关系

```
阶段 1 (TUI 角色管理)
    │
    ▼
阶段 2 (角色与经验连接) ← 必须先完成 1，因为需要 TUI 创建角色
    │
    ├──→ 阶段 3 (embedding 计算) ← 可以并行
    │
    ▼
阶段 4 (Prompt 优化引擎) ← 必须完成 2 和 3
    │
    ▼
阶段 5 (副作用与反馈) ← 增强优化质量，可滞后
```

---

## 优先实现路径

如果时间有限，按此优先级：

```
P0 ─ 阶段 1 + 阶段 2
     ├─ /role list / show / create / edit / delete
     ├─ Agent.role_template_id
     ├─ ExperienceEntry.role_template_id 写入
     └─ 按角色搜索

P1 ─ 阶段 3
     ├─ 角色 embedding 自动计算
     └─ /role embed 命令

P2 ─ 阶段 4
     ├─ optimizer.rs Prompt 优化引擎
     └─ /role optimize 命令

P3 ─ 阶段 5
     ├─ 工具使用记录
     ├─ L2 反馈写入
     └─ 优化频率控制
```
