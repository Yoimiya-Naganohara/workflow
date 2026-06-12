//! Proposal / context panel — right-side panel showing the active plan,
//! task diff, and system status when no plan is loaded.
//!
//! Uses `{}`-style rounded borders (BorderType::Rounded) per the mockup.
//!
//! Matches the HTML preview design:
//! ```text
//! ╭─ Proposal ───────────────────────╮
//! │ # Proposal                        │
//! │ # 1aab5f8                         │
//! │ - abc                             │
//! │ + abcd                            │
//! │ Status                            │
//! │   Budget    72%                   │
//! │   Pool      1,284                 │
//! │   Depth     L2                    │
//! │   Model     gpt-4                 │
//! │ Plan                              │
//! │   ▸ pool compaction  done         │
//! │   ▸ export/import    in prog      │
//! │   ▸ flush timer                   │
//! │   ▸ compaction CLI                │
//! ╰───────────────────────────────────╯
//! ```

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span, Text},
    widgets::Paragraph,
};

use super::state::AppState;
use super::style;
use crate::agent::plan::TaskStatus;

/// Render the proposal / context panel on the right side of the screen.
pub(crate) fn render_proposal(f: &mut Frame, area: Rect, state: &AppState) {
    let core = &state.core;
    let ui = &state.ui;

    let lines = build_proposal_lines(core, ui);

    let block = style::panel_proposal("Proposal");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(Text::from(lines)), inner);
}

/// Build the proposal panel lines matching the HTML preview.
fn build_proposal_lines(
    core: &super::state::CoreState,
    ui: &super::state::UiState,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    // ── Recent commit / diff ──
    lines.push(Line::from(vec![
        Span::styled(" # ", Style::default().fg(style::INACTIVE)),
        Span::styled("Proposal", style::title_style()),
    ]));

    // Show the latest git hash if available
    let commit_hash = get_latest_commit_hash();
    lines.push(Line::from(vec![
        Span::styled(" # ", Style::default().fg(style::INACTIVE)),
        Span::styled(
            commit_hash,
            Style::default().fg(style::VALUE),
        ),
    ]));

    if let Some(diff) = get_recent_diff() {
        for d in &diff {
            let color = if d.starts_with('+') {
                style::SUCCESS
            } else if d.starts_with('-') {
                style::ERROR
            } else {
                style::HINT
            };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(d.clone(), Style::default().fg(color)),
            ]));
        }
    }

    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Status section ──
    lines.push(Line::from(vec![
        Span::styled(" Status", style::title_style()),
    ]));

    // Budget
    lines.push(Line::from(vec![
        Span::styled("   Budget", style::label_style()),
        Span::raw("    "),
        Span::styled(
            format!("{}%", if ui.budget_total > 0 {
                ui.budget_used * 100 / ui.budget_total
            } else { 0 }),
            style::value_style(),
        ),
    ]));

    // Pool (experience count from runtime)
    let pool_count = if let Some(runtime) = &core.runtime {
        if let Ok(rt) = runtime.try_read() {
            format!("{}", rt.experience_count())
        } else {
            "—".to_string()
        }
    } else {
        "—".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("   Pool", style::label_style()),
        Span::raw("      "),
        Span::styled(pool_count, style::value_style()),
    ]));

    // Depth (L level based on state)
    let depth_label = get_depth_label();
    lines.push(Line::from(vec![
        Span::styled("   Depth", style::label_style()),
        Span::raw("     "),
        Span::styled(depth_label, style::value_style()),
    ]));

    // Model (selected model name)
    let model_name = if let Some(sel) = core.selected_models.first() {
        sel.model_id.split('/').last().unwrap_or(&sel.model_id).to_string()
    } else {
        "—".to_string()
    };
    lines.push(Line::from(vec![
        Span::styled("   Model", style::label_style()),
        Span::raw("     "),
        Span::styled(model_name, style::value_style()),
    ]));

    lines.push(Line::from(Span::styled(
        "─".repeat(34),
        Style::default().fg(style::INACTIVE),
    )));

    // ── Plan section ──
    if let Some(plan) = &ui.current_plan {
        lines.push(Line::from(vec![
            Span::styled(" Plan", style::title_style()),
        ]));
        for task in &plan.tasks {
            let (prefix, bullet_color, item_style, tag) = match task.status {
                TaskStatus::Completed => (
                    "▸",
                    style::SUCCESS,
                    Style::default().fg(style::INACTIVE).crossed_out(),
                    Some(("done", style::SUCCESS)),
                ),
                TaskStatus::Running => (
                    "▸",
                    style::ACTIVE,
                    Style::default().fg(style::VALUE),
                    Some(("in prog", style::ACTIVE)),
                ),
                TaskStatus::Failed => (
                    "▸",
                    style::ERROR,
                    Style::default().fg(style::VALUE),
                    None,
                ),
                TaskStatus::Pending => (
                    "▸",
                    style::INACTIVE,
                    Style::default().fg(style::INACTIVE),
                    None,
                ),
            };
            let desc = if task.description.len() > 20 {
                format!("{}…", &task.description[..19])
            } else {
                task.description.clone()
            };

            let mut row = vec![
                Span::styled(format!(" {} ", prefix), Style::default().fg(bullet_color)),
                Span::styled(desc, item_style),
            ];
            if let Some((tag_text, tag_color)) = tag {
                // Right-align the tag with padding
                row.push(Span::raw(" ".repeat(6)));
                row.push(Span::styled(tag_text, Style::default().fg(tag_color)));
            }
            lines.push(Line::from(row));
        }
    } else {
        // Show plan placeholder like the HTML preview
        lines.push(Line::from(vec![
            Span::styled(" Plan", style::title_style()),
        ]));
        let plan_items = get_plan_items();
        for item in plan_items {
            let mut row = vec![
                Span::styled(" ▸ ", Style::default().fg(style::INACTIVE)),
                Span::styled(item.label, Style::default().fg(style::INACTIVE)),
            ];
            if item.active {
                row.push(Span::raw(" ".repeat(6)));
                row.push(Span::styled(
                    "in prog",
                    Style::default().fg(style::ACTIVE),
                ));
            }
            if item.done {
                row.push(Span::raw(" ".repeat(6)));
                row.push(Span::styled(
                    "done",
                    Style::default().fg(style::SUCCESS),
                ));
            }
            lines.push(Line::from(row));
        }
    }

    lines
}

