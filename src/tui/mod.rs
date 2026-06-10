pub mod chat_lines;
pub mod controller;
pub mod dialogs;
pub mod handler;
pub mod keymap;
pub mod render;
pub mod sidebar;
pub mod state;
pub mod style;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, text::Line};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub use self::state::AppState;
use self::state::Panel;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
    chat_lines_cache: Vec<Line<'static>>,
    chat_cache_msg_count: usize,
    chat_cache_width: usize,
}

impl Tui {
    pub fn new(state: Arc<RwLock<AppState>>) -> Result<Self> {
        enable_raw_mode().map_err(|e| {
            anyhow::anyhow!(
                "Failed to enable raw mode: {}. Are you running in an interactive terminal?",
                e
            )
        })?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            state,
            chat_lines_cache: Vec::new(),
            chat_cache_msg_count: 0,
            chat_cache_width: 0,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            crate::tui::controller::load_initial_state(&mut state).await;
        }

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(100);

        loop {
            self.draw().await?;

            if event::poll(tick_rate.saturating_sub(last_tick.elapsed()))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        let mut state = self.state.write().await;

                        // Global keys
                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('c')
                        {
                            return Ok(());
                        }

                        // Panel-specific keys
                        match state.panel {
                            Panel::Chat => {
                                if !self.handle_chat_keys(&mut state, key) {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    Event::Mouse(mouse) => {
                        let mut state = self.state.write().await;
                        match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                if state.show_custom_dialog {
                                    // no scrolling in custom dialog (simple form)
                                } else if state.show_model_picker {
                                    let results = state.models.search_configured_models(&state.model_picker_search_query, &state.configured_providers);
                                    if !results.is_empty() {
                                        state.selected_model_picker_idx =
                                            (state.selected_model_picker_idx + 1).min(results.len() - 1);
                                    }
                                } else if state.show_provider_dialog {
                                    let providers = crate::models::filter_providers(
                                        state.models.providers(),
                                        &state.provider_search_query,
                                    );
                                    if !providers.is_empty() {
                                        state.selected_provider_idx =
                                            (state.selected_provider_idx + 1).min(providers.len() - 1);
                                    }
                                } else {
                                    state.chat_scroll = state.chat_scroll.saturating_add(1);
                                }
                            }
                            MouseEventKind::ScrollUp => {
                                if state.show_custom_dialog {
                                    // no scrolling in custom dialog (simple form)
                                } else if state.show_model_picker {
                                    state.selected_model_picker_idx = state.selected_model_picker_idx.saturating_sub(1);
                                } else if state.show_provider_dialog {
                                    state.selected_provider_idx = state.selected_provider_idx.saturating_sub(1);
                                } else {
                                    state.chat_scroll = state.chat_scroll.saturating_sub(1);
                                    state.auto_scroll = false;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            // Update tick
            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    }
}
