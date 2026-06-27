//! Structured reflection pipeline — rules engine + self-check.
//!
//! After an agent responds, this module runs light-weight local rules
//! to detect common issues. If rules pass, it asks the agent itself
//! a single "yes/no" self-check (1 token). Only if both indicate a
//! problem does it trigger a continuation round.
//!
//! Semantic rules (relevance, semantic_promise) use the embedding service
//! when available. Pass `None` to skip them and fall back to heuristic rules.

use std::collections::HashMap;

use crate::core::simd::cosine_similarity_384;
use crate::llm::EmbeddingService;
use async_trait::async_trait;

// ═══════════════════════════════════════════════════════════════
//  Types
// ═══════════════════════════════════════════════════════════════

/// Per-dimension rule result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RuleVerdict {
    Pass,
    Fail,
    Skip, // rule not applicable to this request
}

/// Outcome of the Level 1 rules pass.
#[derive(Debug, Clone)]
pub struct RulesReport {
    /// Ordered list of rule results (one per registered rule).
    pub results: Vec<RuleResult>,
    /// True if no rule has verdict `Fail`.
    pub all_passed: bool,
}

impl RulesReport {
    /// Return the verdict for a specific rule by ID.
    pub fn verdict_for(&self, rule_id: RuleId) -> RuleVerdict {
        self.results
            .iter()
            .find(|r| r.rule_id == rule_id)
            .map(|r| r.verdict)
            .unwrap_or(RuleVerdict::Skip)
    }

    /// Return rule IDs that failed.
    pub fn failed_rules(&self) -> Vec<&str> {
        self.results
            .iter()
            .filter(|r| r.verdict == RuleVerdict::Fail)
            .map(|r| r.rule_id)
            .collect()
    }

    /// Return results with `Fail` verdict.
    pub fn failed_results(&self) -> Vec<&RuleResult> {
        self.results
            .iter()
            .filter(|r| r.verdict == RuleVerdict::Fail)
            .collect()
    }

    /// Return results with `Pass` verdict.
    pub fn passed_results(&self) -> Vec<&RuleResult> {
        self.results
            .iter()
            .filter(|r| r.verdict == RuleVerdict::Pass)
            .collect()
    }
}

/// Per-rule configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuleConfig {
    pub enabled: bool,
    /// Override threshold (None = use rule's built-in default).
    pub threshold: Option<f32>,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: None,
        }
    }
}

/// Configuration for the reflection engine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReflectionConfig {
    pub auto_reflect: bool,
    pub max_attempts: u8,
    pub rules: HashMap<String, RuleConfig>,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            auto_reflect: false,
            max_attempts: 1,
            rules: HashMap::from([
                ("code_complete".to_string(), RuleConfig::default()),
                ("error_awareness".to_string(), RuleConfig::default()),
                ("multi_question_coverage".to_string(), RuleConfig::default()),
                ("empty_promise".to_string(), RuleConfig::default()),
                ("file_ref_used".to_string(), RuleConfig::default()),
                ("min_output".to_string(), RuleConfig::default()),
                ("relevance".to_string(), RuleConfig::default()),
                ("semantic_promise".to_string(), RuleConfig::default()),
            ]),
        }
    }
}

impl ReflectionConfig {
    /// Returns whether a rule is enabled (defaults to true if not configured).
    pub fn is_rule_enabled(&self, rule_id: &str) -> bool {
        self.rules.get(rule_id).map(|c| c.enabled).unwrap_or(true)
    }

    /// Returns an override threshold for a rule.
    /// `None` → use the rule's built-in default.
    pub fn rule_threshold(&self, rule_id: &str) -> Option<f32> {
        self.rules.get(rule_id).and_then(|c| c.threshold)
    }
}

// ═══════════════════════════════════════════════════════════════
//  Phase 1: ReflectionRule trait + Registry
// ═══════════════════════════════════════════════════════════════

/// Unique rule identifier string.
pub type RuleId = &'static str;

