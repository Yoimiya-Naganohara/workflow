//! Sidebar panel — system status, runtime info, plan, experience, agents.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::AppState;

pub(crate) fn render_sidebar(f: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Status ")
        .style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // ── Agent ──
    if let Some(agent_id) = state.responsible_agent_id {
        lines.push(Line::from(Span::styled(
            " ── Agent ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        let short_id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            agent_id[0], agent_id[1], agent_id[2], agent_id[3]
        );
        lines.push(Line::from(vec![
            Span::styled("  id      ", Style::default().fg(Color::DarkGray)),
            Span::styled(short_id, Style::default().fg(Color::Cyan)),
        ]));
        let agent_status = state
            .agents
            .iter()
            .find(|a| a.id == crate::agent::AgentPool::agent_id_str(&agent_id))
            .map(|a| &a.status);
        let (status_label, status_color) = match agent_status {
            Some(super::state::AgentStatus::Running) => ("running", Color::Yellow),
            Some(super::state::AgentStatus::Completed) => ("completed", Color::Green),
            Some(super::state::AgentStatus::Failed) => ("failed", Color::Red),
            Some(super::state::AgentStatus::Suspended) => ("suspended", Color::DarkGray),
            None => ("idle", Color::DarkGray),
        };
        lines.push(Line::from(vec![
            Span::styled("  status  ", Style::default().fg(Color::DarkGray)),
            Span::styled(status_label, Style::default().fg(status_color)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  agents  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{} spawned", state.agents.len()),
                Style::default().fg(if state.agents.len() > 1 {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
        ]));
    }

    // ── Network ──
    lines.push(Line::from(Span::styled(
        " ── Network ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    let configured = state.configured_providers.len();
    lines.push(Line::from(vec![
        Span::styled("  config  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} providers", configured),
            Style::default().fg(if configured > 0 { Color::Green } else { Color::DarkGray }),
        ),
    ]));
    if state.active_chat_requests > 0 {
        lines.push(Line::from(vec![
            Span::styled("  stream  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "active",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
            ),
        ]));
    }

    // ── Experience ──
    lines.push(Line::from(Span::styled(
        " ── Memory ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    if let Some(runtime) = &state.runtime {
        if let Ok(rt) = runtime.try_read() {
            let exp_count = rt.experience_count();
            let bedrock = rt.bedrock_count();
            let fluid = rt.fluid_count();
            let pending = rt.pending_suspended();
            lines.push(Line::from(vec![
                Span::styled("  total   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", exp_count),
                    Style::default().fg(if exp_count > 0 { Color::Green } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  bedrock ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", bedrock),
                    Style::default().fg(if bedrock > 0 { Color::Green } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  fluid   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", fluid),
                    Style::default().fg(if fluid > 0 { Color::Yellow } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  suspend ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", pending),
                    Style::default().fg(if pending > 0 { Color::Yellow } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  history ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", state.input_history.len()),
                    Style::default().fg(Color::White),
                ),
            ]));
        }
    }

    // ── Budget ──
    lines.push(Line::from(Span::styled(
        " ── Budget ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("  used    ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/{}", state.budget_used, state.budget_total),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  permits ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.permits_available),
            Style::default().fg(if state.permits_available > 0 {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ]));

    f.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }), inner);
}
