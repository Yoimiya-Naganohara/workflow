pub mod agent_tree;
pub mod chat;
pub mod chat_lines;
pub mod commands;
pub mod controller;
pub mod effect;
pub mod handler;
pub mod keymap;
pub mod popup;
pub mod render;
pub mod runtime_bridge;
pub mod state;
pub mod status;
pub mod style;
pub mod tokenizer;

use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, EventStream,
        KeyCode, KeyEventKind, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use self::state::Panel;
pub use self::state::{AppState, Focus};
use crate::tui::chat_lines::ChatRenderOutput;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
    chat_lines_cache: ChatRenderOutput,
    chat_cache_key: (usize, usize, bool, usize, Option<u8>, bool, usize),
    app_event_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::effect::AppEvent>,
    app_event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::tui::effect::AppEvent>,
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
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        let (app_event_tx, app_event_rx) = tokio::sync::mpsc::unbounded_channel();
        Ok(Self {
            terminal,
            state,
            chat_lines_cache: ChatRenderOutput {
                rendered: Vec::new(),
                tables: Vec::new(),
            },
            chat_cache_key: (0, 0, false, 0, None, true, 0),
            app_event_tx,
            app_event_rx,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            crate::tui::controller::load_initial_state(&mut state).await;
        }

        // ── Wire runtime event channel and spawn event loop + broker ──
        // Channel 1: tools → event loop (ActivateAgent)
        let (runtime_tx, event_loop_rx) = tokio::sync::mpsc::channel::<crate::runtime::RuntimeEvent>(256);
        {
            let mut s = self.state.write().await;
            s.core.runtime_event_tx = Some(runtime_tx);
        }
        // Channel 2: event loop → broker (ChildCompleted, AggregationCompleted, etc.)
        let (broker_tx, broker_rx) = tokio::sync::mpsc::channel::<crate::runtime::RuntimeEvent>(256);

        let state_clone = self.state.clone();
        let pool = state_clone.read().await.core.agent_pool.clone();
        let runtime = state_clone.read().await.core.runtime.clone();
        let tool_server = state_clone.read().await.core.tool_server.clone();
        let app_state = state_clone.clone();
        tokio::spawn(async move {
            let rt = runtime.expect("Runtime must be initialised before event loop");
            use crate::runtime::runtime_loop::RuntimeEventLoop;
            let loop_ = RuntimeEventLoop::new(rt, pool, event_loop_rx, broker_tx, tool_server, Some(app_state));
            loop_.run().await;
        });

        let app_tx = self.app_event_tx.clone();
        let state_for_broker = self.state.clone();
        tokio::spawn(async move {
            crate::tui::runtime_bridge::runtime_event_broker(broker_rx, app_tx, state_for_broker).await;
        });

        let mut event_stream = EventStream::new();
        let mut interval = tokio::time::interval(Duration::from_millis(50));

        loop {
            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            match event {
                                Event::Key(key) if key.kind == KeyEventKind::Press => {
                                    let mut state = self.state.write().await;

                                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                                        && key.code == KeyCode::Char('c')
                                    {
                                        return Ok(());
                                    }

                                    match state.ui.panel {
                                        Panel::Chat => {
                                            if !self.handle_chat_keys(&mut state, key) {
                                                self.save_context_on_shutdown().await;
                                                return Ok(());
                                            }
                                        }
                                    }

                                    let effects = std::mem::take(&mut state.effects);
                                    drop(state);
                                    for effect in effects {
                                        let tx = self.app_event_tx.clone();
                                        tokio::spawn(async move {
                                            crate::tui::effect::execute_effect(effect, &tx).await;
                                        });
                                    }
                                }
                                Event::Mouse(mouse) => {
                                    let mut state = self.state.write().await;
                                    match mouse.kind {
                                        MouseEventKind::ScrollDown => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_add(3);
                                        }
                                        MouseEventKind::ScrollUp => {
                                            state.ui.chat_scroll = state.ui.chat_scroll.saturating_sub(3);
                                            state.ui.auto_scroll = false;
                                        }
                                        _ => {}
                                    }
                                }
                                Event::Paste(text) => {
                                    let mut state = self.state.write().await;
                                    // 无弹窗 + 输入框焦点 → paste marker；其余情况都直接插入（弹窗过滤、KeyInput 等）
                                    if state.popup_mode == crate::tui::state::PopupMode::None
                                        && state.ui.focus == crate::tui::state::Focus::Input
                                    {
                                        state.ui.pending_paste = Some(text.clone());
                                        let summary = if text.lines().count() > 1 {
                                            format!("[Pasted {} chars / {} lines]", text.chars().count(), text.lines().count())
                                        } else {
                                            format!("[Pasted {} chars]", text.chars().count())
                                        };
                                        let byte_idx = crate::tui::chat_lines::char_idx_to_byte_idx(
                                            &state.ui.input,
                                            state.ui.input_cursor,
                                        );
                                        state.ui.input.insert_str(byte_idx, &summary);
                                        state.ui.input_cursor += summary.chars().count();
                                    } else {
                                        let byte_idx = crate::tui::chat_lines::char_idx_to_byte_idx(
                                            &state.ui.input,
                                            state.ui.input_cursor,
                                        );
                                        state.ui.input.insert_str(byte_idx, &text);
                                        state.ui.input_cursor += text.chars().count();
                                    }
                                }
                                _ => {}
                            }
                        }
                        Some(Err(e)) => { eprintln!("Event stream error: {}", e); }
                        None => {
                            self.save_context_on_shutdown().await;
                            return Ok(());
                        },
                    }
                }

                Some(app_event) = self.app_event_rx.recv() => {
                    let mut state = self.state.write().await;
                    state.handle_event(app_event);
                    let effects = std::mem::take(&mut state.effects);
                    drop(state);
                    for effect in effects {
                        let tx = self.app_event_tx.clone();
                        tokio::spawn(async move {
                            crate::tui::effect::execute_effect(effect, &tx).await;
                        });
                    }
                }

                _ = interval.tick() => {}
            }

            self.draw().await?;
        }
    }

    /// Save conversation context for the next session.
    async fn save_context_on_shutdown(&self) {
        let state = self.state.read().await;
        if let Some(agent_id) = state.core.responsible_agent_id {
            if let Ok(pool) = state.core.agent_pool.try_read() {
                if let Some(agent) = pool.get_agent(&agent_id) {
                    if !agent.context.is_empty() {
                        let ctx = crate::persistence::ContextSave {
                            agent_id: agent.id,
                            role: agent.role.clone(),
                            context: agent.context.clone(),
                            saved_at: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        };
                        let _ = crate::persistence::save_context(&ctx);
                    }
                }
            }
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // ── Graceful shutdown: consolidate fluid experiences to bedrock ──
        // This ensures all accumulated experiences (even those below the
        // high-water mark) are preserved to disk before the process exits.
        if let Ok(state) = self.state.try_read() {
            if let Some(runtime) = &state.core.runtime {
                if let Ok(mut rt) = runtime.try_write() {
                    rt.consolidate_experience_pool();
                    let _ = rt.flush_experience_pool();
                }
            }
        }

        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste,
        );
    }
}
