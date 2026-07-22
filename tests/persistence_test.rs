//! Integration tests for persistence (state save/load, sessions).
//!
//! NOTE: All tests in this file run in a single test function to avoid
//! interference from parallel `set_var` calls on HOME/USERPROFILE.

use workflow::persistence;
use workflow::tui::state::{ChatMessage, MessageRole, MessageStatus};

fn make_msg(role: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: match role {
            "user" => MessageRole::User,
            "system" => MessageRole::System,
            _ => MessageRole::User,
        },
        content: content.to_string(),
        reasoning: String::new(),
        chunks: vec![],
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        status: MessageStatus::Completed,
    }
}

fn set_test_home(dir: &std::path::Path) {
    // SAFETY: single-threaded test, no concurrent env reads
    unsafe {
        std::env::set_var("HOME", dir);
        std::env::set_var("USERPROFILE", dir);
    }
}

#[test]
fn test_persistence_all() {
    let dir = tempfile::TempDir::new().expect("should create temp dir");
    set_test_home(dir.path());

    // ── 1. Fresh state is empty ──
    let state = persistence::load();
    assert!(state.api_keys.is_empty());
    assert!(state.selected_models.is_empty());

    // ── 2. Save and reload state ──
    let mut state = persistence::load();
    state
        .api_keys
        .insert("anthropic".to_string(), "sk-test-123".to_string());
    state.configured_providers.push("anthropic".to_string());
    persistence::save(&state).expect("should save state");

    let loaded = persistence::load();
    assert_eq!(
        loaded
            .api_keys
            .get("anthropic")
            .expect("anthropic key should exist"),
        "sk-test-123"
    );
    assert!(
        loaded
            .configured_providers
            .contains(&"anthropic".to_string())
    );

    // ── 3. Session roundtrip ──
    let msgs = vec![make_msg("user", "Hello"), make_msg("user", "Hi there!")];
    persistence::save_session(&msgs).expect("should save session");
    let loaded = persistence::load_session().expect("should load session");
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].content, "Hello");

    // ── 4. Named sessions ──
    persistence::save_session_as("project-x", &[make_msg("user", "Session A")])
        .expect("should save project-x session");
    persistence::save_session_as("project-y", &[make_msg("user", "Session B")])
        .expect("should save project-y session");

    let sessions = persistence::list_sessions();
    assert!(
        sessions.contains(&"project-x".to_string()),
        "sessions: {:?}",
        sessions
    );
    assert!(sessions.contains(&"project-y".to_string()));

    let loaded = persistence::load_session_as("project-x")
        .expect("should load project-x session")
        .expect("project-x session should exist");
    assert_eq!(loaded[0].content, "Session A");

    // ── 5. Delete session ──
    persistence::delete_session("project-y")
        .expect("should delete project-y session");
    let sessions = persistence::list_sessions();
    assert!(!sessions.contains(&"project-y".to_string()));

    // ── 6. Nonexistent session ──
    let result = persistence::load_session_as("nonexistent-session");
    assert!(result.is_none());
}
