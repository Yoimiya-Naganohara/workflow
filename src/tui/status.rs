//! Status bar — clean, minimal design with model info, mode, and key metrics.
//!
//! Layout (left → right):
//!   {Mode} • {Provider} • {Model} [{CapBadge}] ctx:{N}K • {Budget}  [{Spinner}]  ↑Xk ↓Xk  [⚠T]

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use super::state::{AppMode, AppState};
use super::style;

/// Render the one-line status bar at the bottom of the screen.
pub(crate) fn render_status_bar<'a>(f: &mut Frame, area: Rect, state: &'a AppState) {
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(24);

    // ── 1. Mode badge ──
    let (mode_label, mode_color) = match state.ui.mode {
        AppMode::Plan => ("Plan", style::BLUE),
        AppMode::Build => ("Build", style::GREEN),
    };
    spans.push(Span::styled(
        mode_label,
        Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
    ));

    // ── 2. Separator ──
    push_sep(&mut spans);

    // ── 3. Model info (provider • model_name [cap_badge] ctx:N) ──
    if let Some(sel) = state.core.selected_models.first() {
        // Provider name
        spans.push(Span::styled(
            &sel.provider_name,
            Style::default().fg(style::TEXT_SECONDARY),
        ));
        spans.push(Span::styled(" • ", Style::default().fg(style::TEXT_MUTED)));

        // Model name
        spans.push(Span::styled(
            &sel.model_name,
            Style::default().fg(style::BLUE).add_modifier(Modifier::BOLD),
        ));

        // Capability badge + context window
        if let Some(caps) = state.model_capabilities() {
            let badge = format_cap_badge(&caps);
            let ctx = if caps.max_context >= 1024 {
                format!("{}K", caps.max_context / 1024)
            } else {
                caps.max_context.to_string()
            };
            spans.push(Span::styled(" ", Style::default()));
            spans.push(Span::styled(badge, Style::default().fg(style::TEXT_SECONDARY)));
            spans.push(Span::styled(
                format!(" ctx:{}", ctx),
                Style::default().fg(style::TEXT_MUTED),
            ));
        }
    } else {
        spans.push(Span::styled("no model", Style::default().fg(style::TEXT_MUTED)));
    }

    // ── 4. Separator ──
    push_sep(&mut spans);

    // ── 5. Budget (used / total) ──
    let budget_used_k = state.ui.budget_used as f64 / 1000.0;
    let budget_total_k = state.ui.budget_total as f64 / 1000.0;
    spans.push(Span::styled(
        format!("{:0.1}K/{:0.1}K", budget_used_k, budget_total_k),
        Style::default().fg(style::TEXT_PRIMARY),
    ));

    // ── 6. Activity spinner ──
    if state.ui.active_chat_requests > 0 {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let phase = (state.ui.think_frame as usize / 2) % spinner.len();
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            spinner[phase],
            Style::default().fg(style::YELLOW).add_modifier(Modifier::BOLD),
        ));
    }

    // ── 7. Token metrics (from cached tokenizer values) ──
    let in_tokens = state.ui.cached_input_tokens;
    let out_tokens = state.ui.cached_output_tokens;
    if in_tokens > 0 || out_tokens > 0 {
        spans.push(Span::styled("  ", Style::default()));
        if in_tokens > 0 {
            let in_k = in_tokens as f64 / 1000.0;
            spans.push(Span::styled(
                format!("↑{:0.1}k", in_k),
                Style::default().fg(style::TEXT_SECONDARY),
            ));
        }
        if out_tokens > 0 {
            let out_k = out_tokens as f64 / 1000.0;
            let prefix = if in_tokens > 0 { " ↓" } else { "↓" };
            spans.push(Span::styled(
                format!("{}{:0.1}k", prefix, out_k),
                Style::default().fg(style::TEXT_SECONDARY),
            ));
        }
    }

    // ── 8. Tokenizer uninitialised warning ──
    if state.ui.active_chat_requests == 0 && !state.ui.tokenizer_initialized && (in_tokens > 0 || out_tokens > 0) {
        spans.push(Span::styled(" ⚠T", Style::default().fg(style::WARNING)));
    }

    // ── 9. Permits indicator (if constrained) ──
    let permits_used = state.ui.permits_total.saturating_sub(state.ui.permits_available);
    if permits_used > 0 {
        let fill_pct = if state.ui.permits_total > 0 {
            (permits_used as f64 / state.ui.permits_total as f64) * 100.0
        } else {
            0.0
        };
        // Only show if > 50% utilised
        if fill_pct > 50.0 {
            spans.push(Span::styled("  ", Style::default()));
            spans.push(Span::styled(
                format!("🧵{:0.0}%", fill_pct),
                Style::default().fg(style::WARNING),
            ));
        }
    }

    // ── Fill remaining space (use Unicode display width for CJK safety) ──
    let content_width: usize = spans.iter().map(|s| s.content.as_ref().width()).sum();
    let fill = (area.width as usize).saturating_sub(content_width + 1);
    if fill > 0 {
        spans.push(Span::raw(" ".repeat(fill)));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ═══════════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Push a " • " separator span.
fn push_sep<'a>(spans: &mut Vec<Span<'a>>) {
    spans.push(Span::styled(" • ", Style::default().fg(style::TEXT_MUTED)));
}

/// Format capability badges like `[T R V]`.
fn format_cap_badge(caps: &crate::models::ModelCapabilities) -> String {
    let mut parts = Vec::with_capacity(4);
    if caps.supports_tool_call {
        parts.push("T");
    }
    if caps.supports_reasoning {
        parts.push("R");
    }
    if caps.supports_vision {
        parts.push("V");
    }
    if caps.supports_attachment {
        parts.push("A");
    }
    if parts.is_empty() {
        return "[-]".to_string();
    }
    format!("[{}]", parts.join(" "))
}