/// Context passed to every [`ReflectionRule::check`].
pub struct RuleContext<'a> {
    pub input: &'a str,
    pub response: &'a str,
    pub tool_trace: &'a str,
    pub embedding: Option<&'a dyn EmbeddingService>,
    /// Optional reflection config for runtime threshold overrides.
    /// `None` → rules use their built-in defaults.
    pub cfg: Option<&'a ReflectionConfig>,
}

/// Single rule evaluation result.
#[derive(Debug, Clone)]
pub struct RuleResult {
    pub rule_id: RuleId,
    pub verdict: RuleVerdict,
}

/// A single reflection rule that can be registered and checked.
#[async_trait]
pub trait ReflectionRule: Send + Sync {
    fn id(&self) -> RuleId;
    fn description(&self) -> &'static str;
    fn default_enabled(&self) -> bool {
        true
    }
    fn needs_embedding(&self) -> bool {
        false
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict;
}

/// Registry of active reflection rules.
pub struct RuleRegistry {
    rules: Vec<Box<dyn ReflectionRule>>,
}

impl RuleRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Register a rule.
    pub fn register(&mut self, rule: Box<dyn ReflectionRule>) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Look up a rule by ID.
    pub fn get(&self, id: RuleId) -> Option<&dyn ReflectionRule> {
        self.rules.iter().find(|r| r.id() == id).map(|r| r.as_ref())
    }

    /// Iterate over registered rules.
    pub fn iter(&self) -> impl Iterator<Item = &dyn ReflectionRule> {
        self.rules.iter().map(|r| r.as_ref())
    }

    /// Return all rule IDs.
    pub fn ids(&self) -> Vec<RuleId> {
        self.rules.iter().map(|r| r.id()).collect()
    }

