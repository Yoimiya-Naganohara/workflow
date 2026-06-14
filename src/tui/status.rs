//! Status bar — clean, minimal design with model info, mode, and key metrics.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::state::{AppMode, AppState};
use super::style;

pub(crate) fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
    // ── Model name ──
    let model_name = state
        .core
        .selected_models
        .first()
        .map(|m| m.model_name.as_str())
        .unwrap_or("no model");

    // ── Mode indicator ──
    let (mode_label, mode_color) = match state.ui.mode {
        AppMode::Plan => ("Plan", style::BLUE),
        AppMode::Build => ("Build", style::GREEN),
    };

    // ── Budget (used / total) ──
    let budget_used_k = state.ui.budget_used as f64 / 1000.0;
    let budget_total_k = state.ui.budget_total / 1000;
    let budget_display = if budget_used_k.fract() == 0.0 {
        format!("{:.0}K/{}K", budget_used_k, budget_total_k)
    } else {
        format!("{:.1}K/{}K", budget_used_k, budget_total_k)
    };

    // ── Token estimates (based on char count, not actual tokenizer) ──
    let total_chars: usize = state
        .core
        .messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    let input_k = total_chars / 4000;
    let output_k = input_k / 4;

    let is_active = state.ui.active_chat_requests > 0;

    let mut spans = vec![
        // Mode badge
        Span::styled(
            mode_label,
            Style::default()
                .fg(mode_color)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" • ", Style::default().fg(style::TEXT_MUTED)),
        // Model name
        Span::styled(
            model_name,
            Style::default()
                .fg(style::BLUE)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::styled(" • ", Style::default().fg(style::TEXT_MUTED)),
        // Budget fraction
        Span::styled(&budget_display, Style::default().fg(style::TEXT_PRIMARY)),
    ];

    // ── Activity spinner ──
    if is_active {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let phase = (state.ui.think_frame as usize / 2) % spinner.len();
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            spinner[phase],
            Style::default()
                .fg(style::YELLOW)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }

    // ── Token metrics ──
    if input_k > 0 || output_k > 0 {
        spans.push(Span::styled("  ", Style::default()));
        if input_k > 0 {
            spans.push(Span::styled(
                format!("↑{}k", input_k),
                Style::default().fg(style::TEXT_SECONDARY),
            ));
        }
        if output_k > 0 {
            spans.push(Span::styled(
                format!(" ↓{}k", output_k),
                Style::default().fg(style::TEXT_SECONDARY),
            ));
        }
    }

    // ── Fill remaining space ──
    let content_width: usize = spans.iter().map(|s| s.content.as_ref().chars().count()).sum();
    let fill = (area.width as usize).saturating_sub(content_width + 1);
    if fill > 0 {
        spans.push(Span::raw(" ".repeat(fill)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
