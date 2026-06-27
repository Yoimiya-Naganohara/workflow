# Reflection 模块重构计划：从硬编码数组到 Trait 注册模式

## 状态

- 创建日期：2025-01-xx
- 状态：已批准 ✅
- 影响范围：`src/reflection.rs`, `src/tui/effect.rs`, `src/tui/command_tree/handlers.rs`, `src/tui/state.rs`, `src/core/state.rs`

---

## 一、为什么重构？

### 当前痛点

| 痛点 | 具体表现 | 影响 |
|------|---------|------|
| **规则注册分散** | 加一条规则需改 5 个地方：`const RULE_X`、`RulesReport` 字段、`check_rules` 分支、数组大小、测试 | 开发效率低，容易遗漏 |
| **配置硬编码** | `rules_enabled: [bool; 8]` 用索引寻址 | 调用方需知道常量值，扩展性差 |
| **阈值编译期固定** | `const SEMANTIC_THRESHOLD: f32 = 0.30` | 用户无法运行时微调 |
| **结果报告不可迭代** | `RulesReport` 每规则一个字段 | 批量处理需逐字段 match，新增规则需改所有调用方 |

### 目标

1. **可扩展**：新增规则只需写一个 struct + 在 registry 注册一行
2. **可配置**：规则可独立启/禁，阈值可运行时调整
3. **可迭代**：规则结果可通过 `Vec<RuleResult>` 统一遍历
4. **零破坏迁移**：每个 Phase 独立可提交，不打断 `cargo build && cargo test`

---

## 二、设计概览

```
┌─────────────────────────────────────────────────────┐
│                    RuleRegistry                       │
│  ┌─────────────────────────────────────────────────┐ │
│  │ Vec<Box<dyn ReflectionRule>>                    │ │
│  │  ├─ CodeCompleteRule                            │ │
│  │  ├─ ErrorAwarenessRule                          │ │
│  │  ├─ MultiQuestionCoverageRule                   │ │
│  │  ├─ EmptyPromiseRule                            │ │
│  │  ├─ FileRefUsedRule                             │ │
│  │  ├─ MinOutputRule                               │ │
│  │  ├─ RelevanceRule (needs_embedding)             │ │
│  │  └─ SemanticPromiseRule (needs_embedding)       │ │
│  └─────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
         │ check_all(cfg, ctx)
         ▼
┌─────────────────────────────────────────────────────┐
│                  RulesReport                         │
│  ┌─────────────────────────────────────────────────┐ │
│  │ results: Vec<RuleResult>                        │ │
│  │  ├─ { rule_id: "code_complete", verdict: Pass } │ │
│  │  ├─ { rule_id: "error_awareness", verdict: Fail}│ │
│  │  └─ ...                                          │ │
│  │ all_passed: bool                                │ │
│  └─────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
```

### 核心类型

```rust
// 规则唯一标识符
type RuleId = &'static str;

// 规则执行上下文
struct RuleContext<'a> {
    input: &'a str,
    response: &'a str,
    tool_trace: &'a str,
    embedding: Option<&'a dyn EmbeddingService>,
}

// 规则 trait
#[async_trait]
trait ReflectionRule: Send + Sync {
    fn id(&self) -> RuleId;
    fn description(&self) -> &'static str;
    fn default_enabled(&self) -> bool { true }
    fn needs_embedding(&self) -> bool { false }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict;
}

// 单条结果
struct RuleResult {
    rule_id: RuleId,
    verdict: RuleVerdict,
}

// 注册表
struct RuleRegistry {
    rules: Vec<Box<dyn ReflectionRule>>,
}
```

### 配置模型

```rust
struct ReflectionConfig {
    pub auto_reflect: bool,
    pub max_attempts: u8,
    pub rules: HashMap<RuleId, RuleConfig>,
}

struct RuleConfig {
    pub enabled: bool,
    pub threshold: Option<f32>,  // None = 使用规则默认值
}
```

---

## 三、Phase 划分

### Phase 1：Trait 定义 + 注册表骨架

**类型**：纯新增，零破坏

**改动文件**：仅 `src/reflection.rs`

**内容**：
1. 新增 `RuleId`, `RuleContext`, `RuleResult` 类型
2. 定义 `ReflectionRule` trait
3. 定义 `RuleRegistry` struct + `new()`, `register()`, `get()`, `iter()`, `ids()`
4. 依赖：`async_trait` crate

**验收标准**：
- [x] `cargo build` 通过
- [x] `cargo test` 所有现有测试通过
- [x] 零处现有代码被修改
- [x] 新增测试验证 trait/registry 可用

---

### Phase 2：逐条规则包装为 Trait impl

**类型**：纯新增，零破坏

**改动文件**：仅 `src/reflection.rs`

**内容**：
1. 为 8 条规则各创建一个 struct + `ReflectionRule` impl
   - `CodeCompleteRule`
   - `ErrorAwarenessRule`
   - `MultiQuestionCoverageRule`
   - `EmptyPromiseRule`
   - `FileRefUsedRule`
   - `MinOutputRule`
   - `RelevanceRule { threshold: f32 }`
   - `SemanticPromiseRule { threshold: f32 }`
2. `check()` 方法先委托给现有的自由函数（纯委托，后续可逐步内联）
3. 提供 `default_registry()` 工厂函数

**验收标准**：
- [x] `cargo build` 通过
- [x] `cargo test` 所有现有测试通过
- [x] `default_registry().iter().count() == 8`
- [x] 每条规则 ID 唯一

