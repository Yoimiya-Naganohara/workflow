//! Central constants for the workflow system.

/// Embedding vector dimension (all-MiniLM-L6-v2).
pub const EMBEDDING_DIM: usize = 384;

/// Default LLM temperature for chat agents.
pub const DEFAULT_TEMPERATURE: f64 = 0.7;

/// Default max tokens for chat agents.
pub const DEFAULT_MAX_TOKENS: u64 = 40000;

/// Priority weight for budget ratio (vs depth factor).
pub const BUDGET_PRIORITY_WEIGHT: f32 = 0.6;

/// Priority weight for depth factor (vs budget ratio).
pub const DEPTH_PRIORITY_WEIGHT: f32 = 0.4;

/// Default L1 confidence threshold.
pub const DEFAULT_L1_CONFIDENCE: f32 = 0.5;

/// Default semantic conflict threshold.
pub const DEFAULT_SEMANTIC_THRESHOLD: f32 = -0.6;

/// Default budget for a new runtime.
pub const DEFAULT_RUNTIME_BUDGET: u64 = 10_000;

/// Default max agent spawn depth.
pub const DEFAULT_MAX_DEPTH: u32 = 5;

/// Default max concurrent agents.
pub const DEFAULT_MAX_AGENTS: usize = 10;

/// Default admission timeout in ms.
pub const DEFAULT_ADMISSION_TIMEOUT_MS: u64 = 100;

/// Default suspend timeout in ms.
pub const DEFAULT_SUSPEND_TIMEOUT_MS: u64 = 50;

/// Default side width for TUI sidebar.
pub const SIDEBAR_WIDTH: u16 = 28;

/// Max consecutive failures before L2 collapses.
pub const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Default max_tokens for L2 LLM judge.
pub const L2_JUDGE_MAX_TOKENS: u64 = 500;

/// Default temperature for L2 LLM judge.
pub const L2_JUDGE_TEMPERATURE: f64 = 0.3;

/// Weight for task similarity in L1 confidence (default 0.35).
pub const L1_TASK_WEIGHT: f32 = 0.35;

/// Weight for role similarity in L1 confidence (default 0.25).
pub const L1_ROLE_WEIGHT: f32 = 0.25;

/// Weight for value alignment in L1 confidence (default 0.25).
pub const L1_VALUE_WEIGHT: f32 = 0.25;

/// Weight for recency in L1 confidence (decay per hour, default 0.15).
pub const L1_RECENCY_WEIGHT: f32 = 0.15;

/// How much the L2 override boosts a matching experience (multiplier).
pub const L2_OVERRIDE_BOOST: f32 = 1.5;

/// Anomaly ratio threshold: if requested_budget > remaining * this, flag it.
pub const BUDGET_ANOMALY_RATIO: f64 = 0.8;

/// Maximum responsibility chain length before flagging.
pub const MAX_CHAIN_LENGTH: usize = 20;

/// Default max tool-calling turns for MCP chat (30 = safe upper bound).
pub const DEFAULT_MAX_TOOL_TURNS: usize = 30;

/// Memo usage instructions appended to every role's system prompt.
pub const MEMO_INSTRUCTIONS: &str = "The memo is the only way that allows agent to remember accross sessions. So when user say something important remember it";