/// Get the latest git commit short hash.
fn get_latest_commit_hash() -> String {
    // Try to read from git HEAD
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
    {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !hash.is_empty() {
                return hash;
            }
        }
    }
    "—".to_string()
}

/// Get recent diff lines (staged or unstaged).
fn get_recent_diff() -> Option<Vec<String>> {
    // Try to get a short diff stat
    if let Ok(output) = std::process::Command::new("git")
        .args(["diff", "--stat"])
        .output()
    {
        if output.status.success() {
            let stat = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !stat.is_empty() {
                return Some(
                    stat.lines()
                        .filter(|l| !l.is_empty())
                        .map(|l| format!(" {}", l))
                        .collect(),
                );
            }
        }
    }
    None
}

/// Get the depth label (L level).
fn get_depth_label() -> String {
    "L2".to_string()
}

/// Placeholder plan items when no real plan exists (matching HTML preview).
struct PlanItem {
    label: String,
    done: bool,
    active: bool,
}

fn get_plan_items() -> Vec<PlanItem> {
    vec![
        PlanItem {
            label: "pool compaction".to_string(),
            done: true,
            active: false,
        },
        PlanItem {
            label: "export/import".to_string(),
            done: false,
            active: true,
        },
        PlanItem {
            label: "flush timer".to_string(),
            done: false,
            active: false,
        },
        PlanItem {
            label: "compaction CLI".to_string(),
            done: false,
            active: false,
        },
    ]
}
