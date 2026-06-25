//! Integration tests for the reflection rules engine (public API only).
use workflow::reflection::*;

#[test]
fn test_reflection_config_default() {
    let cfg = ReflectionConfig::default();
    assert!(!cfg.auto_reflect);
    assert_eq!(cfg.max_attempts, 1);
    assert!(cfg.rules_enabled.iter().all(|&e| e));
}

#[tokio::test]
async fn test_reflection_report_code_complete_pass() {
    let cfg = ReflectionConfig::default();
    let report = check_rules(
        &cfg,
        "implement a sort function",
        "```rust\nfn sort() {}\n```",
        "",
        None,
    )
    .await;
    assert_eq!(report.code_complete, RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_code_complete_fail() {
    let cfg = ReflectionConfig::default();
    let report = check_rules(
        &cfg,
        "implement",
        "just do it",
        "",
        None,
    )
    .await;
    assert_eq!(report.code_complete, RuleVerdict::Fail);
    assert!(!report.all_passed);
}

#[tokio::test]
async fn test_reflection_report_error_awareness() {
    let cfg = ReflectionConfig::default();

    // Error in trace, not acknowledged → Fail
    let report = check_rules(&cfg, "run", "looks fine", "error: permission denied", None).await;
    assert_eq!(report.error_awareness, RuleVerdict::Fail);

    // Error in trace, acknowledged → Pass
    let report = check_rules(&cfg, "run", "there was an error, fixing", "error: fail", None).await;
    assert_eq!(report.error_awareness, RuleVerdict::Pass);

    // No error → Skip
    let report = check_rules(&cfg, "hi", "hello", "", None).await;
    assert_eq!(report.error_awareness, RuleVerdict::Skip);
}

#[tokio::test]
async fn test_reflection_report_multi_question() {
    let cfg = ReflectionConfig::default();

    // Single Q → Skip
    let report = check_rules(&cfg, "how are you?", "fine", "", None).await;
    assert_eq!(report.multi_question_coverage, RuleVerdict::Skip);

    // Multi Q, short → Fail
    let report = check_rules(&cfg, "a? b? c?", "short", "", None).await;
    assert_eq!(report.multi_question_coverage, RuleVerdict::Fail);
}

#[tokio::test]
async fn test_reflection_report_empty_promise() {
    let cfg = ReflectionConfig::default();

    // Promise without tools → Fail
    let report = check_rules(&cfg, "fix", "I will fix it later", "", None).await;
    assert_eq!(report.empty_promise, RuleVerdict::Fail);

    // Promise with tools → Pass
    let report = check_rules(&cfg, "fix", "I will fix it", "read_file -> ok", None).await;
    assert_eq!(report.empty_promise, RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_file_ref() {
    let cfg = ReflectionConfig::default();

    // @file referenced → Pass
    let report = check_rules(&cfg, "check @src/main.rs", "in main.rs", "", None).await;
    assert_eq!(report.file_ref_used, RuleVerdict::Pass);

    // @file not referenced → Fail
    let report = check_rules(&cfg, "check @src/main.rs", "looks fine", "", None).await;
    assert_eq!(report.file_ref_used, RuleVerdict::Fail);
}

#[tokio::test]
async fn test_reflection_report_min_output() {
    let cfg = ReflectionConfig::default();

    let report = check_rules(&cfg, "hi", "ok", "", None).await;
    assert_eq!(report.min_output, RuleVerdict::Fail);

    let report = check_rules(&cfg, "hi", "this is a sufficiently long response", "", None).await;
    assert_eq!(report.min_output, RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_all_pass_no_embedding() {
    let cfg = ReflectionConfig::default();
    let report = check_rules(
        &cfg,
        "hello",
        "this is a sufficiently long and complete response",
        "",
        None,
    )
    .await;
    assert!(report.all_passed);
    assert_eq!(report.relevance, RuleVerdict::Skip);
    assert_eq!(report.semantic_promise, RuleVerdict::Skip);
}

#[test]
fn test_build_self_check_prompt() {
    let prompt = build_self_check_prompt("user question", "agent answer");
    assert!(prompt.contains("user question"));
    assert!(prompt.contains("agent answer"));
    assert!(prompt.contains("'yes' or 'no'"));
}

#[test]
fn test_build_continuation_feedback() {
    let feedback = build_continuation_feedback(&["code_complete", "min_output"]);
    assert!(feedback.contains("code_complete"));
    assert!(feedback.contains("min_output"));
    assert!(feedback.contains("improve"));
}
