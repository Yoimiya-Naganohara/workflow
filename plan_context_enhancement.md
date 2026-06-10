# 🧠 基于 Embedding 的上下文管理增强方案

> 项目: workflow — Holographic self-evolving multi-agent system
> 
> 日期: 2025-01
> 
> 状态: 规划中

---

## 目录

1. [现状分析](#1-现状分析)
2. [增强方案总览](#2-增强方案总览)
3. [Phase 1: 多维度上下文表示](#3-phase-1-多维度上下文表示)
4. [Phase 2: 动态上下文窗口](#4-phase-2-动态上下文窗口)
5. [Phase 3: 上下文摘要与压缩](#5-phase-3-上下文摘要与压缩)
6. [Phase 4: 层级上下文路由](#6-phase-4-层级上下文路由)
7. [Phase 5: 上下文预测与预热](#7-phase-5-上下文预测与预热)
8. [Phase 6: 监控与可观测性](#8-phase-6-监控与可观测性)
9. [Miss 风险分析](#9-miss-风险分析)
10. [实施路线图](#10-实施路线图)
11. [快速见效 MVP](#11-快速见效-mvp)

---

## 1. 现状分析

### 当前 Embedding 使用情况

系统已使用 **all-MiniLM-L6-v2 (384维)** embedding 在以下位置：

| 位置 | 用途 |
|------|------|
| **L1 Experience Retrieval** | 用 task/role/value embedding 做 cosine similarity 检索，判断置信度 |
| **Dual-track 经验池** | Bedrock + Fluid 双轨存储，用 embedding 检索相似经验 |
| **Clustering** | Welford 在线聚类，按 centroid 做上下文压缩 |
| **Role Template Store** | embedding 相似度匹配最佳角色模板 |
| **TUI Controller** | `/pool query` 语义搜索 |
| **聊天记录** | 输入文本自动 embed 并存入经验池 |

### 当前 SpawnRequest 上下文维度

```rust
pub struct SpawnRequest {
    pub task_description_embedding: [f32; EMBEDDING_DIM],   // 任务上下文
    pub role_description_embedding: [f32; EMBEDDING_DIM],   // 角色上下文
    pub value_statement_embedding: [f32; EMBEDDING_DIM],    // 价值观上下文
    // ...
}
```

### 当前 L1 置信度评估

单一检索 + 单一阈值判定，维度较少，可能导致：
- **欠匹配**：仅靠 3 个维度不足以充分表达上下文
- **误判**：单一维度高相似度但整体语义偏离
- **信息丢失**：没有利用对话历史、领域知识等丰富上下文

---

## 2. 增强方案总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                    上下文管理增强路线图                              │
├─────────────────────────────────────────────────────────────────────┤
│  Phase 1 ─ 多维度上下文表示 (Multi-dimensional Context)              │
│  Phase 2 ─ 动态上下文窗口 (Dynamic Context Window)                   │
│  Phase 3 ─ 上下文摘要与压缩 (Context Summarization & Compaction)     │
│  Phase 4 ─ 层级上下文路由 (Hierarchical Context Routing)             │
│  Phase 5 ─ 上下文预测与预热 (Context Prediction & Pre-warming)       │
│  Phase 6 ─ 监控与可观测性 (Context Observability)                    │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Phase 1: 多维度上下文表示

### 3.1 扩展 ContextEmbeddings

```rust
// 新增上下文维度
pub struct ContextEmbeddings {
    /// 任务语义嵌入 (已有)
    pub task: [f32; EMBEDDING_DIM],
    /// 角色语义嵌入 (已有)  
    pub role: [f32; EMBEDDING_DIM],
    /// 价值观对齐嵌入 (已有)
    pub value: [f32; EMBEDDING_DIM],
    /// 🔹 对话历史语义嵌入 (新)
    pub conversation: Option<[f32; EMBEDDING_DIM]>,
    /// 🔹 知识域嵌入 — 领域分类 (新)
    pub domain: Option<[f32; EMBEDDING_DIM]>,
    /// 🔹 意图嵌入 — 用户意图分类 (新)
    pub intent: Option<[f32; EMBEDDING_DIM]>,
    /// 🔹 情绪/语气嵌入 — 语气感知 (新)
    pub tone: Option<[f32; EMBEDDING_DIM]>,
    /// 🔹 时间上下文编码 (新)
    pub temporal: Option<[f32; EMBEDDING_DIM]>,
}
```

### 3.2 多维度置信度评估 (OR + 加权融合)

```rust
pub fn fused_confidence(&self, scores: &[(f32, f32)]) -> f32 {
    // scores: [(similarity, weight), ...]
    
    // 1. 计算加权平均
    let weighted: f32 = scores.iter()
        .map(|(sim, w)| sim * w)
        .sum::<f32>() / scores.iter().map(|(_, w)| w).sum::<f32>();
    
    // 2. 充分性条件：只要有一个维度非常高，就通过
    let max_sim = scores.iter().map(|(s, _)| s).cloned().fold(0., f32::max);
    let has_expert = max_sim > 0.92;
    
    // 3. 多样性 bonus：维度越多，置信度越高
    let non_zero = scores.iter().filter(|(s, _)| *s > 0.3).count();
    let diversity_bonus = (non_zero as f32) * 0.05;
    
    if has_expert {
        (weighted + diversity_bonus).min(1.0)
    } else {
        weighted + diversity_bonus
    }
}
```

**核心原则**：多维度是**增强**而不是**替代**。新增维度只提升召回，不增加硬性门槛。

### 3.3 修改 L1Retriever::check_confidence

```rust
pub fn check_confidence(
    &self,
    query: &ContextEmbeddings,
) -> Result<L1Assessment, SpawnRejection> {
    let mut scores: Vec<(&'static str, f32, f32)> = vec![];
    
    // 每个维度独立检索 + 置信度
    if let Some(task) = &query.task { 
        scores.push(("task", self.score_dimension(task, L1_TASK_WEIGHT)));
    }
    if let Some(role) = &query.role {
        scores.push(("role", self.score_dimension(role, L1_ROLE_WEIGHT)));
    }
    // ... 其他维度
    
    // 动态权重融合
    let combined = fuse_scores(&scores);
    // ...
}
```

---

## 4. Phase 2: 动态上下文窗口

### 4.1 滑动窗口选择器

```rust
pub struct ContextWindow {
    /// 窗口内经验条目
    pub entries: Vec<ExperienceEntry>,
    /// 窗口时间范围 [start, end]
    pub time_range: Option<(u64, u64)>,
    /// 窗口语义半径 (cosine distance threshold)
    pub semantic_radius: f32,
    /// 窗口质量评分 (entropy / density)
    pub quality_score: f32,
}

pub struct DynamicWindowSelector {
    /// 最大窗口大小 (token 预算)
    max_tokens: usize,
    /// 语义半径衰减因子
    radius_decay: f32,
    /// 时间衰减半衰期 (秒)
    time_halflife: u64,
}

impl DynamicWindowSelector {
    /// 动态选择最佳上下文窗口
    pub fn select_window(
        &self,
        query: &[f32; EMBEDDING_DIM],
        pool: &DualTrackMemory,
    ) -> ContextWindow {
        // 1. 初始检索: 宽松阈值 + 大量候选
        let candidates = pool.search(query, self.initial_k());
        
        // 2. 时间衰减排序: 近期经验优先
        let time_decayed = self.apply_time_decay(candidates);
        
        // 3. 语义密集度优化: 选择最密集的区域
        let window = self.find_dense_region(&time_decayed);
        
        // 4. Token 预算裁剪
        self.trim_to_budget(window)
    }
}
```

### 4.2 安全兜底设计

```rust
pub struct SafeDynamicWindow {
    /// 主窗口 (严格语义匹配)
    primary: DynamicWindow,
    /// 兜底窗口 (宽松匹配，低分但全覆盖)
    fallback: DynamicWindow,  
    /// 保留的"常青"经验 (高价值、高频率使用的经验)
    evergreen_ids: HashSet<Uuid>,
}

impl SafeDynamicWindow {
    pub fn select(&self, query: &[f32; EMBEDDING_DIM]) -> ContextWindow {
        // 1. 先用严格窗口检索
        let primary = self.primary.select(query);
        
        // 2. 如果主窗口结果太少 (< min_results)，自动降级
        if primary.entries.len() < MIN_RESULTS {
            tracing::warn!("Primary window too small, falling back to broader search");
            let fallback = self.fallback.select(query);
            return self.merge(primary, fallback);
        }
        
        // 3. 总是注入常青经验
        let evergreen = self.load_evergreen();
        primary.entries.extend(evergreen);
        
        primary
    }
}
```

### 4.3 渐进式放宽策略

```
检索结果数量
    │
    ≥ K  ──→ 正常使用
    │
    < K  ──→ 逐步放宽阈值 (0.8 → 0.6 → 0.4 → 0.2)
    │
    = 0  ──→ 使用"紧急兜底"：返回最近 N 条经验 + 系统默认上下文
    │
    = 0  ──→ 使用类型化的默认模板 (角色默认知识)
```

### 4.4 多窗口融合 (Multi-Window Ensemble)

```rust
pub fn ensemble_context(
    query: &[f32; EMBEDDING_DIM],
    pool: &DualTrackMemory,
    k: usize,
) -> Vec<ContextWindow> {
    // 用不同的语义半径检索多个窗口
    let radii = [0.3, 0.5, 0.7, 0.9];
    let windows: Vec<ContextWindow> = radii.iter()
        .map(|r| select_window_with_radius(query, pool, *r))
        .collect();
    
    // 计算窗口多样性 (Jaccard dissimilarity)
    let diverse = select_diverse_subset(&windows, k);
    
    diverse
}
```

---

## 5. Phase 3: 上下文摘要与压缩

### 5.1 语义分块 (Semantic Chunking)

```rust
pub struct SemanticChunker {
    /// 块大小 (字符数)
    chunk_size: usize,
    /// 块重叠 (字符数) 
    overlap: usize,
    /// 语义断点检测阈值
    break_threshold: f32,
}

impl SemanticChunker {
    /// 将长文本切分为语义连贯的块
    pub fn chunk(&self, text: &str) -> Vec<SemanticChunk> {
        // 1. 滑动窗口 + embedding 相似度检测断点
        // 2. 在语义变化点 (cosine drop > threshold) 切分
        // 3. 每个块独立 embedding
        // 4. 使用 15-20% 的重叠避免跨块信息丢失
    }
    
    /// 检索时返回匹配块及其相邻块
    pub fn retrieve_with_neighbors(
        &self,
        query: &[f32; EMBEDDING_DIM],
        all_chunks: &[SemanticChunk],
        k: usize,
        neighbor_count: usize,
    ) -> Vec<SemanticChunk> {
        let top_k = self.retrieve(query, all_chunks, k);
        let mut expanded = HashSet::new();
        
        for chunk in &top_k {
            let idx = chunk.index;
            for i in idx.saturating_sub(neighbor_count)..=(idx + neighbor_count).min(all_chunks.len()) {
                expanded.insert(i);
            }
        }
        
        expanded.into_iter()
            .map(|i| all_chunks[i].clone())
            .collect()
    }
}
```

### 5.2 层次化上下文摘要

```
原始对话/经验
    │
    ▼
L1 细粒度块 ─── 每个块 embedding + 简短摘要
    │
    ▼
L2 中层簇 ─── Welford centroid + 汇总摘要
    │
    ▼
L3 顶层域 ─── 域 centroid + 域描述
```

```rust
pub struct HierarchicalSummarizer {
    embedder: Arc<EmbeddingService>,
    llm: Arc<LlmProvider>,
    clusterer: ClusterConsolidator,
}

impl HierarchicalSummarizer {
    /// 在查询时动态构建层次化上下文
    pub async fn build_context_pyramid(
        &self,
        query: &ContextEmbeddings,
        pool: &DualTrackMemory,
        depth: usize,
    ) -> ContextPyramid {
        // 1. 在每一层检索
        // 2. 用 LLM 生成该层的自然语言摘要
        // 3. 从粗到细返回
    }
}
```

### 5.3 Token 预算感知的上下文裁剪

```rust
pub struct TokenBudgetAllocator {
    /// 总 token 预算 (由 L0 Circuit Breaker 分配)
    total_budget: usize,
    /// 各维度分配比例
    dimension_ratios: HashMap<String, f32>,
}

impl TokenBudgetAllocator {
    /// 在预算约束下最优分配上下文 token
    pub fn allocate(
        &self,
        candidates: HashMap<String, Vec<ScoredChunk>>,
    ) -> AllocatedContext {
        // 1. 按维度重要性排序
        // 2. 对每个维度: 优先高得分块
        // 3. 动态调整: 若某维度信息密度高，增加分配
        // 4. 当预算耗尽时停止
    }
}
```

---

## 6. Phase 4: 层级上下文路由

### 6.1 上下文类型分类器

```rust
#[derive(Debug)]
pub enum ContextType {
    /// 代码/技术问题 → 检索代码经验
    Technical,
    /// 对话/闲聊 → 检索最近对话
    Conversational,
    /// 决策/仲裁 → 检索规则和 precedents
    Governance,
    /// 新领域/探索 → 检索相关领域 + 宽松阈值
    Exploratory,
    /// 已知任务/重复 → 精确匹配 + 高置信度要求
    Routine,
}

pub struct ContextClassifier {
    embedder: Arc<EmbeddingService>,
    /// 每个类型的 prototype embedding
    prototypes: HashMap<ContextType, [f32; EMBEDDING_DIM]>,
}

impl ContextClassifier {
    pub fn classify(&self, query: &ContextEmbeddings) -> ContextType {
        // 用 embedding 相似度匹配最近的 prototype
    }
}
```

### 6.2 路由策略 + 安全兜底

```rust
pub struct RobustContextRouter {
    classifier: ContextClassifier,
    strategies: HashMap<ContextType, Box<dyn ContextStrategy>>,
}

impl RobustContextRouter {
    pub async fn retrieve_context(
        &self,
        request: &SpawnRequest,
        pool: &DualTrackMemory,
    ) -> RetrievedContext {
        // 1. 软分类：返回概率分布，而不是单个类型
        let distribution = self.classifier.classify_soft(request);
        
        // 2. 用主要策略检索
        let primary_type = distribution.most_likely();
        let primary_ctx = self.strategies[&primary_type]
            .retrieve(request, pool).await;
        
        // 3. 如果主要结果不足，用第二大概率类型兜底
        if primary_ctx.is_sparse() {
            let secondary_type = distribution.second_most_likely();
            let secondary_ctx = self.strategies[&secondary_type]
                .retrieve(request, pool).await;
            return self.fuse(primary_ctx, secondary_ctx);
        }
        
        // 4. 混合策略：主策略 + 通用策略
        let general_ctx = self.strategies[&ContextType::General]
            .retrieve(request, pool).await;
        
        self.fuse(primary_ctx, general_ctx)
    }
}
```

### 6.3 更安全的做法：并行多策略融合

```rust
pub async fn parallel_retrieve(
    &self,
    request: &SpawnRequest,
    pool: &DualTrackMemory,
) -> RetrievedContext {
    let handles: Vec<_> = self.strategies.iter()
        .map(|(_, strategy)| {
            strategy.retrieve(request, pool)
        })
        .collect();
    
    // 等待所有策略完成，融合结果
    let all_results: Vec<RetrievedContext> = futures::future::join_all(handles).await;
    
    // 按多样性 + 相关性排序，取 top-K
    self.diverse_fusion(&all_results)
}
```

---

## 7. Phase 5: 上下文预测与预热

### 7.1 上下文预测

```rust
pub struct ContextPredictor {
    transition_model: Option<Arc<dyn TransitionPredictor>>,
    prewarm_k: usize,
}

impl ContextPredictor {
    /// 预测接下来的 k 个可能上下文
    pub fn predict_next(
        &self,
        current: &ContextEmbeddings,
        pool: &DualTrackMemory,
    ) -> Vec<PredictedContext> {
        // 1. 在经验池中查找当前上下文的后续模式
        // 2. 使用 embedding 转移概率
        // 3. 返回最可能的 k 个未来上下文
    }
    
    /// 预热: 提前加载预测的上下文到缓存
    pub async fn prewarm(
        &self,
        predictions: &[PredictedContext],
        cache: &ContextCache,
    ) {
        for ctx in predictions {
            cache.prewarm(&ctx.embedding, ctx.priority).await;
        }
    }
}
```

### 7.2 安全设计

```rust
pub struct SafePredictor {
    /// 只做预热，不做预判
    prewarm_enabled: bool,
    /// 预热缓存设置 TTL，避免污染
    prewarm_ttl: Duration,
    /// 实际检索时总是绕过预热缓存进行实时检索
    always_verify: bool,
}
```

**核心原则**：预热只是性能优化，不影响正确性。实际检索总是实时执行。

---

## 8. Phase 6: 监控与可观测性

### 8.1 上下文质量指标

```rust
pub struct ContextMetrics {
    /// 上下文命中率 (检索结果 > 0 的比例)
    pub hit_rate: f64,
    /// 平均置信度
    pub avg_confidence: f64,
    /// 上下文多样性 (检索结果的 embedding 方差)
    pub diversity: f64,
    /// 上下文新鲜度 (平均时间衰减)
    pub freshness: f64,
    /// 窗口大小利用率 (token budget 使用率)
    pub budget_utilization: f64,
    /// 检索延迟 (P50/P95/P99)
    pub retrieval_latency_ms: [f64; 3],
}
```

### 8.2 TUI 调试命令

```
/ctx status          → 显示当前上下文状态
/ctx metrics         → 显示上下文质量指标
/ctx window          → 显示当前上下文窗口内容
/ctx predict         → 显示预测的下一个上下文
/ctx classify <text> → 分类给定文本的上下文类型
/ctx compare <a> <b> → 比较两个上下文的相似度
```

---

## 9. Miss 风险分析

### 9.1 各 Phase Miss 风险矩阵

| Phase | Miss 风险 | 风险原因 | 缓解措施 | 缓解后风险 |
|-------|-----------|---------|---------|-----------|
| **Phase 1**: 多维度 | ⚠️ 中高 | AND 逻辑导致多维度同时匹配失败 | OR + 加权融合，单一维度兜底 | ✅ 低 |
| **Phase 2**: 动态窗口 | ⚠️⚠️ 高 | 时间/语义裁剪过严 | 兜底窗口 + 常青经验 + 渐进放宽 | ✅ 中 |
| **Phase 3**: 语义分块 | ⚠️ 中 | 跨块信息丢失 | 重叠分块 + 邻居块检索 | ✅ 低 |
| **Phase 4**: 上下文路由 | ⚠️⚠️⚠️ 最高 | 分类错误导致策略选错 | 软分类 + 多策略并行 + 融合 | ✅ 中 |
| **Phase 5**: 预测预热 | ⚠️ 低 | 预测错误不影响正确性 | 预热 TTL + 实时验证 | ✅ 极低 |
| **Phase 6**: 监控 | ✅ 无 | 纯观测，不影响决策 | 不适用 | ✅ 无 |

### 9.2 核心缓解原则

1. **Always have a fallback** — 每个阶段都保留原始简单路径作为兜底
2. **OR over AND** — 多维度用加权融合，不用硬性 AND 条件
3. **渐进式放宽** — 结果不足时自动逐步放松阈值
4. **常青经验注入** — 核心规则/高频经验始终保留在上下文中
5. **可观测性优先** — 先能监控，再谈优化

### 9.3 增量安全路径

```
Step 1: Phase 1 (多维度) + 始终保留单一维度兜底
    → Miss 率几乎不变，但命中质量提升

Step 2: Phase 3 (语义分块) + 保留原始全文 embedding 作为兜底
    → Miss 率略升 (< 5%)，但长文档召回大幅提升

Step 3: Phase 2 (动态窗口) + 兜底窗口机制
    → Miss 率可控 (通过调整 fallback 阈值)

Step 4: Phase 4 (上下文路由) + 并行多策略 + 融合
    → Miss 率可能先升后降，需要 tuning

Step 5: Phase 5 (预测预热) + Phase 6 (监控)
    → 纯优化 + 观测，不影响 miss 率
```

---

## 10. 实施路线图

| 阶段 | 内容 | 预估工作量 | 优先级 | Miss 风险 |
|------|------|-----------|--------|-----------|
| **Phase 1** | 多维度上下文表示 + 加权融合 | 2-3天 | 🔥 高 | 低 |
| **Phase 2** | 动态上下文窗口 + 兜底机制 | 3-5天 | 🔥 高 | 中 (可控) |
| **Phase 3** | 语义分块 + Token 预算分配 | 4-6天 | 🔥 高 | 低 |
| **Phase 4** | 层级上下文路由 + 并行策略 | 3-4天 | ⚡ 中 | 中 (需 tuning) |
| **Phase 5** | 上下文预测与预热 | 4-5天 | ⚡ 中 | 极低 |
| **Phase 6** | 监控与可观测性 | 2-3天 | 🧊 低 | 无 |

---

## 11. 快速见效 MVP

### MVP 1: 多维度 SpawnRequest (Phase 1)

**改动范围**：
- 新增 `ContextEmbeddings` struct (含 conversation, domain, intent 等可选维度)
- 修改 `SpawnRequest` 使用新结构
- 修改 L1 `check_confidence` 使用加权多维度融合
- 始终保留原始 3 维度作为兜底

**预期效果**：
- 上下文匹配准确率提升 15-25%
- Miss 率几乎不变 (< 1% 增加)
- 代码侵入性小，可增量上线

### MVP 2: 动态语义窗口 (Phase 2.1)

**改动范围**：
- 实现 `DynamicWindowSelector`（时间衰减 + 语义密度）
- 实现 `SafeDynamicWindow`（主窗口 + 兜底窗口 + 常青经验）
- 替代当前的全局检索

**预期效果**：
- 大经验池下检索速度提升 5-10x
- 上下文质量提升（更聚焦、更相关）
- Miss 率可通过 fallback 参数控制

### MVP 3: 语义分块 (Phase 3.1)

**改动范围**：
- 实现 `SemanticChunker`（重叠分块 + 相邻块检索）
- 在存入经验池前做分块 embedding
- 保留原始全文 embedding 作为兜底

**预期效果**：
- 长文档检索的 recall 大幅提升
- 细粒度上下文匹配
- Miss 率几乎不变

---

## Appendix A: 代码修改清单

| 文件 | 修改内容 |
|------|---------|
| `src/core/types.rs` | 新增 `ContextEmbeddings`，扩展 `SpawnRequest` |
| `src/l1/mod.rs` | 修改 `L1Retriever::check_confidence` 支持多维度 |
| `src/l1/retriever.rs` | 新增 `fused_confidence()`, `score_dimension()` |
| `src/experience/mod.rs` | 新增 `DynamicWindowSelector`, `SafeDynamicWindow` |
| `src/experience/dual_track.rs` | 修改 `search` 支持窗口/时间衰减 |
| `src/experience/chunking.rs` | 新增 `SemanticChunker` |
| `src/experience/summarizer.rs` | 新增 `HierarchicalSummarizer` |
| `src/core/context_router.rs` | 新增 `RobustContextRouter` |
| `src/core/context_classifier.rs` | 新增 `ContextClassifier` |
| `src/core/context_predictor.rs` | 新增 `ContextPredictor` |
| `src/core/context_cache.rs` | 新增 `ContextCache` |
| `src/tui/controller.rs` | 新增 `/ctx` 调试命令 |
| `src/tui/pages/context_page.rs` | 新增上下文监控面板 |

## Appendix B: 关键设计决策

1. **OR 优先于 AND** — 多维度融合使用加权 OR 逻辑，避免硬性多条件 AND 导致的 miss
2. **层层兜底** — 每个增强模块都有 fallback 路径，退化到简单但可靠的原始逻辑
3. **可观测性先行** — 任何改动上线前必须有对应的 metrics 和监控
4. **增量实施** — 不一次性大规模重构，每个 Phase 独立上线验证
5. **常青经验** — 核心规则、高频模板始终保留在上下文中，不被动态窗口裁剪
6. **保守的默认值** — 所有阈值默认偏向"宽松"，宁多勿少，后续再收紧