    /// Number of registered rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════
//  Rules engine
// ═══════════════════════════════════════════════════════════════

/// Run all enabled Level 1 rules via the registry.
///
/// Each registered rule is checked in order. Semantic rules are skipped
/// when `ctx.embedding` is `None`.
pub async fn check_rules(
    cfg: &ReflectionConfig,
    registry: &RuleRegistry,
    ctx: &RuleContext<'_>,
) -> RulesReport {
    let mut results: Vec<RuleResult> = Vec::new();
    for rule in registry.iter() {
        if cfg.is_rule_enabled(rule.id()) {
            let verdict = rule.check(ctx).await;
            results.push(RuleResult {
                rule_id: rule.id(),
                verdict,
            });
        }
    }

    let all_passed = results.iter().all(|r| r.verdict != RuleVerdict::Fail);

    RulesReport {
        results,
        all_passed,
    }
}

/// Rule 1: If user asked for code, response must contain a fenced code block
/// with balanced braces.
fn rule_code_complete(input: &str, response: &str) -> RuleVerdict {
    let wants_code = input.contains("code")
        || input.contains("implement")
        || input.contains("函数")
        || input.contains("实现")
        || input.contains("fix")
        || input.contains("write ");
    if !wants_code {
        return RuleVerdict::Skip;
    }

    // Must have at least one fenced code block
    if !response.contains("```") {
        return RuleVerdict::Fail;
    }

    // Stack-based brace balance check scoped to code block regions only.
    // This avoids false positives from natural-language braces outside blocks.
    let mut in_block = false;
    let mut stack: Vec<char> = Vec::new();
    for line in response.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if in_block {
            for ch in line.chars() {
                match ch {
                    '{' | '[' | '(' => stack.push(ch),
                    '}' => {
                        if stack.last() == Some(&'{') {
                            stack.pop();
                        } else {
                            return RuleVerdict::Fail;
                        }
                    }
                    ']' => {
                        if stack.last() == Some(&'[') {
                            stack.pop();
                        } else {
                            return RuleVerdict::Fail;
                        }
                    }
                    ')' => {
                        if stack.last() == Some(&'(') {
                            stack.pop();
                        } else {
                            return RuleVerdict::Fail;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    if !stack.is_empty() {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Rule 2: If any tool call returned an error, response must acknowledge it.
fn rule_error_awareness(_input: &str, response: &str, tool_trace: &str) -> RuleVerdict {
    let has_error = tool_trace.to_lowercase().contains("error")
        || tool_trace.to_lowercase().contains("stderr")
        || tool_trace.to_lowercase().contains("fail")
        || tool_trace.contains("✗")
        || tool_trace.contains("✘")
        || tool_trace.contains("错误")
        || tool_trace.contains("失败")
        || tool_trace.contains("异常")
        || tool_trace.contains("报错");
    if !has_error {
        return RuleVerdict::Skip;
    }

    let ack_lower = response.to_lowercase();
    let acknowledges = ack_lower.contains("error")
        || ack_lower.contains("fail")
        || ack_lower.contains("bug")
        || response.contains("修正")
        || response.contains("问题")
        || response.contains("错误")
        || response.contains("失败")
        || response.contains("报错")
        || response.contains("可以忽略");
    if !acknowledges {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Rule 3: If user asked multiple questions, response should be proportionally long.
fn rule_multi_question_coverage(input: &str, response: &str) -> RuleVerdict {
    let question_count = input.chars().filter(|c| *c == '?' || *c == '？').count();
    let numbered_items = input
        .lines()
        .filter(|l| {
            let t = l.trim();
            t.starts_with(|c: char| c.is_ascii_digit())
                || t.starts_with("一、")
                || t.starts_with("二、")
                || t.starts_with("三、")
                || t.starts_with("四、")
                || t.starts_with("五、")
                || t.starts_with("第一")
                || t.starts_with("第二")
                || t.starts_with("第三")
                || t.starts_with("首先")
                || t.starts_with("其次")
                || t.starts_with("最后")
        })
        .count();
    let total_questions = question_count + numbered_items;

    if total_questions <= 1 {
        return RuleVerdict::Skip;
    }

    let min_len = total_questions * 30;
    if response.len() < min_len {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Shared keywords for future-tense promise detection across heuristic and semantic rules.
const PROMISE_INDICATORS: &[&str] = &["I will", "接下来", "下一步", "接下来我会"];

/// Rule 4: If response makes future-tense promises, there must be tool calls to back them up.
fn rule_empty_promise(response: &str, tool_trace: &str) -> RuleVerdict {
    let has_promise = PROMISE_INDICATORS.iter().any(|p| response.contains(p));
    if !has_promise {
        return RuleVerdict::Skip;
    }

    let has_tool_calls = !tool_trace.is_empty();
    if !has_tool_calls {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Rule 5: If input contains @file references, response must reference them.
fn rule_file_ref_used(input: &str, response: &str) -> RuleVerdict {
    let has_ref = input.contains('@');
    if !has_ref {
        return RuleVerdict::Skip;
    }

    // Extract file paths from input
    let paths: Vec<&str> = input
        .split_whitespace()
        .filter(|w| w.starts_with('@'))
        .map(|w| w.trim_start_matches('@'))
        .collect();

    if paths.is_empty() {
        return RuleVerdict::Skip;
    }

    // Check if at least one file name or key identifier appears in response
    for path in &paths {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = name.rsplit('.').next().unwrap_or(name);
        if response.contains(stem) || response.contains(path) {
            return RuleVerdict::Pass;
        }
    }

    RuleVerdict::Fail
}

/// Rule 6: Response must have meaningful length.
fn rule_min_output(response: &str) -> RuleVerdict {
    if response.trim().len() < 20 {
        RuleVerdict::Fail
    } else {
        RuleVerdict::Pass
    }
}

// ═══════════════════════════════════════════════════════════════
//  Semantic rules (embedding-based)
// ═══════════════════════════════════════════════════════════════

/// Built-in default threshold for semantic rules.
const SEMANTIC_THRESHOLD: f32 = 0.50;

/// Rule 7: Semantic relevance — embed question + response, check cosine
/// similarity.  A low score indicates the response is not about the question.
///
/// When `threshold_override` is `Some`, it takes precedence over `SEMANTIC_THRESHOLD`.
async fn rule_relevance(
    embedding: &dyn EmbeddingService,
    input: &str,
    response: &str,
    threshold_override: Option<f32>,
) -> RuleVerdict {
    let threshold = threshold_override.unwrap_or(SEMANTIC_THRESHOLD);
    let (q_res, r_res) = tokio::join!(embedding.embed(input), embedding.embed(response));
    match (q_res, r_res) {
        (Ok(q), Ok(r)) => {
            let sim = cosine_similarity_384(&q, &r);
            if sim < threshold {
                tracing::debug!(
                    input_len = input.len(),
                    response_len = response.len(),
                    sim,
                    threshold,
                    "relevance rule FAIL"
                );
                RuleVerdict::Fail
            } else {
                RuleVerdict::Pass
            }
        }
        _ => RuleVerdict::Skip, // embedding failure → don't penalise
    }
}

/// Rule 8: Semantic promise check — if the response contains future-tense
/// promises, embed the promise segment and the tool trace, and verify they
/// are semantically related.  Falls back to the heuristic rule when the
/// tool trace is empty.
///
/// When `threshold_override` is `Some`, it takes precedence over `SEMANTIC_THRESHOLD`.
async fn rule_semantic_promise(
    embedding: &dyn EmbeddingService,
    response: &str,
    tool_trace: &str,
    threshold_override: Option<f32>,
) -> RuleVerdict {
    // Extract the first sentence containing a promise indicator.
    let promise_sentence = response
        .lines()
        .find(|l| PROMISE_INDICATORS.iter().any(|p| l.contains(p)))
        .unwrap_or("");

    if promise_sentence.is_empty() {
        return RuleVerdict::Skip;
    }

    // If there is no tool trace at all, this is the same as the heuristic
    // empty_promise rule — no semantic check needed.
    if tool_trace.is_empty() {
        return RuleVerdict::Skip;
    }

    let threshold = threshold_override.unwrap_or(SEMANTIC_THRESHOLD);
    let (p_res, t_res) = tokio::join!(
        embedding.embed(promise_sentence),
        embedding.embed(tool_trace)
    );
    match (p_res, t_res) {
        (Ok(p), Ok(t)) => {
            let sim = cosine_similarity_384(&p, &t);
            if sim < threshold {
                tracing::debug!(sim, threshold, "semantic_promise rule FAIL");
                RuleVerdict::Fail
            } else {
                RuleVerdict::Pass
            }
        }
        _ => RuleVerdict::Skip,
    }
}

// ═══════════════════════════════════════════════════════════════
//  Prompt builders
// ═══════════════════════════════════════════════════════════════

/// Build the self-check prompt (agent evaluates its own response).
/// The agent replies with exactly "yes" or "no" — 1 token.
pub fn build_self_check_prompt(user_input: &str, agent_response: &str) -> String {
    format!(
        "You previously responded to this request:\n\
         ---\n\
         {}\n\
         ---\n\n\
         Your response was:\n\
         ---\n\
         {}\n\
         ---\n\n\
         Have you fully completed the user's request? \
         Reply with exactly 'yes' or 'no'.\n\
         Do not add any other text.",
        user_input, agent_response
    )
}

/// Build the continuation prompt when a retry is needed.
pub fn build_continuation_feedback(failed_rules: &[&str]) -> String {
    let mut msg =
        String::from("Please review and improve your previous response.\n\nIssues found:\n");
    for rule in failed_rules {
        msg.push_str(&format!("- {}\n", rule));
    }
    msg.push_str("\nPlease address the above and provide a complete response.");
    msg
}

// ═══════════════════════════════════════════════════════════════
//  Phase 2: Rule struct implementations
// ═══════════════════════════════════════════════════════════════

pub struct CodeCompleteRule;
pub struct ErrorAwarenessRule;
pub struct MultiQuestionCoverageRule;
pub struct EmptyPromiseRule;
pub struct FileRefUsedRule;
pub struct MinOutputRule;

pub struct RelevanceRule {
    pub threshold: f32,
}

pub struct SemanticPromiseRule {
    pub threshold: f32,
}

#[async_trait]
impl ReflectionRule for CodeCompleteRule {
    fn id(&self) -> RuleId {
        "code_complete"
    }
    fn description(&self) -> &'static str {
        "Code blocks must be complete with balanced braces"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_code_complete(ctx.input, ctx.response)
    }
}

#[async_trait]
impl ReflectionRule for ErrorAwarenessRule {
    fn id(&self) -> RuleId {
        "error_awareness"
    }
    fn description(&self) -> &'static str {
        "Tool call errors must be acknowledged in the response"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_error_awareness(ctx.input, ctx.response, ctx.tool_trace)
    }
}

#[async_trait]
impl ReflectionRule for MultiQuestionCoverageRule {
    fn id(&self) -> RuleId {
        "multi_question_coverage"
    }
    fn description(&self) -> &'static str {
        "Multiple questions must receive a proportionally long response"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_multi_question_coverage(ctx.input, ctx.response)
    }
}

#[async_trait]
impl ReflectionRule for EmptyPromiseRule {
    fn id(&self) -> RuleId {
        "empty_promise"
    }
    fn description(&self) -> &'static str {
        "Future-tense promises must be backed by tool calls"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_empty_promise(ctx.response, ctx.tool_trace)
    }
}

#[async_trait]
impl ReflectionRule for FileRefUsedRule {
    fn id(&self) -> RuleId {
        "file_ref_used"
    }
    fn description(&self) -> &'static str {
        "@file references in input must be addressed in the response"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_file_ref_used(ctx.input, ctx.response)
    }
}

#[async_trait]
impl ReflectionRule for MinOutputRule {
    fn id(&self) -> RuleId {
        "min_output"
    }
    fn description(&self) -> &'static str {
        "Response must have meaningful length"
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        rule_min_output(ctx.response)
    }
}

#[async_trait]
impl ReflectionRule for RelevanceRule {
    fn id(&self) -> RuleId {
        "relevance"
    }
    fn description(&self) -> &'static str {
        "Response must be semantically related to the question"
    }
    fn needs_embedding(&self) -> bool {
        true
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        if let Some(emb) = ctx.embedding {
            let override_threshold = ctx.cfg.and_then(|c| c.rule_threshold(self.id()));
            rule_relevance(emb, ctx.input, ctx.response, override_threshold).await
        } else {
            RuleVerdict::Skip
        }
    }
}

#[async_trait]
impl ReflectionRule for SemanticPromiseRule {
    fn id(&self) -> RuleId {
        "semantic_promise"
    }
    fn description(&self) -> &'static str {
        "Future-tense promises must be semantically backed by tool execution"
    }
    fn needs_embedding(&self) -> bool {
        true
    }
    async fn check(&self, ctx: &RuleContext<'_>) -> RuleVerdict {
        if let Some(emb) = ctx.embedding {
            let override_threshold = ctx.cfg.and_then(|c| c.rule_threshold(self.id()));
            rule_semantic_promise(emb, ctx.response, ctx.tool_trace, override_threshold).await
        } else {
            RuleVerdict::Skip
        }
    }
}

/// Build the default registry with all 8 rules registered.
pub fn default_registry() -> RuleRegistry {
    let mut reg = RuleRegistry::new();
    reg.register(Box::new(CodeCompleteRule));
    reg.register(Box::new(ErrorAwarenessRule));
    reg.register(Box::new(MultiQuestionCoverageRule));
    reg.register(Box::new(EmptyPromiseRule));
    reg.register(Box::new(FileRefUsedRule));
    reg.register(Box::new(MinOutputRule));
    reg.register(Box::new(RelevanceRule {
        threshold: SEMANTIC_THRESHOLD,
    }));
    reg.register(Box::new(SemanticPromiseRule {
        threshold: SEMANTIC_THRESHOLD,
    }));
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule_code_complete_pass() {
        let result = rule_code_complete("implement a function", "```rust\nfn main() {}\n```");
        assert_eq!(result, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_code_complete_fail_no_code_block() {
        let result = rule_code_complete("write a function", "just do it");
        assert_eq!(result, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_code_complete_unbalanced() {
        let result = rule_code_complete("implement", "```\nfn foo() {\n```");
        assert_eq!(result, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_error_awareness_skip_no_error() {
        let r = rule_error_awareness("hello", "fine", "");
        assert_eq!(r, RuleVerdict::Skip);
    }

    #[test]
    fn test_rule_error_awareness_fail() {
        let r = rule_error_awareness("run", "ok", "error: not found");
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_error_awareness_pass() {
        let r = rule_error_awareness("run", "got an error, fixing", "error: not found");
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_multi_question_skip() {
        let r = rule_multi_question_coverage("hello", "hi");
        assert_eq!(r, RuleVerdict::Skip);
    }

    #[test]
    fn test_rule_multi_question_fail() {
        let r = rule_multi_question_coverage("a? b? c?", "short");
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_empty_promise_skip_no_promise() {
        let r = rule_empty_promise("done", "");
        assert_eq!(r, RuleVerdict::Skip);
    }

    #[test]
    fn test_rule_empty_promise_fail() {
        let r = rule_empty_promise("I will fix it later", "");
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_empty_promise_pass_with_tools() {
        let r = rule_empty_promise("I will fix it", "read_file -> ok");
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_file_ref_used_pass() {
        let r = rule_file_ref_used("check @src/main.rs", "in main.rs we see");
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_file_ref_used_fail() {
        let r = rule_file_ref_used("check @src/main.rs", "it looks fine");
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_min_output_fail() {
        let r = rule_min_output("ok");
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_min_output_pass() {
        let r = rule_min_output("this is a long enough response to pass the rule");
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[tokio::test]
    async fn test_check_rules_all_pass_no_embedding() {
        let cfg = ReflectionConfig::default();
        let registry = default_registry();
        let ctx = RuleContext {
            input: "hello",
            response: "this is a sufficiently long and complete response to the user's greeting",
            tool_trace: "",
            embedding: None,
            cfg: Some(&cfg),
        };
        let report = check_rules(&cfg, &registry, &ctx).await;
        assert!(report.all_passed);
        assert_eq!(report.verdict_for("relevance"), RuleVerdict::Skip);
        assert_eq!(report.verdict_for("semantic_promise"), RuleVerdict::Skip);
    }

    #[test]
    fn test_build_self_check_prompt() {
        let p = build_self_check_prompt("hello", "hi there");
        assert!(p.contains("hello"));
        assert!(p.contains("hi there"));
        assert!(p.contains("yes' or 'no"));
    }

    // ── Semantic rule tests (require fastembed / ONNX) ──

    fn try_embedding() -> Option<crate::llm::embedding::EmbeddingService> {
        std::panic::catch_unwind(crate::llm::embedding::EmbeddingService::new).ok()
    }

    #[tokio::test]
    async fn test_rule_relevance_related() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let r = rule_relevance(
            &emb,
            "how to sort a vector in Rust?",
            "Use sort() on Vec<T>.",
            None,
        )
        .await;
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[tokio::test]
    async fn test_rule_relevance_unrelated() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let r = rule_relevance(
            &emb,
            "how to bake a cake",
            "The quick brown fox jumps.",
            None,
        )
        .await;
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[tokio::test]
    async fn test_rule_semantic_promise_skip_no_promise() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let r = rule_semantic_promise(&emb, "Done!", "read_file -> ok", None).await;
        assert_eq!(r, RuleVerdict::Skip);
    }

    #[tokio::test]
    async fn test_rule_semantic_promise_skip_no_tools() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let r = rule_semantic_promise(&emb, "I will fix the bug now", "", None).await;
        assert_eq!(r, RuleVerdict::Skip);
    }

    #[tokio::test]
    async fn test_rule_semantic_promise_pass() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        // Promise about installing dependencies, tool output confirms installation.
        let r = rule_semantic_promise(
            &emb,
            "I will install the required npm packages",
            "npm install: added 42 packages in 3.2s",
            None,
        )
        .await;
        assert_eq!(r, RuleVerdict::Pass);
    }

    #[tokio::test]
    async fn test_rule_semantic_promise_fail() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        // Promise about installing packages, but tools are about deploying to AWS.
        let r = rule_semantic_promise(
            &emb,
            "I will install the required npm packages",
            "aws s3 sync /dist s3://bucket",
            None,
        )
        .await;
        assert_eq!(r, RuleVerdict::Fail);
    }

    #[tokio::test]
    async fn test_check_rules_with_embedding() {
        let Some(emb) = try_embedding() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let cfg = ReflectionConfig::default();
        let registry = default_registry();
        let ctx = RuleContext {
            input: "how do I sort a vector in Rust in descending order",
            response: "You can sort a vector in descending order by calling sort() then reverse(), or use sort_by() with a custom comparator. Both approaches work in-place.",
            tool_trace: "",
            embedding: Some(&emb as &dyn crate::llm::EmbeddingService),
            cfg: Some(&cfg),
        };
        let report = check_rules(&cfg, &registry, &ctx).await;
        assert!(report.all_passed);
        assert_eq!(report.verdict_for("relevance"), RuleVerdict::Pass);
    }

    // ═══════════════════════════════════════════════════════════
    //  Phase 1 + 2: Trait / Registry / Rule structs
    // ═══════════════════════════════════════════════════════════

    #[test]
    fn test_default_registry_has_eight_rules() {
        let reg = default_registry();
        assert_eq!(reg.len(), 8);
    }

    #[test]
    fn test_default_registry_all_ids_unique() {
        let reg = default_registry();
        let ids = reg.ids();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate rule IDs");
    }

    #[test]
    fn test_registry_get_finds_rule() {
        let reg = default_registry();
        let rule = reg.get("code_complete").expect("code_complete rule");
        assert_eq!(rule.id(), "code_complete");
        assert!(!rule.needs_embedding());
    }

    #[test]
    fn test_registry_get_missing_returns_none() {
        let reg = default_registry();
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_empty_new() {
        let reg = RuleRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_rule_struct_code_complete_delegates() {
        let rule = CodeCompleteRule;
        let ctx = RuleContext {
            input: "implement a function",
            response: "```rust\nfn main() {}\n```",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_struct_code_complete_fail() {
        let rule = CodeCompleteRule;
        let ctx = RuleContext {
            input: "write a function",
            response: "just do it",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_struct_error_awareness_skip() {
        let rule = ErrorAwarenessRule;
        let ctx = RuleContext {
            input: "hello",
            response: "fine",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Skip);
    }

    #[test]
    fn test_rule_struct_empty_promise_fail() {
        let rule = EmptyPromiseRule;
        let ctx = RuleContext {
            input: "fix it",
            response: "I will fix it later",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Fail);
    }

    #[test]
    fn test_rule_struct_min_output_pass() {
        let rule = MinOutputRule;
        let ctx = RuleContext {
            input: "hello",
            response: "this is a long enough response to pass the rule",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Pass);
    }

    #[test]
    fn test_rule_struct_relevance_skips_without_embedding() {
        let rule = RelevanceRule { threshold: 0.5 };
        let ctx = RuleContext {
            input: "hello",
            response: "world",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Skip);
    }

    #[test]
    fn test_rule_struct_file_ref_used_fail() {
        let rule = FileRefUsedRule;
        let ctx = RuleContext {
            input: "check @src/main.rs",
            response: "it looks fine",
            tool_trace: "",
            embedding: None,
            cfg: None,
        };
        let result = futures::executor::block_on(rule.check(&ctx));
        assert_eq!(result, RuleVerdict::Fail);
    }
}
