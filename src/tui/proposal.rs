//! Proposal / context panel — right-side panel showing the active plan,
//! task diff, and system status when no plan is loaded.
//!
//! Uses `{}`-style rounded borders (BorderType::Rounded) per the mockup.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use super::state::AppState;
use super::style;
use crate::agent::plan::{PlanStatus, TaskStatus};

/// Render the proposal / context panel on the right side of the screen.
pub(crate) fn render_proposal(f: &mut Frame, area: Rect, state: &AppState) {
    let core = &state.core;
    let ui = &state.ui;

    let (title, lines) = if let Some(plan) = &ui.current_plan {
        render_plan_panel(plan)
    } else {
        render_status_panel(core, ui)
    };

    let block = style::panel_proposal(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Build lines for the plan proposal panel.
fn render_plan_panel(plan: &crate::agent::plan::Plan) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    // ── Plan header (goal) ──
    let status_label = match plan.status {
        PlanStatus::Draft => "Draft",
        PlanStatus::Approved => "Approved",
        PlanStatus::Executing => "Executing",
        PlanStatus::Completed => "Completed",
        PlanStatus::Failed => "Failed",
    };
    let status_color = match plan.status {
        PlanStatus::Draft => style::WARNING,
        PlanStatus::Approved => style::SUCCESS,
        PlanStatus::Executing => style::WARNING,
        PlanStatus::Completed => style::SUCCESS,
        PlanStatus::Failed => style::ERROR,
    };
    lines.push(Line::from(vec![
        Span::styled("Goal  ", style::label_style()),
        Span::styled(plan.goal.clone(), style::value_style()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("State ", style::label_style()),
        Span::styled(status_label, Style::default().fg(status_color).bold()),
    ]));

    // ── Divider ──
    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Task list (diff-style) ──
    let completed = plan.tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
    let total = plan.tasks.len();
    lines.push(Line::from(vec![
        Span::styled(
            format!(" Tasks  {}/{}", completed, total),
            style::title_style(),
        ),
    ]));

    for task in &plan.tasks {
        let (prefix, color) = match task.status {
            TaskStatus::Completed => ("✓", style::SUCCESS),
            TaskStatus::Running => ("●", style::WARNING),
            TaskStatus::Failed => ("✗", style::ERROR),
            TaskStatus::Pending => (" ", style::INACTIVE),
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", prefix), Style::default().fg(color).bold()),
            Span::styled(
                if task.description.len() > 28 {
                    format!("{}…", &task.description[..27])
                } else {
                    task.description.clone()
                },
                Style::default().fg(if task.status == TaskStatus::Pending {
                    style::INACTIVE
                } else {
                    style::VALUE
                }),
            ),
        ]));
    }

    // ── Summary stats ──
    if total > 0 {
        lines.push(Line::from(Span::styled(
            "─".repeat(34),
            Style::default().fg(style::INACTIVE),
        )));
        let running = plan.tasks.iter().filter(|t| t.status == TaskStatus::Running).count();
        let failed = plan.tasks.iter().filter(|t| t.status == TaskStatus::Failed).count();
        let pending = plan.tasks.iter().filter(|t| t.status == TaskStatus::Pending).count();
        let mut stats = Vec::new();
        if pending > 0 {
            stats.push(Span::styled(format!("○ {} pending", pending), style::hint_style()));
        }
        if running > 0 {
            stats.push(Span::styled(format!("● {} running", running), Style::default().fg(style::WARNING)));
        }
        if failed > 0 {
            stats.push(Span::styled(format!("✗ {} failed", failed), Style::default().fg(style::ERROR)));
        }
        if !stats.is_empty() {
            lines.push(Line::from(stats));
        }
    }

    ("Proposal".to_string(), lines)
}

/// Build lines for the system status panel (fallback when no plan is active).
fn render_status_panel(core: &super::state::CoreState, ui: &super::state::UiState) -> (String, Vec<Line<'static>>) {
    let mut lines: Vec<Line> = Vec::new();

    // ── Agent info ──
    lines.push(Line::from(vec![
        Span::styled(" Agent", style::title_style()),
    ]));
    if let Some(agent_id) = core.responsible_agent_id {
        let short_id = format!(
            "{:02x}{:02x}{:02x}{:02x}",
            agent_id[0], agent_id[1], agent_id[2], agent_id[3]
        );
        lines.push(Line::from(vec![
            Span::styled("  id     ", style::label_style()),
            Span::styled(short_id, style::value_style()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  agents ", style::label_style()),
            Span::styled(format!("{} spawned", core.agents.len()), style::value_style()),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("  id     ", style::label_style()),
            Span::styled("—", style::hint_style()),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Network ──
    lines.push(Line::from(vec![
        Span::styled(" Network", style::title_style()),
    ]));
    let configured = core.configured_providers.len();
    lines.push(Line::from(vec![
        Span::styled("  config ", style::label_style()),
        Span::styled(
            format!("{} providers", configured),
            Style::default().fg(if configured > 0 { style::SUCCESS } else { style::INACTIVE }),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  stream ", style::label_style()),
        if ui.active_chat_requests > 0 {
            Span::styled("active", Style::default().fg(style::WARNING))
        } else {
            Span::styled("idle", style::hint_style())
        },
    ]));

    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Memory ──
    lines.push(Line::from(vec![
        Span::styled(" Memory", style::title_style()),
    ]));
    if let Some(runtime) = &core.runtime {
        if let Ok(rt) = runtime.try_read() {
            let exp_count = rt.experience_count();
            let bedrock = rt.bedrock_count();
            let fluid = rt.fluid_count();
            let pending = rt.pending_suspended();
            lines.push(Line::from(vec![
                Span::styled("  total  ", style::label_style()),
                Span::styled(format!("{}", exp_count), style::value_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  bedrock", style::label_style()),
                Span::styled(format!("{}", bedrock), style::value_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  fluid  ", style::label_style()),
                Span::styled(format!("{}", fluid), style::hint_style()),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  suspend", style::label_style()),
                Span::styled(format!("{}", pending), style::hint_style()),
            ]));
        }
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Budget ──
    lines.push(Line::from(vec![
        Span::styled(" Budget", style::title_style()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  used   ", style::label_style()),
        Span::styled(format!("{}/{}", ui.budget_used, ui.budget_total), style::value_style()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  permits", style::label_style()),
        Span::styled(
            format!("{}", ui.permits_available),
            Style::default().fg(if ui.permits_available > 0 {
                style::SUCCESS
            } else {
                style::ERROR
            }),
        ),
    ]));

    ("Status".to_string(), lines)
}
