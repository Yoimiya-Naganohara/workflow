//! Status bar rendering — shows keyboard hints and context usage.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
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
    let hint = if state.active_dialog.is_some() {
        "Esc cancel · Enter confirm".to_string()
    } else if state.ui.focus == super::state::Focus::Chat {
        "↑↓ scroll · g top · G bottom · Ctrl+C quit".to_string()
    } else if state.ui.active_chat_requests > 0 {
        "Ctrl+X stop · Ctrl+C quit".to_string()
    } else {
        "Enter send · Alt+Enter newline · ↑↓ history · /cmd · Tab sidebar · Ctrl+C quit".to_string()
    };

    let total_chars: usize = state.core.messages.iter().map(|m| m.content.len()).sum();
    let ctx_tokens = estimate_tokens(total_chars);
    let ctx_info = if state.ui.context_limit > 0 {
        format!("ctx: {:>4}/{}", ctx_tokens, state.ui.context_limit)
    } else {
        format!("ctx: {:>4}", ctx_tokens)
    };

    let hint_width = hint.len() as u16;
    let ctx_width = ctx_info.len() as u16;
    let pad = area.width.saturating_sub(hint_width + ctx_width + 2);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(hint, style::hint_style()),
            Span::raw(" ".repeat(pad as usize)),
            Span::styled(ctx_info, Style::default().fg(style::SUCCESS)),
        ])),
        area,
    );
}
