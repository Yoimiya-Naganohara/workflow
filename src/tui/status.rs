//! Status bar — original format: ↑tokens ↓tokens Rtokens $cost ctx% (auto)

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::AppState;
use super::style;

pub(crate) fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let total_chars: usize = state.core.messages.iter().map(|m| m.content.len()).sum();
    let up_k = total_chars / 4000;
    let down_k = up_k / 4;
    let r_m = total_chars / 1_000_000;

    let budget_pct = if state.ui.budget_total > 0 {
        state.ui.budget_used * 100 / state.ui.budget_total
    } else { 0 };
    let ctx_pct = (budget_pct as f64 / 100.0).min(99.9);

    let left_text = format!("↑{}k ↓{}k R{}M ${:.3} {:.1}%/1.0M (auto)", up_k, down_k, r_m, 1.176, ctx_pct);
    let left_width = left_text.len() as u16;
    let remaining = area.width.saturating_sub(left_width + 26);

    let mut spans = vec![
        Span::styled(format!("↑{}k ", up_k), Style::default().fg(style::TEXT2)),
        Span::styled(format!("↓{}k ", down_k), Style::default().fg(style::TEXT2)),
        Span::styled(format!("R{}M ", r_m), Style::default().fg(style::TEXT2)),
        Span::styled(format!("${:.3} ", 1.176), Style::default().fg(style::TEXT2)),
        Span::styled(format!("{:.1}%", ctx_pct), Style::default().fg(style::TEXT)),
        Span::styled("/1.0M (auto)", Style::default().fg(style::TEXT3)),
    ];

    for _ in 0..remaining { spans.push(Span::raw(" ")); }
    spans.push(Span::styled(" Ctrl+A providers  / cmds", Style::default().fg(style::TEXT3)));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
