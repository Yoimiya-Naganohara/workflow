//! Status bar — clean, minimal design with model info and key metrics.

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

    let model_name = state.core.selected_models.first()
        .map(|m| m.model_name.as_str())
        .unwrap_or("no model");

    let is_thinking = state.ui.active_chat_requests > 0;

    let left_text = format!("{} • {:.1}%/1.0M (auto)", model_name, ctx_pct);
    let left_width = left_text.len() as u16;
    let remaining = area.width.saturating_sub(left_width + 40);

    let mut spans = vec![
        Span::styled(model_name, Style::default().fg(style::BLUE).add_modifier(ratatui::style::Modifier::BOLD)),
        Span::styled(" • ", Style::default().fg(style::TEXT_MUTED)),
        Span::styled(format!("{:.1}%", ctx_pct), Style::default().fg(style::TEXT_PRIMARY)),
        Span::styled("/1.0M (auto)", Style::default().fg(style::TEXT_MUTED)),
    ];

    // Thinking indicator
    if is_thinking {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let phase = (state.ui.think_frame as usize / 2) % spinner.len();
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(spinner[phase], Style::default().fg(style::YELLOW).add_modifier(ratatui::style::Modifier::BOLD)));
        spans.push(Span::styled(" thinking", Style::default().fg(style::YELLOW)));
    }

    // Metrics
    spans.push(Span::styled("  ", Style::default()));
    spans.push(Span::styled(format!("↑{}k", up_k), Style::default().fg(style::TEXT_SECONDARY)));
    spans.push(Span::styled(" ", Style::default()));
    spans.push(Span::styled(format!("↓{}k", down_k), Style::default().fg(style::TEXT_SECONDARY)));
    spans.push(Span::styled(" ", Style::default()));
    spans.push(Span::styled(format!("R{}M", r_m), Style::default().fg(style::TEXT_SECONDARY)));
    spans.push(Span::styled(" ", Style::default()));
    spans.push(Span::styled(format!("${:.3}", 1.176), Style::default().fg(style::TEXT_SECONDARY)));

    // Fill remaining space
    for _ in 0..remaining { spans.push(Span::raw(" ")); }

    // Key hints
    spans.push(Span::styled("Ctrl+A ", Style::default().fg(style::TEXT_MUTED)));
    spans.push(Span::styled("providers", Style::default().fg(style::BLUE)));
    spans.push(Span::styled("  ", Style::default()));
    spans.push(Span::styled("/", Style::default().fg(style::TEXT_MUTED)));
    spans.push(Span::styled(" cmds", Style::default().fg(style::BLUE)));

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