---

### Phase 3：Config 迁移

**类型**：轻度破坏（需更新 3 个调用方文件）

**改动文件**：
| 文件 | 改动量 |
|------|--------|
| `src/reflection.rs` | ~30 行改写 |
| `src/tui/command_tree/handlers.rs` | ~15 行 |
| `src/tui/state.rs` | ~3 行 |
| `src/core/state.rs` | 0 行（类型引用不变） |

**内容**：
1. `ReflectionConfig.rules_enabled: [bool; 8]` → `rules: HashMap<RuleId, RuleConfig>`
2. 添加 `is_rule_enabled(rule_id)` 和 `rule_threshold(rule_id)` 方法
3. `check_rules` 签名增加 `registry: &RuleRegistry` 和 `ctx: &RuleContext`
4. 更新 effect.rs 的 call site
5. 更新 handlers.rs 的命令分发（常量索引 → 字符串 ID）

**验收标准**：
- [x] `cargo build` 通过
- [x] `cargo test` 所有测试通过
- [x] TUI `/reflection code on/off` 命令仍然正常

---

### Phase 4：Report 迁移

**类型**：轻度破坏（需更新 effect.rs）

**改动文件**：
| 文件 | 改动量 |
|------|--------|
| `src/reflection.rs` | ~60 行 |
| `src/tui/effect.rs` | ~30 行 |

**内容**：
1. `RulesReport` 改为 `{ results: Vec<RuleResult>, all_passed: bool }`
2. 保留兼容方法：`verdict_for(rule_id)`, `failed_rules()`
3. effect.rs 的逐字段 if → 迭代 `report.failed_rules()`
4. 可删除 `RULE_CODE_COMPLETE` 等 8 个 `const usize` 常量

**验收标准**：
- [x] `cargo build` 通过
- [x] `cargo test` 所有测试通过
- [x] 反射续传行为与重构前一致
- [x] 旧常量可安全删除

---

### Phase 5（Bonus）：阈值运行时配置 + 持久化

**类型**：增强功能

**内容**：
1. `RelevanceRule::new()` 接受 `threshold: f32` 参数
2. `check()` 优先使用 `ctx.cfg.rule_threshold(self.id())`
3. 若项目有配置持久化，`ReflectionConfig` 实现 `Serialize/Deserialize`

---

## 四、迁移示意图

### 添加一条新规则：前后对比

```diff
// 当前：改 5 处
- const RULE_X: usize = 8;
- struct RulesReport { ..., pub rule_x: RuleVerdict, }
- fn check_rules(...) { ..., rule_x: rule_x(input, response), }
- let failed = vec![ if report.rule_x == Fail { "X" } ];
- rules_enabled[RULE_X] = true;

// 重构后：改 2 处
+ struct XRule;
+ impl ReflectionRule for XRule { ... }
+ registry.register(XRule);
```

### effect.rs 调用方：前后对比

```diff
// 当前
- let cfg = ReflectionConfig::default();
- let report = check_rules(&cfg, &input, &response, &trace, embed).await;
- if report.code_complete == Fail { ... }
- if report.error_awareness == Fail { ... }
- // ... 8 个 if ...

// 重构后
+ let cfg = ReflectionConfig::default();
+ let registry = default_registry();
+ let ctx = RuleContext { input, response, tool_trace, embedding };
+ let report = check_rules(&cfg, &registry, &ctx).await;
+ for failed_id in report.failed_rules() {
+     match failed_id {
+         "code_complete" => ...,
+         "error_awareness" => ...,
+         _ => ...,
+     }
+ }
```

---

## 五、风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| `async_trait` 增加编译时间 | 高 | 低 | 仅 8 个 impl，可忽略 |
| effect.rs 遗漏字段迁移 | 低 | 中 | Phase 4 用 grep 确认 |
| 生命周期 `'a` 传染 | 低 | 高 | 避免在 async 块中跨 await 持有引用 |
| 语义阈值遗漏 | 中 | 低 | Phase 5 作为 Bonus 不阻塞主线 |

---

## 六、依赖图

```
Phase 1 ──────────────┐
  (Trait+Registry)     │
                       ▼
Phase 2 ────────► Phase 3 ────► Phase 4 ────► Phase 5
  (Rule impls)     (Config)      (Report)      (阈值)
```

- 每个 Phase 独立可提交
- 每个 Phase 前 `cargo build && cargo test` 为绿色
- 合并到 main 前无需 feature flag

---

## 七、附录：8 条规则映射

| 当前函数名 | 新 struct 名 | rule_id | needs_embedding |
|-----------|-------------|---------|-----------------|
| `rule_code_complete` | `CodeCompleteRule` | `"code_complete"` | false |
| `rule_error_awareness` | `ErrorAwarenessRule` | `"error_awareness"` | false |
| `rule_multi_question_coverage` | `MultiQuestionCoverageRule` | `"multi_question_coverage"` | false |
| `rule_empty_promise` | `EmptyPromiseRule` | `"empty_promise"` | false |
| `rule_file_ref_used` | `FileRefUsedRule` | `"file_ref_used"` | false |
| `rule_min_output` | `MinOutputRule` | `"min_output"` | false |
| `rule_relevance` | `RelevanceRule` | `"relevance"` | true |
| `rule_semantic_promise` | `SemanticPromiseRule` | `"semantic_promise"` | true |
