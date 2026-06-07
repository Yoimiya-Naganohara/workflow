//! Sidebar panel — system status, runtime info, plan, experience, agents.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::state::{AppMode, AppState};
use crate::agent::plan::PlanStatus;

pub(crate) fn render_sidebar(f: &mut Frame, area: Rect, state: &AppState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Status ")
        .style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // ── System ──
    lines.push(Line::from(Span::styled(
        " ── System ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    let mode_str = match state.mode {
        AppMode::Plan => "plan",
        AppMode::Build => "build",
    };
    lines.push(Line::from(vec![
        Span::styled("  mode    ", Style::default().fg(Color::DarkGray)),
        Span::styled(mode_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ]));
    let focus_str = match state.focus {
        super::state::Focus::Sidebar => "sidebar",
        super::state::Focus::Chat => "chat",
        super::state::Focus::Input => "input",
    };
    lines.push(Line::from(vec![
        Span::styled("  focus   ", Style::default().fg(Color::DarkGray)),
        Span::styled(focus_str, Style::default().fg(Color::White)),
    ]));
    let panel_str = match state.panel {
        super::state::Panel::Chat => "chat",
    };
    lines.push(Line::from(vec![
        Span::styled("  panel   ", Style::default().fg(Color::DarkGray)),
        Span::styled(panel_str, Style::default().fg(Color::White)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  budget  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/{}", state.budget_used, state.budget_total),
            Style::default().fg(Color::White),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  avail   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.permits_available),
            Style::default().fg(if state.permits_available > 0 {
                Color::Green
            } else {
                Color::Red
            }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  agents  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", state.agents.len()),
            Style::default().fg(if !state.agents.is_empty() {
                Color::Green
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    // ── Network ──
    lines.push(Line::from(Span::styled(
        " ── Network ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    let configured = state.configured_providers.len();
    let clients = state.provider_clients.len();
    lines.push(Line::from(vec![
        Span::styled("  config  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} providers", configured),
            Style::default().fg(if configured > 0 { Color::Green } else { Color::DarkGray }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  clients ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} active", clients),
            Style::default().fg(if clients > 0 { Color::Cyan } else { Color::DarkGray }),
        ),
    ]));
    if state.active_chat_requests > 0 {
        lines.push(Line::from(vec![
            Span::styled("  request ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "streaming",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
            ),
        ]));
    }

    // ── Plan ──
    if let Some(plan) = &state.current_plan {
        lines.push(Line::from(Span::styled(
            " ── Plan ──",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        )));
        let total = plan.tasks.len();
        let completed = plan
            .tasks
            .iter()
            .filter(|t| t.status == crate::agent::plan::TaskStatus::Completed)
            .count();
        let running = plan
            .tasks
            .iter()
            .filter(|t| t.status == crate::agent::plan::TaskStatus::Running)
            .count();
        let pending = plan
            .tasks
            .iter()
            .filter(|t| t.status == crate::agent::plan::TaskStatus::Pending)
            .count();
        let failed = plan
            .tasks
            .iter()
            .filter(|t| t.status == crate::agent::plan::TaskStatus::Failed)
            .count();

        let plan_color = if failed > 0 {
            Color::Red
        } else if running > 0 {
            Color::Yellow
        } else if completed == total && total > 0 {
            Color::Green
        } else {
            Color::White
        };
        lines.push(Line::from(vec![
            Span::styled("  status  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                match plan.status {
                    PlanStatus::Draft => "draft",
                    PlanStatus::Approved => "approved",
                    PlanStatus::Executing => "executing",
                    PlanStatus::Completed => "completed",
                    PlanStatus::Failed => "failed",
                },
                Style::default().fg(plan_color),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  tasks   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}/{}", completed, total),
                Style::default().fg(if completed == total { Color::Green } else { Color::White }),
            ),
        ]));
        if running > 0 {
            lines.push(Line::from(vec![
                Span::styled("  running ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", running), Style::default().fg(Color::Yellow)),
            ]));
        }
        if pending > 0 {
            lines.push(Line::from(vec![
                Span::styled("  pending ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", pending), Style::default().fg(Color::DarkGray)),
            ]));
        }
        if failed > 0 {
            lines.push(Line::from(vec![
                Span::styled("  failed  ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", failed), Style::default().fg(Color::Red)),
            ]));
        }
        if !plan.goal.is_empty() {
            let goal = if plan.goal.len() > (area.width as usize).saturating_sub(6) {
                format!("{}…", &plan.goal[..(area.width as usize).saturating_sub(8)])
            } else {
                plan.goal.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("  goal    ", Style::default().fg(Color::DarkGray)),
                Span::styled(goal, Style::default().fg(Color::White)),
            ]));
        }
    }

    // ── Experience ──
    lines.push(Line::from(Span::styled(
        " ── Experience ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("  history ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} entries", state.input_history.len()),
            Style::default().fg(Color::White),
        ),
    ]));
    if let Some(runtime) = &state.runtime {
        if let Ok(rt) = runtime.try_read() {
            let exp_count = rt.experience_count();
            let bedrock = rt.bedrock_count();
            let fluid = rt.fluid_count();
            let pending = rt.pending_suspended();
            lines.push(Line::from(vec![
                Span::styled("  total   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} entries", exp_count),
                    Style::default().fg(if exp_count > 0 { Color::Green } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  bedrock ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} entries", bedrock),
                    Style::default().fg(if bedrock > 0 { Color::Green } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  fluid   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} entries", fluid),
                    Style::default().fg(if fluid > 0 { Color::Yellow } else { Color::DarkGray }),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  suspend ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} queued", pending),
                    Style::default().fg(if pending > 0 { Color::Yellow } else { Color::DarkGray }),
                ),
            ]));
        }
    }

    // ── Runtime ──
    lines.push(Line::from(Span::styled(
        " ── Runtime ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("  request ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("#{}", state.active_chat_request_id),
            Style::default().fg(Color::White),
        ),
    ]));
    if let Some(plan) = &state.current_plan {
        let plan_status = match plan.status {
            PlanStatus::Draft => "draft",
            PlanStatus::Approved => "approved",
            PlanStatus::Executing => "executing",
            PlanStatus::Completed => "completed",
            PlanStatus::Failed => "failed",
        };
        lines.push(Line::from(vec![
            Span::styled("  plan    ", Style::default().fg(Color::DarkGray)),
            Span::styled(plan_status, Style::default().fg(Color::Cyan)),
        ]));
    }
    let agents_running = state
        .agents
        .iter()
        .filter(|a| a.status == super::state::AgentStatus::Running)
        .count();
    let agents_total = state.agents.len();
    lines.push(Line::from(vec![
        Span::styled("  agents  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}/{}", agents_running, agents_total),
            Style::default().fg(if agents_running > 0 {
                Color::Green
            } else {
                Color::DarkGray
            }),
        ),
    ]));

    f.render_widget(Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }), inner);
}