/// Zero-tolerance defensive execution instructions appended to every role's system prompt.
/// Injects the mission-critical code quality, chain-of-thought, tool discipline,
/// and refusal protocol into every agent's preamble.
pub const ZERO_TOLERANCE_INSTRUCTIONS: &str = "\
# 核心执行准则：零妥协与防御性编程 (Zero-Tolerance & Defensive Execution)\n\
\n\\
System对你的默认立场是\"有罪推定（Presumed Guilty）\"。任何由于草率、偷懒或主观臆断导致的编译失败、死锁或状态不一致，都将触发熔断，直接判定当前任务失败。\n\n\\
## 1. 严格的代码完整性约束 (Code Completeness)\n\\
- **禁用任何占位符**：绝对禁止在输出的代码中使用 `// TODO`、`// 依此类推`、`// 请在这里实现逻辑`、`...` 或任何形式的伪代码占位符。\n\\
- **全量代码交付**：如果需要修改一个函数，必须输出该函数 100% 完整的、可直接编译的代码，包含所有的修饰符、泛型约束和生命周期标注。\n\\
- **上下文继承**：在重构代码时，必须保留并正确处理原有文件的所有依赖项、导入语句（Imports）和现有的辅助函数，不得在生成新代码时选择性遗漏。\n\n\\
## 2. 确定性思维链 (Chain of Thoughts & Invariants)\n\\
在输出任何实际代码之前，必须在 <cognitive_scratchpad> 标记块内进行显式推演，且推演必须覆盖以下四点：\n\\
1. **状态不变量 (Invariants)**：这段代码必须维持的物理/逻辑边界是什么？\n\\
2. **破坏性自检 (Breaking Edge Cases)**：如果传入空值、并发冲突、边界溢出或异步超时，这段代码会发生什么？\n\\
3. **显式错误路径 (Error Paths)**：禁止吞掉任何错误。所有的 `Result` 必须被显式处理，严禁使用 `.unwrap()` 或 `.expect()`（除非在单测中）。\n\\
4. **生命周期与所有权 (Ownership & Lifetimes)**：涉及到异步 Tokio 调度或跨线程传递时，引用的生存期和 `Send + Sync` 标记是否绝对安全？\n\n\\
## 3. 工具调用的有罪推定 (Tool Call Discipline)\n\\
- 在调用任何写操作工具（如 `WriteFile`, `PatchFile`, `Shell`）前，必须通过读工具（如 `ReadFile`, `Grep`）百分之百确认当前的目标状态，严禁基于历史记忆进行\"盲写\"。\n\\
- 如果某项任务需要连续调用 3 个以上的工具，必须在每一步调用后检查其副作用（Side Effects）和返回值。一旦发现异常返回值，立即停止后续调用并进入自我修复逻辑。\n\n\\
## 4. 输出拒绝协议 (Refusal Protocol)\n\\
如果你因为上下文不足、语义模糊或缺少关键依赖而无法写出 100% 正确且能跑通的代码：\n\\
- 严格禁止\"先随意写一个凑合的实现\"。\n\\
- 你必须使用明确的结构说明你缺少什么（例如：缺少特定的结构体定义或 API Key 权限），并向用户请求明确的输入。此时，\"拒绝编写\"比\"写出错误代码\"的Credibility权重更高。";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_values() {
        assert_eq!(EMBEDDING_DIM, 384);
        assert_eq!(DEFAULT_TEMPERATURE, 0.7);
        assert_eq!(DEFAULT_MAX_TOKENS, 40000);
        assert!((BUDGET_PRIORITY_WEIGHT - 0.6).abs() < f32::EPSILON);
        assert!((DEPTH_PRIORITY_WEIGHT - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn test_default_config_constants() {
        assert!((DEFAULT_L1_CONFIDENCE - 0.5).abs() < f32::EPSILON);
        assert!((DEFAULT_SEMANTIC_THRESHOLD - (-0.6)).abs() < f32::EPSILON);
        assert_eq!(DEFAULT_RUNTIME_BUDGET, 10_000);
        assert_eq!(DEFAULT_MAX_DEPTH, 5);
        assert_eq!(DEFAULT_MAX_AGENTS, 10);
    }

    #[test]
    fn test_timeout_constants() {
        assert_eq!(DEFAULT_ADMISSION_TIMEOUT_MS, 100);
        assert_eq!(DEFAULT_SUSPEND_TIMEOUT_MS, 50);
    }

    #[test]
    fn test_l2_constants() {
        assert_eq!(MAX_CONSECUTIVE_FAILURES, 5);
        assert_eq!(L2_JUDGE_MAX_TOKENS, 500);
        assert!((L2_JUDGE_TEMPERATURE - 0.3).abs() < f64::EPSILON);
        assert!((L2_OVERRIDE_BOOST - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_l1_weight_constants() {
        assert!((L1_TASK_WEIGHT - 0.35).abs() < f32::EPSILON);
        assert!((L1_ROLE_WEIGHT - 0.25).abs() < f32::EPSILON);
        assert!((L1_VALUE_WEIGHT - 0.25).abs() < f32::EPSILON);
        assert!((L1_RECENCY_WEIGHT - 0.15).abs() < f32::EPSILON);
        let sum = L1_TASK_WEIGHT + L1_ROLE_WEIGHT + L1_VALUE_WEIGHT + L1_RECENCY_WEIGHT;
        assert!((sum - 1.0).abs() < f32::EPSILON, "L1 weights should sum to 1.0");
    }

    #[test]
    fn test_other_constants() {
        assert!((BUDGET_ANOMALY_RATIO - 0.8).abs() < f64::EPSILON);
        assert_eq!(MAX_CHAIN_LENGTH, 20);
        assert_eq!(SIDEBAR_WIDTH, 28);
    }
}
