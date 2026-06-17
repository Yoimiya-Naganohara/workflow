//! Structured reflection pipeline — rules engine + self-check.
//!
//! After an agent responds, this module runs light-weight local rules
//! to detect common issues. If rules pass, it asks the agent itself
//! a single "yes/no" self-check (1 token). Only if both indicate a
//! problem does it trigger a continuation round.

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
    pub code_complete: RuleVerdict,
    pub error_awareness: RuleVerdict,
    pub multi_question_coverage: RuleVerdict,
    pub empty_promise: RuleVerdict,
    pub file_ref_used: RuleVerdict,
    pub min_output: RuleVerdict,
    pub all_passed: bool,
}

/// Configuration for the reflection engine.
#[derive(Debug, Clone)]
pub struct ReflectionConfig {
    pub auto_reflect: bool,
    pub max_attempts: u8,
    pub rules_enabled: [bool; 6],
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            auto_reflect: false, // default off, user opts in
            max_attempts: 1,     // only one retry
            rules_enabled: [true; 6],
        }
    }
}

/// Indexes into the rules_enabled array.
pub const RULE_CODE_COMPLETE: usize = 0;
pub const RULE_ERROR_AWARENESS: usize = 1;
pub const RULE_MULTI_QUESTION: usize = 2;
pub const RULE_EMPTY_PROMISE: usize = 3;
pub const RULE_FILE_REF_USED: usize = 4;
pub const RULE_MIN_OUTPUT: usize = 5;

// ═══════════════════════════════════════════════════════════════
//  Rules engine
// ═══════════════════════════════════════════════════════════════

/// Run all enabled Level 1 rules.
/// All inputs are string-based; no LLM calls.
pub fn check_rules(
    cfg: &ReflectionConfig,
    input: &str,
    response: &str,
    tool_trace: &str,
) -> RulesReport {
    let code_complete = if cfg.rules_enabled[RULE_CODE_COMPLETE] {
        rule_code_complete(input, response)
    } else {
        RuleVerdict::Skip
    };

    let error_awareness = if cfg.rules_enabled[RULE_ERROR_AWARENESS] {
        rule_error_awareness(input, response, tool_trace)
    } else {
        RuleVerdict::Skip
    };

    let multi_question_coverage = if cfg.rules_enabled[RULE_MULTI_QUESTION] {
        rule_multi_question_coverage(input, response)
    } else {
        RuleVerdict::Skip
    };

    let empty_promise = if cfg.rules_enabled[RULE_EMPTY_PROMISE] {
        rule_empty_promise(response, tool_trace)
    } else {
        RuleVerdict::Skip
    };

    let file_ref_used = if cfg.rules_enabled[RULE_FILE_REF_USED] {
        rule_file_ref_used(input, response)
    } else {
        RuleVerdict::Skip
    };

    let min_output = if cfg.rules_enabled[RULE_MIN_OUTPUT] {
        rule_min_output(response)
    } else {
        RuleVerdict::Skip
    };

    // A rule is "passed" if it's either Pass or Skip (not applicable)
    let results = [
        code_complete,
        error_awareness,
        multi_question_coverage,
        empty_promise,
        file_ref_used,
        min_output,
    ];
    let all_passed = results.iter().all(|r| *r != RuleVerdict::Fail);

    RulesReport {
        code_complete,
        error_awareness,
        multi_question_coverage,
        empty_promise,
        file_ref_used,
        min_output,
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

    // Balance-check: count opening { } [ ] ( ) inside code blocks
    // Simple check: count total braces in response
    let opens: usize = response.matches(['{', '[', '(']).count();
    let closes: usize = response.matches(['}', ']', ')']).count();
    if opens != closes {
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
        || tool_trace.contains("✘");
    if !has_error {
        return RuleVerdict::Skip;
    }

    let acknowledges = response.to_lowercase().contains("error")
        || response.to_lowercase().contains("fail")
        || response.to_lowercase().contains("bug")
        || response.to_lowercase().contains("修正")
        || response.to_lowercase().contains("问题");
    if !acknowledges {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Rule 3: If user asked multiple questions, response should be proportionally long.
fn rule_multi_question_coverage(input: &str, response: &str) -> RuleVerdict {
    let question_count = input.chars().filter(|c| *c == '?').count();
    let numbered_items = input
        .lines()
        .filter(|l| l.trim().starts_with(|c: char| c.is_ascii_digit()))
        .count();
    let total_questions = question_count + numbered_items;

    if total_questions <= 1 {
        return RuleVerdict::Skip;
    }

    let min_len = total_questions * 50;
    if response.len() < min_len {
        return RuleVerdict::Fail;
    }

    RuleVerdict::Pass
}

/// Rule 4: If response makes future-tense promises, there must be tool calls to back them up.
fn rule_empty_promise(response: &str, tool_trace: &str) -> RuleVerdict {
    let has_promise = response.contains("I will")
        || response.contains("接下来")
        || response.contains("下一步")
        || response.contains("接下来我会");
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

    #[test]
    fn test_check_rules_all_pass() {
        let cfg = ReflectionConfig::default();
        let report = check_rules(
            &cfg,
            "hello",
            "this is a sufficiently long and complete response to the user's greeting",
            "",
        );
        assert!(report.all_passed);
    }

    #[test]
    fn test_build_self_check_prompt() {
        let p = build_self_check_prompt("hello", "hi there");
        assert!(p.contains("hello"));
        assert!(p.contains("hi there"));
        assert!(p.contains("yes' or 'no"));
    }
}
