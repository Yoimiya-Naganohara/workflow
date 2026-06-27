//! Integration tests for the reflection rules engine (public API only).
use workflow::reflection::*;

#[test]
fn test_reflection_config_default() {
    let cfg = ReflectionConfig::default();
    assert!(!cfg.auto_reflect);
    assert_eq!(cfg.max_attempts, 1);
    assert!(cfg.is_rule_enabled("code_complete"));
    assert!(cfg.is_rule_enabled("error_awareness"));
    assert!(cfg.is_rule_enabled("relevance"));
}

#[tokio::test]
async fn test_reflection_report_code_complete_pass() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();
    let ctx = RuleContext {
        input: "implement a sort function",
        response: "```rust\nfn sort() {}\n```",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("code_complete"), RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_code_complete_fail() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();
    let ctx = RuleContext {
        input: "implement",
        response: "just do it",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("code_complete"), RuleVerdict::Fail);
    assert!(!report.all_passed);
}

#[tokio::test]
async fn test_reflection_report_error_awareness() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();

    // Error in trace, not acknowledged → Fail
    let ctx = RuleContext {
        input: "run",
        response: "looks fine",
        tool_trace: "error: permission denied",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("error_awareness"), RuleVerdict::Fail);

    // Error in trace, acknowledged → Pass
    let ctx = RuleContext {
        input: "run",
        response: "there was an error, fixing",
        tool_trace: "error: fail",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("error_awareness"), RuleVerdict::Pass);

    // No error → Skip
    let ctx = RuleContext {
        input: "hi",
        response: "hello",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("error_awareness"), RuleVerdict::Skip);
}

#[tokio::test]
async fn test_reflection_report_multi_question() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();

    // Single Q → Skip
    let ctx = RuleContext {
        input: "how are you?",
        response: "fine",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(
        report.verdict_for("multi_question_coverage"),
        RuleVerdict::Skip
    );

    // Multi Q, short → Fail
    let ctx = RuleContext {
        input: "a? b? c?",
        response: "short",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(
        report.verdict_for("multi_question_coverage"),
        RuleVerdict::Fail
    );
}

#[tokio::test]
async fn test_reflection_report_empty_promise() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();

    // Promise without tools → Fail
    let ctx = RuleContext {
        input: "fix",
        response: "I will fix it later",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("empty_promise"), RuleVerdict::Fail);

    // Promise with tools → Pass
    let ctx = RuleContext {
        input: "fix",
        response: "I will fix it",
        tool_trace: "read_file -> ok",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("empty_promise"), RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_file_ref() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();

    // @file referenced → Pass
    let ctx = RuleContext {
        input: "check @src/main.rs",
        response: "in main.rs",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("file_ref_used"), RuleVerdict::Pass);

    // @file not referenced → Fail
    let ctx = RuleContext {
        input: "check @src/main.rs",
        response: "looks fine",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("file_ref_used"), RuleVerdict::Fail);
}

#[tokio::test]
async fn test_reflection_report_min_output() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();

    let ctx = RuleContext {
        input: "hi",
        response: "ok",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("min_output"), RuleVerdict::Fail);

    let ctx = RuleContext {
        input: "hi",
        response: "this is a sufficiently long response",
        tool_trace: "",
        embedding: None,
        cfg: Some(&cfg),
    };
    let report = check_rules(&cfg, &registry, &ctx).await;
    assert_eq!(report.verdict_for("min_output"), RuleVerdict::Pass);
}

#[tokio::test]
async fn test_reflection_report_all_pass_no_embedding() {
    let cfg = ReflectionConfig::default();
    let registry = default_registry();
    let ctx = RuleContext {
        input: "hello",
        response: "this is a sufficiently long and complete response",
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

// ── Phase 4: new API tests ──

#[test]
fn test_rules_report_failed_rules() {
    let results = vec![
        RuleResult {
            rule_id: "code_complete",
            verdict: RuleVerdict::Pass,
        },
        RuleResult {
            rule_id: "error_awareness",
            verdict: RuleVerdict::Fail,
        },
        RuleResult {
            rule_id: "min_output",
            verdict: RuleVerdict::Fail,
        },
    ];
    let report = RulesReport {
        all_passed: false,
        results,
    };
    let failed = report.failed_rules();
    assert_eq!(failed.len(), 2);
    assert!(failed.contains(&"error_awareness"));
    assert!(failed.contains(&"min_output"));
}

#[test]
fn test_rules_report_verdict_for_missing() {
    let report = RulesReport {
        results: vec![],
        all_passed: true,
    };
    assert_eq!(report.verdict_for("nonexistent"), RuleVerdict::Skip);
}

// ── Phase 5: threshold override ──

#[test]
fn test_threshold_override_none_by_default() {
    let cfg = ReflectionConfig::default();
    assert_eq!(cfg.rule_threshold("relevance"), None);
}

#[test]
fn test_threshold_override_set_returns_value() {
    let mut cfg = ReflectionConfig::default();
    if let Some(rc) = cfg.rules.get_mut("relevance") {
        rc.threshold = Some(0.65);
    }
    assert_eq!(cfg.rule_threshold("relevance"), Some(0.65));
}
