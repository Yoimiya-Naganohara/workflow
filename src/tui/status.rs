//! Status bar rendering — shows keyboard hints, provider, and context stats.
//!
//! Simple, clean design matching Claude Code's aesthetic.

use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::AppState;
use super::style;

/// Rough token estimation: ~4 chars per token.
fn estimate_tokens(char_count: usize) -> usize {
    char_count / 4 + 1
}

/// Render the 1-line status bar at the bottom of the terminal.
pub(crate) fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
    // ── Right: keyboard hints ──
    let hint = if state.active_dialog.is_some() {
        "Esc cancel  ·  Enter confirm"
    } else if state.ui.focus == super::state::Focus::Chat {
        "↑↓ scroll  ·  g/G top/bottom  ·  Ctrl+C quit"
    } else if state.ui.active_chat_requests > 0 {
        "Ctrl+X stop  ·  Ctrl+C quit"
    } else {
        "Ctrl+A providers  ·  Ctrl+P commands  ·  / cmds  ·  Esc clear"
    };

    // ── Left: stat counters ──
    let total_chars: usize = state.core.messages.iter().map(|m| m.content.len()).sum();
    let tokens = estimate_tokens(total_chars);

    let user_chars: usize = state
        .core
        .messages
        .iter()
        .filter(|m| matches!(m.role, super::state::MessageRole::User))
        .map(|m| m.content.len())
        .sum();
    let agent_chars: usize = state
        .core
        .messages
        .iter()
        .filter(|m| matches!(m.role, super::state::MessageRole::Agent))
        .map(|m| m.content.len())
        .sum();
    let up_tokens = estimate_tokens(user_chars);
    let down_tokens = estimate_tokens(agent_chars);

    let ctx_pct = if state.ui.context_limit > 0 && tokens > 0 {
        (tokens as f64 / state.ui.context_limit as f64 * 100.0).min(99.9)
    } else {
        0.0
    };

    // Format: hints ... stats
    let stats = vec![
        Span::styled(format!("↑{}k ", up_tokens / 1000), style::value_style()),
        Span::styled(format!("↓{}k ", down_tokens / 1000), style::hint_style()),
        Span::styled(format!("R{}k ", tokens / 1000), style::hint_style()),
        Span::styled(format!("CH{:.1}% ", ctx_pct), style::value_style()),
        Span::styled("$0.000 ", style::hint_style()),
        Span::styled(format!("{:.1}%/1.0M ", ctx_pct), style::value_style()),
    ];

    let stats_text: String = stats.iter().map(|s| s.content.clone()).collect::<Vec<_>>().concat();
    let stats_width = stats_text.len() as u16;
    let hint_width = hint.len() as u16;
    let spacer = if area.width > hint_width + stats_width + 4 {
        area.width - hint_width - stats_width
    } else {
        1
    };

    let mut spans = vec![Span::styled(hint, style::hint_style())];
    if spacer > 1 {
        spans.push(Span::raw(" ".repeat(spacer as usize)));
    }
    spans.extend(stats);

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
