//! Sidebar panel — system status, runtime info, plan, experience, agents.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use super::state::AppState;
use super::style;

/// Section header label.
fn section_label(text: &str) -> Span<'static> {
    Span::styled(
        format!(" {} ", text),
        Style::default().fg(style::ACTIVE).add_modifier(Modifier::BOLD),
    )
}

/// Key-value pair: label (dark gray) + value (colored).
fn kv(label: &str, value: Span<'static>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<9}", label), style::label_style()),
        value,
    ])
}

/// Numeric value with optional color based on threshold.
fn num_val(n: usize, positive: bool) -> Span<'static> {
    let color = if n > 0 && positive {
        style::SUCCESS
    } else if n > 0 {
        style::WARNING
    } else {
        style::INACTIVE
    };
    Span::styled(format!("{}", n), Style::default().fg(color))
}

pub(crate) fn render_sidebar(f: &mut Frame, area: Rect, state: &AppState) {
    let block = style::panel("Status");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let core = &state.core;
    let ui = &state.ui;

    let mut lines: Vec<Line> = Vec::new();

    // ── Agent ──
    lines.push(Line::from(section_label("Agent")));
    if let Some(agent_id) = core.responsible_agent_id {
        let short_id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            agent_id[0], agent_id[1], agent_id[2], agent_id[3]
        );
        lines.push(kv("id", Span::styled(short_id, style::value_style())));

        let agent_status = core
            .agents
            .iter()
            .find(|a| a.id == crate::agent::AgentPool::agent_id_str(&agent_id))
            .map(|a| &a.status);
        let (status_label, status_color) = match agent_status {
            Some(super::state::AgentStatus::Running) => ("running", style::WARNING),
            Some(super::state::AgentStatus::Completed) => ("completed", style::SUCCESS),
            Some(super::state::AgentStatus::Failed) => ("failed", style::ERROR),
            Some(super::state::AgentStatus::Suspended) => ("suspended", style::INACTIVE),
            None => ("idle", style::INACTIVE),
        };
        lines.push(kv(
            "status",
            Span::styled(status_label, Style::default().fg(status_color)),
        ));
        lines.push(kv(
            "agents",
            Span::styled(format!("{} spawned", core.agents.len()), style::value_style()),
        ));
    } else {
        lines.push(kv("id", Span::styled("—", style::hint_style())));
        lines.push(kv("agents", Span::styled("0", style::hint_style())));
    }

    // ── Network ──
    lines.push(Line::from(section_label("Network")));
    let configured = core.configured_providers.len();
    lines.push(kv(
        "config",
        Span::styled(
            format!("{} providers", configured),
            Style::default().fg(if configured > 0 {
                style::SUCCESS
            } else {
                style::INACTIVE
            }),
        ),
    ));
    lines.push(kv(
        "stream",
        if ui.active_chat_requests > 0 {
            Span::styled(
                "active",
                Style::default().fg(style::WARNING).add_modifier(Modifier::SLOW_BLINK),
            )
        } else {
            Span::styled("idle", style::hint_style())
        },
    ));

    // ── Memory ──
    lines.push(Line::from(section_label("Memory")));
    if let Some(runtime) = &core.runtime {
        if let Ok(rt) = runtime.try_read() {
            let exp_count = rt.experience_count();
            let bedrock = rt.bedrock_count();
            let fluid = rt.fluid_count();
            let pending = rt.pending_suspended();
            lines.push(kv("total", num_val(exp_count, true)));
            lines.push(kv("bedrock", num_val(bedrock, true)));
            lines.push(kv("fluid", num_val(fluid, false)));
            lines.push(kv("suspend", num_val(pending, false)));
            lines.push(kv("history", num_val(ui.input_history.len(), true)));
        }
    }

    // ── Budget ──
    lines.push(Line::from(section_label("Budget")));
    lines.push(kv(
        "used",
        Span::styled(format!("{}/{}", ui.budget_used, ui.budget_total), style::value_style()),
    ));
    lines.push(kv(
        "permits",
        Span::styled(
            format!("{}", ui.permits_available),
            Style::default().fg(if ui.permits_available > 0 {
                style::SUCCESS
            } else {
                style::ERROR
            }),
        ),
    ));

    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}
