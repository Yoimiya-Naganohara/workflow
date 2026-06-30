//! Main UI layout — full-width chat, opencode-style status bar.
//!
//! When the root agent has active child delegations, the chat area
//! shrinks to make room for a diagnostic tree in the lower portion of
//! the screen.  The tree height adapts to content (3–12 lines, clamped
//! to at most ⅓ of the terminal height).

use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Style,
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph},
};

use super::{Tui, style};
use wf_agent::AgentStatus;
use crate::tui::agent_tree::{build_agent_tree_lines, has_active_delegations};
use crate::tui::chat::render_chat;
use crate::tui::chat_lines::build_chat_content;
use crate::tui::status::render_status_bar;

impl Tui {
    pub(crate) async fn draw(&mut self) -> Result<()> {
        let is_streaming = self.state.read().await.ui.active_chat_requests > 0;
        if is_streaming {
            let mut s = self.state.write().await;
            s.ui.think_frame = s.ui.think_frame.wrapping_add(1);
        }

        let state = self.state.read().await;
        let term_size = self.terminal.size()?;

        let msg_count = state.core.messages.len();
        let is_streaming = state.ui.active_chat_requests > 0;
        let last_content_len = state
            .core
            .messages
            .last()
            .map(|m| m.content.len())
            .unwrap_or(0);
        let max_input = (term_size.height / 3).max(3) as usize;
        let input_lines = state.ui.input.lines().count().clamp(1, max_input) as u16;
        let chat_width = term_size.width.saturating_sub(4).max(10) as usize;

        let cache_key = (
            msg_count,
            last_content_len,
            is_streaming,
            chat_width,
            is_streaming.then_some(state.ui.think_frame),
            state.ui.auto_scroll,
            if state.ui.auto_scroll {
                0
            } else {
                state.ui.chat_scroll
            },
        );

        // Rebuild cache when content has changed
        if cache_key != self.chat_cache_key {
            let new_content = build_chat_content(
                &state.core,
                chat_width,
                state.ui.think_frame,
                state.ui.think_level,
            );
            self.chat_lines_cache = new_content;
            self.chat_cache_key = cache_key;
        }

        // ── Diagnostic tree (Phase 1/3) ──
        let tree_lines = if let Some(rid) = state.core.responsible_agent_id {
            match state.core.agent_pool.try_read() {
                Ok(pool) => {
                    if has_active_delegations(&pool, &rid) {
                        build_agent_tree_lines(&pool, &rid)
                    } else {
                        Vec::new()
                    }
                }
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        let tree_item_count = tree_lines.len();
        let tree_height = if tree_item_count > 0 {
            let max_tree = (term_size.height as usize / 3).clamp(3, 12);
            (4 + tree_item_count).min(max_tree) as u16
        } else {
            0
        };
        let show_tree = tree_height > 0;

        // Compute actual visible height of the message area.
        // Layout: chat_area (with 2-pixel border) [+ tree_sep + tree] + status_bar.
        // Inside chat_area: msg_area [+ popup] + input.
        let pop_h: u16 = crate::tui::popup::popup_height(&state);
        let visible_height = (term_size
            .height
            .saturating_sub(input_lines + 3) // border(2) + status_bar(1)
            .saturating_sub(if show_tree { 1 + tree_height } else { 0 })
            .saturating_sub(pop_h))
        .max(1) as usize;

        let total = self.chat_lines_cache.total_lines();
        let max_scroll = self
            .chat_lines_cache
            .max_scroll_for_height(visible_height, chat_width);
        let reached_bottom = !state.ui.auto_scroll && state.ui.chat_scroll >= max_scroll;
        let chat_scroll = if state.ui.auto_scroll || reached_bottom {
            if reached_bottom {
                // User scrolled to the bottom — re-enable auto-scroll
                // so new streaming content follows naturally.
                // (Applied in the write section below.)
            }
            max_scroll
        } else {
            state.ui.chat_scroll.min(max_scroll)
        };

        // ── Snapshot embedding cache stats before dropping the read lock ──
        let embedding_cache = state
            .core
            .runtime
            .as_ref()
            .and_then(|rt| rt.try_read().ok())
            .map(|rt_guard| {
                let emb = rt_guard.embedding_service();
                crate::tui::state::CacheStats {
                    hits: emb.cache_hits(),
                    misses: emb.cache_misses(),
                }
            })
            .unwrap_or_default();

        // ── Batch all state writes after dropping the read lock ──
        drop(state);
        if let Ok(mut s) = self.state.try_write() {
            s.ui.tree_agent_ids = tree_lines.iter().map(|tl| tl.agent_id).collect();
            s.ui.selected_agent_idx =
                s.ui.selected_agent_idx
                    .min(tree_lines.len().saturating_sub(1));
            s.ui.total_chat_lines = total;
            s.ui.chat_scroll = chat_scroll;
            s.ui.auto_scroll = s.ui.auto_scroll || reached_bottom;
            s.ui.embedding_cache = embedding_cache;
        }
        let state = self.state.read().await;

        self.terminal.draw(|f| {
            let area = f.area();

            // ── Build vertical constraints ──
            let mut constraints = vec![Constraint::Min(0)]; // chat area
            if show_tree {
                constraints.push(Constraint::Length(1)); // separator
                constraints.push(Constraint::Length(tree_height)); // tree
            }
            constraints.push(Constraint::Length(1)); // status bar

            let vert_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(area);

            let chat_area = vert_chunks[0];
            let tree_sep_idx = if show_tree { 1 } else { 0 };
            let tree_idx = if show_tree { 2 } else { 0 };
            let status_idx = if show_tree { 3 } else { 1 };

            // ── Chat area ──
            let chat_border = crate::tui::style::panel_chat("");
            let chat_inner = chat_border.inner(chat_area);
            f.render_widget(chat_border, chat_area);

            // Pass the full output + scroll info to render_chat
            render_chat(
                f,
                chat_inner,
                &state,
                &self.chat_lines_cache,
                chat_scroll,
                visible_height,
            );

            // ── Diagnostic tree ──
            if show_tree {
                let separator = Paragraph::new("── Delegations ──")
                    .style(Style::default().fg(style::TEXT_MUTED));
                f.render_widget(separator, vert_chunks[tree_sep_idx]);

                let tree_items: Vec<ListItem> = tree_lines
                    .iter()
                    .map(|tl| {
                        let fg = match tl.status {
                            AgentStatus::AwaitingChildren | AgentStatus::Aggregating => {
                                style::YELLOW
                            }
                            AgentStatus::Failed => style::RED,
                            AgentStatus::Completed => style::GREEN,
                            _ => style::TEXT_PRIMARY,
                        };
                        ListItem::new(tl.display_text.as_str()).style(Style::default().fg(fg))
                    })
                    .collect();

                let tree_list = List::new(tree_items)
                    .block(
                        Block::default()
                            .borders(Borders::TOP)
                            .border_type(BorderType::Plain)
                            .border_style(Style::default().fg(style::TEXT_MUTED)),
                    )
                    .highlight_style(
                        Style::default()
                            .bg(style::BG_SECONDARY)
                            .add_modifier(ratatui::style::Modifier::BOLD),
                    );

                f.render_widget(tree_list, vert_chunks[tree_idx]);
            }

            // ── Status bar ──
            render_status_bar(f, vert_chunks[status_idx], &state);
        })?;

        Ok(())
    }
}
