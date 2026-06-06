use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::models::{Model, ModelRegistry, Provider};

#[derive(Clone, PartialEq)]
pub enum Focus {
    Sidebar,
    Chat,
    Input,
}

#[derive(Clone, PartialEq)]
pub enum Panel {
    Chat,
    Models,
}

pub struct AppState {
    pub panel: Panel,
    pub focus: Focus,
    pub agents: Vec<AgentEntry>,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_cursor: usize,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub budget_used: u64,
    pub budget_total: u64,
    pub permits_available: usize,
    pub permits_total: usize,
    pub models: ModelRegistry,
    pub model_scroll: usize,
    pub selected_model_idx: usize,
    pub search_query: String,
    pub search_cursor: usize,
    pub search_mode: bool,
    pub show_provider_dialog: bool,
    pub selected_provider_idx: usize,
    pub provider_search_query: String,
    pub provider_search_cursor: usize,
}

#[derive(Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub budget: u64,
}

#[derive(Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Suspended,
    Completed,
    Failed,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
}

#[derive(Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            panel: Panel::Chat,
            focus: Focus::Input,
            agents: vec![
                AgentEntry {
                    id: "agent-001".to_string(),
                    name: "auth-service".to_string(),
                    status: AgentStatus::Running,
                    budget: 500,
                },
                AgentEntry {
                    id: "agent-002".to_string(),
                    name: "db-migration".to_string(),
                    status: AgentStatus::Completed,
                    budget: 200,
                },
                AgentEntry {
                    id: "agent-003".to_string(),
                    name: "api-gateway".to_string(),
                    status: AgentStatus::Suspended,
                    budget: 300,
                },
            ],
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "Holographic Multi-Agent System v0.1.0".to_string(),
                    timestamp: "00:00:00".to_string(),
                },
                ChatMessage {
                    role: MessageRole::System,
                    content: "Type a task to spawn an agent".to_string(),
                    timestamp: "00:00:00".to_string(),
                },
            ],
            input: String::new(),
            input_cursor: 0,
            sidebar_scroll: 0,
            chat_scroll: 0,
            budget_used: 2000,
            budget_total: 10000,
            permits_available: 7,
            permits_total: 10,
            models: ModelRegistry::new(),
            model_scroll: 0,
            selected_model_idx: 0,
            search_query: String::new(),
            search_cursor: 0,
            search_mode: false,
            show_provider_dialog: false,
            selected_provider_idx: 0,
            provider_search_query: String::new(),
            provider_search_cursor: 0,
        }
    }
}

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    state: Arc<RwLock<AppState>>,
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
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal, state })
    }

    pub async fn run(&mut self) -> Result<()> {
        {
            let mut state = self.state.write().await;
            match state.models.fetch().await {
                Ok(_) => {
                    let count = state.models.providers().len();
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Loaded {} providers from models.dev", count),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    });
                }
                Err(e) => {
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Failed to load models: {}", e),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    });
                }
            }
        }

        let mut last_tick = Instant::now();
        let tick_rate = Duration::from_millis(100);

        loop {
            self.draw().await?;

            if event::poll(tick_rate.saturating_sub(last_tick.elapsed()))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                let mut state = self.state.write().await;

                // Global keys
                match key.code {
                    KeyCode::Char('c')
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }

                // Panel-specific keys
                match state.panel {
                    Panel::Chat => {
                        if !self.handle_chat_keys(&mut state, key.code) {
                            return Ok(());
                        }
                    }
                    Panel::Models => {
                        self.handle_models_keys(&mut state, key.code);
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                last_tick = Instant::now();
            }
        }
    }

    fn handle_chat_keys(&self, state: &mut AppState, code: KeyCode) -> bool {
        // Provider dialog
        if state.show_provider_dialog {
            let filtered: Vec<_> = state
                .models
                .providers()
                .iter()
                .filter(|p| {
                    state.provider_search_query.is_empty()
                        || p.name
                            .to_lowercase()
                            .contains(&state.provider_search_query.to_lowercase())
                })
                .collect();
            match code {
                KeyCode::Esc => {
                    state.show_provider_dialog = false;
                    state.provider_search_query.clear();
                    state.provider_search_cursor = 0;
                }
                KeyCode::Char('j') if !filtered.is_empty() => {
                    state.selected_provider_idx =
                        (state.selected_provider_idx + 1).min(filtered.len() - 1);
                }
                KeyCode::Char('k') => {
                    state.selected_provider_idx = state.selected_provider_idx.saturating_sub(1);
                }
                KeyCode::Enter => {
                    if let Some(provider) = filtered.get(state.selected_provider_idx) {
                        let pid = provider.id.clone();
                        let pname = provider.name.clone();
                        let penv = provider.env.join(", ");
                        state.models.select_provider(&pid);
                        let now = chrono::Local::now().format("%H:%M:%S").to_string();
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("Provider: {} (key: {})", pname, penv),
                            timestamp: now,
                        });
                    }
                    state.show_provider_dialog = false;
                    state.provider_search_query.clear();
                    state.provider_search_cursor = 0;
                }
                KeyCode::Backspace => {
                    if state.provider_search_cursor > 0 {
                        state.provider_search_cursor -= 1;
                        state.provider_search_query.remove(state.provider_search_cursor);
                        state.selected_provider_idx = 0;
                    }
                }
                KeyCode::Left => {
                    state.provider_search_cursor = state.provider_search_cursor.saturating_sub(1);
                }
                KeyCode::Right => {
                    if state.provider_search_cursor < state.provider_search_query.len() {
                        state.provider_search_cursor += 1;
                    }
                }
                KeyCode::Char(c) => {
                    state.provider_search_query.insert(state.provider_search_cursor, c);
                    state.provider_search_cursor += 1;
                    state.selected_provider_idx = 0;
                }
                _ => {}
            }
            return true;
        }
                    KeyCode::Enter => {
                        state.provider_search_mode = false;
                        state.selected_provider_idx = 0;
                    }
                    KeyCode::Char(c) => {
                        state
                            .provider_search_query
                            .insert(state.provider_search_cursor, c);
                        state.provider_search_cursor += 1;
                        state.selected_provider_idx = 0;
                    }
                    KeyCode::Backspace => {
                        if state.provider_search_cursor > 0 {
                            state.provider_search_cursor -= 1;
                            state
                                .provider_search_query
                                .remove(state.provider_search_cursor);
                            state.selected_provider_idx = 0;
                        }
                    }
                    KeyCode::Left => {
                        state.provider_search_cursor =
                            state.provider_search_cursor.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        if state.provider_search_cursor < state.provider_search_query.len() {
                            state.provider_search_cursor += 1;
                        }
                    }
                    _ => {}
                }
            } else {
                let providers: Vec<_> = state
                    .models
                    .providers()
                    .iter()
                    .filter(|p| {
                        state.provider_search_query.is_empty()
                            || p.name
                                .to_lowercase()
                                .contains(&state.provider_search_query.to_lowercase())
                    })
                    .collect();
                match code {
                    KeyCode::Esc => {
                        state.show_provider_dialog = false;
                        state.provider_search_query.clear();
                        state.provider_search_cursor = 0;
                        state.provider_search_mode = false;
                    }
                    KeyCode::Char('/') => {
                        state.provider_search_mode = true;
                        state.provider_search_cursor = state.provider_search_query.len();
                    }
                    KeyCode::Char('j') if !providers.is_empty() => {
                        state.selected_provider_idx =
                            (state.selected_provider_idx + 1).min(providers.len() - 1);
                    }
                    KeyCode::Char('k') => {
                        state.selected_provider_idx = state.selected_provider_idx.saturating_sub(1);
                    }
                    KeyCode::Enter => {
                        if let Some(provider) = providers.get(state.selected_provider_idx) {
                            let pid = provider.id.clone();
                            let pname = provider.name.clone();
                            let penv = provider.env.join(", ");
                            state.models.select_provider(&pid);
                            let now = chrono::Local::now().format("%H:%M:%S").to_string();
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!("Provider: {} (key: {})", pname, penv),
                                timestamp: now,
                            });
                        }
                        state.show_provider_dialog = false;
                        state.provider_search_query.clear();
                        state.provider_search_cursor = 0;
                        state.provider_search_mode = false;
                    }
                    _ => {}
                }
            }
            return true;
        }

        match code {
            KeyCode::Esc => {
                state.focus = Focus::Input;
                state.input.clear();
                state.input_cursor = 0;
            }
            KeyCode::Tab => {
                state.panel = Panel::Models;
                state.search_mode = false;
            }
            KeyCode::Char('1') => state.panel = Panel::Chat,
            KeyCode::Char('2') => state.panel = Panel::Models,
            KeyCode::Char('p') if !state.models.providers().is_empty() => {
                state.show_provider_dialog = true;
                state.selected_provider_idx = 0;
            }
            KeyCode::Char('j') if state.focus == Focus::Chat => {
                state.chat_scroll = state.chat_scroll.saturating_add(1);
            }
            KeyCode::Char('k') if state.focus == Focus::Chat => {
                state.chat_scroll = state.chat_scroll.saturating_sub(1);
            }
            KeyCode::Enter if state.focus == Focus::Input => {
                let input = state.input.clone();
                if !input.is_empty() {
                    let now = chrono::Local::now().format("%H:%M:%S").to_string();
                    state.messages.push(ChatMessage {
                        role: MessageRole::User,
                        content: input.clone(),
                        timestamp: now.clone(),
                    });

                    // Simulate pipeline
                    state.messages.push(ChatMessage {
                        role: MessageRole::Agent,
                        content: "→ L-1: Admission granted".to_string(),
                        timestamp: now.clone(),
                    });
                    state.messages.push(ChatMessage {
                        role: MessageRole::Agent,
                        content: "→ L0: Budget allocated, depth check passed".to_string(),
                        timestamp: now.clone(),
                    });
                    state.messages.push(ChatMessage {
                        role: MessageRole::Agent,
                        content: "→ L1: Experience match (confidence: 0.87)".to_string(),
                        timestamp: now.clone(),
                    });
                    state.messages.push(ChatMessage {
                        role: MessageRole::Decision,
                        content: "✓ Spawn APPROVED".to_string(),
                        timestamp: now,
                    });

                    let agent_id = format!("agent-{:03}", state.agents.len() + 1);
                    state.agents.push(AgentEntry {
                        id: agent_id,
                        name: input.chars().take(20).collect(),
                        status: AgentStatus::Running,
                        budget: 1000,
                    });

                    state.budget_used += 1000;
                    state.permits_available -= 1;
                    state.input.clear();
                    state.input_cursor = 0;
                    state.chat_scroll = 0;
                }
            }
            KeyCode::Char(c) if state.focus == Focus::Input => {
                let cursor = state.input_cursor;
                state.input.insert(cursor, c);
                state.input_cursor += 1;
            }
            KeyCode::Backspace if state.focus == Focus::Input => {
                if state.input_cursor > 0 {
                    state.input_cursor -= 1;
                    let cursor = state.input_cursor;
                    state.input.remove(cursor);
                }
            }
            KeyCode::Left if state.focus == Focus::Input => {
                state.input_cursor = state.input_cursor.saturating_sub(1);
            }
            KeyCode::Right if state.focus == Focus::Input => {
                if state.input_cursor < state.input.len() {
                    state.input_cursor += 1;
                }
            }
            _ => {}
        }
        true
    }

    fn handle_models_keys(&self, state: &mut AppState, code: KeyCode) {
        if state.search_mode {
            match code {
                KeyCode::Esc => {
                    state.search_mode = false;
                    state.search_query.clear();
                    state.search_cursor = 0;
                    state.selected_model_idx = 0;
                }
                KeyCode::Enter => {
                    state.search_mode = false;
                    state.selected_model_idx = 0;
                }
                KeyCode::Char(c) => {
                    state.search_query.insert(state.search_cursor, c);
                    state.search_cursor += 1;
                    state.selected_model_idx = 0;
                }
                KeyCode::Backspace => {
                    if state.search_cursor > 0 {
                        state.search_cursor -= 1;
                        state.search_query.remove(state.search_cursor);
                        state.selected_model_idx = 0;
                    }
                }
                KeyCode::Left => {
                    state.search_cursor = state.search_cursor.saturating_sub(1);
                }
                KeyCode::Right => {
                    if state.search_cursor < state.search_query.len() {
                        state.search_cursor += 1;
                    }
                }
                _ => {}
            }
            return;
        }

        let results = self.get_filtered_models(state);

        match code {
            KeyCode::Tab | KeyCode::Esc => {
                state.panel = Panel::Chat;
            }
            KeyCode::Char('/') => {
                state.search_mode = true;
                state.search_cursor = state.search_query.len();
            }
            KeyCode::Char('j') => {
                if !results.is_empty() {
                    state.selected_model_idx =
                        (state.selected_model_idx + 1).min(results.len() - 1);
                }
            }
            KeyCode::Char('k') => {
                state.selected_model_idx = state.selected_model_idx.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some((provider, model)) = results.get(state.selected_model_idx) {
                    let provider_id = provider.id.clone();
                    let model_id = model.id.clone();
                    let provider_name = provider.name.clone();
                    let model_name = model.name.clone();
                    state.models.select_provider(&provider_id);
                    state.models.select_model(&model_id);
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Selected: {} / {}", provider_name, model_name),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    });
                    state.panel = Panel::Chat;
                }
            }
            _ => {}
        }
    }

    fn get_filtered_models<'a>(&self, state: &'a AppState) -> Vec<(&'a Provider, &'a Model)> {
        if state.search_query.is_empty() {
            state
                .models
                .providers()
                .iter()
                .flat_map(|p| p.models.values().map(move |m| (p, m)))
                .collect()
        } else {
            state.models.search_models(&state.search_query)
        }
    }

    async fn draw(&mut self) -> Result<()> {
        let state = self.state.read().await;

        self.terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(1),
                ])
                .split(f.area());

            // Header
            Self::render_header(f, chunks[0], &state);

            // Main content
            match state.panel {
                Panel::Chat => Self::render_chat_panel(f, chunks[1], &state),
                Panel::Models => Self::render_models_panel(f, chunks[1], &state),
            }

            // Status bar
            Self::render_status_bar(f, chunks[2], &state);

            // Provider dialog overlay
            if state.show_provider_dialog {
                Self::render_provider_dialog(f, f.area(), &state);
            }
        })?;

        Ok(())
    }

    fn render_header(f: &mut Frame, area: Rect, state: &AppState) {
        let panel_indicator = match state.panel {
            Panel::Chat => Span::styled(
                " Chat ",
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Panel::Models => Span::styled(
                " Models ",
                Style::default()
                    .bg(Color::Cyan)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
        };

        let header = Line::from(vec![
            Span::styled(
                " workflow ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            panel_indicator,
            Span::raw("  "),
            Span::styled(
                format!("Agents: {}", state.agents.len()),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw("  "),
            Span::styled(
                format!("Budget: {}/{}", state.budget_used, state.budget_total),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("  "),
            Span::styled(
                format!("Permits: {}", state.permits_available),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!(
                    "Provider: {}",
                    state.models.selected_provider().unwrap_or("none")
                ),
                Style::default().fg(Color::Magenta),
            ),
        ]);

        f.render_widget(Paragraph::new(header), area);
    }

    fn render_chat_panel(f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(25), Constraint::Percentage(75)])
            .split(area);

        Self::render_sidebar(f, chunks[0], state);
        Self::render_chat(f, chunks[1], state);
    }

    fn render_models_panel(f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(8),
            ])
            .split(area);

        // Search bar
        Self::render_search_bar(f, chunks[0], state);

        // Model list
        Self::render_model_list(f, chunks[1], state);

        // Model details
        Self::render_model_details_panel(f, chunks[2], state);
    }

    fn render_sidebar(f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        // Stats
        let stats = vec![
            Line::from(vec![
                Span::styled("Agents: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", state.agents.len()),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("Messages: ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}", state.messages.len()),
                    Style::default().fg(Color::White),
                ),
            ]),
        ];

        f.render_widget(
            Paragraph::new(stats).block(Block::default().borders(Borders::ALL).title("System")),
            chunks[0],
        );

        // Agent list
        let agents: Vec<ListItem> = state
            .agents
            .iter()
            .enumerate()
            .skip(state.sidebar_scroll)
            .map(|(_, a)| {
                let icon = match a.status {
                    AgentStatus::Running => Span::styled("●", Style::default().fg(Color::Green)),
                    AgentStatus::Suspended => Span::styled("●", Style::default().fg(Color::Yellow)),
                    AgentStatus::Completed => Span::styled("●", Style::default().fg(Color::Blue)),
                    AgentStatus::Failed => Span::styled("●", Style::default().fg(Color::Red)),
                };
                ListItem::new(Line::from(vec![icon, Span::raw(" "), Span::raw(&a.name)]))
            })
            .collect();

        f.render_widget(
            List::new(agents).block(Block::default().borders(Borders::ALL).title("Agents")),
            chunks[1],
        );
    }

    fn render_chat(f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);

        // Messages
        let messages: Vec<ListItem> = state
            .messages
            .iter()
            .rev()
            .skip(state.chat_scroll)
            .take(chunks[0].height as usize - 2)
            .map(|m| {
                let (icon, color) = match m.role {
                    MessageRole::System => ("◆", Color::DarkGray),
                    MessageRole::User => ("❯", Color::Cyan),
                    MessageRole::Agent => ("→", Color::Blue),
                    MessageRole::Decision => ("✓", Color::Green),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(icon, Style::default().fg(color)),
                    Span::raw(" "),
                    Span::raw(&m.content),
                ]))
            })
            .collect();

        f.render_widget(
            List::new(messages).block(Block::default().borders(Borders::ALL).title("Pipeline")),
            chunks[0],
        );

        // Input
        let input_style = if state.focus == Focus::Input {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        f.render_widget(
            Paragraph::new(state.input.as_str())
                .style(input_style)
                .block(Block::default().borders(Borders::ALL).title("Task")),
            chunks[1],
        );

        if state.focus == Focus::Input {
            let cursor_x = chunks[1].x + state.input_cursor as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn render_search_bar(f: &mut Frame, area: Rect, state: &AppState) {
        let style = if state.search_mode {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };

        let text = if state.search_query.is_empty() && !state.search_mode {
            "/ to search...".to_string()
        } else {
            state.search_query.clone()
        };

        f.render_widget(
            Paragraph::new(text)
                .style(style)
                .block(Block::default().borders(Borders::ALL).title("Search")),
            area,
        );

        if state.search_mode {
            let cursor_x = area.x + state.search_cursor as u16 + 1;
            let cursor_y = area.y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn render_model_list(f: &mut Frame, area: Rect, state: &AppState) {
        let results = if state.search_query.is_empty() {
            state
                .models
                .providers()
                .iter()
                .flat_map(|p| p.models.values().map(move |m| (p, m)))
                .collect::<Vec<_>>()
        } else {
            state.models.search_models(&state.search_query)
        };

        let items: Vec<ListItem> = results
            .iter()
            .enumerate()
            .map(|(i, (p, m))| {
                let style = if i == state.selected_model_idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let flags = format!(
                    "{}{}",
                    if m.reasoning { "R" } else { "" },
                    if m.tool_call { "T" } else { "" }
                );
                let ctx = format!("{}k", m.limit.context / 1000);

                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("[{:2}]", flags),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(" "),
                    Span::styled(&p.name, Style::default().fg(Color::DarkGray)),
                    Span::raw("/"),
                    Span::styled(&m.name, style),
                    Span::raw("  "),
                    Span::styled(ctx, Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        let title = format!("Models ({})", results.len());

        f.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
            area,
        );
    }

    fn render_model_details_panel(f: &mut Frame, area: Rect, state: &AppState) {
        let results = if state.search_query.is_empty() {
            state
                .models
                .providers()
                .iter()
                .flat_map(|p| p.models.values().map(move |m| (p, m)))
                .collect::<Vec<_>>()
        } else {
            state.models.search_models(&state.search_query)
        };

        if let Some((provider, model)) = results.get(state.selected_model_idx) {
            let details = vec![
                Line::from(vec![
                    Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&provider.name),
                ]),
                Line::from(vec![
                    Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(&model.name),
                ]),
                Line::from(vec![
                    Span::styled("Context: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{} tokens", model.limit.context),
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Cost: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!(
                            "${:.2}/{:.2} per M tokens",
                            model.cost.input, model.cost.output
                        ),
                        Style::default().fg(Color::Green),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Features: ", Style::default().fg(Color::DarkGray)),
                    Span::raw(format!(
                        "{}{}{}",
                        if model.reasoning { "reasoning " } else { "" },
                        if model.tool_call { "tools " } else { "" },
                        if model.open_weights { "open" } else { "" }
                    )),
                ]),
            ];

            f.render_widget(
                Paragraph::new(details)
                    .block(Block::default().borders(Borders::ALL).title("Details")),
                area,
            );
        } else {
            f.render_widget(
                Paragraph::new("No model selected")
                    .block(Block::default().borders(Borders::ALL).title("Details")),
                area,
            );
        }
    }

    fn render_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
        let all_providers = state.models.providers();
        if all_providers.is_empty() {
            return;
        }

        let filtered: Vec<&Provider> = all_providers
            .iter()
            .filter(|p| {
                state.provider_search_query.is_empty()
                    || p.name
                        .to_lowercase()
                        .contains(&state.provider_search_query.to_lowercase())
            })
            .collect();

        if filtered.is_empty() {
            return;
        }

        let dialog_w = 44.min(area.width.saturating_sub(4));
        let list_h = filtered.len() as u16;
        let dialog_h = (list_h + 6).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(dialog_area);

        // Search bar
        let search_text = if state.provider_search_query.is_empty() {
            "type to filter...".to_string()
        } else {
            state.provider_search_query.clone()
        };
        f.render_widget(
            Paragraph::new(search_text)
                .style(Style::default().fg(Color::Cyan))
                .block(Block::default().borders(Borders::ALL).title("Search")),
            chunks[0],
        );
        let cursor_x = chunks[0].x + state.provider_search_cursor as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x, cursor_y));

        // Provider list
        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let style = if i == state.selected_provider_idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let count = p.models.len();
                let env = p.env.first().map(|e| e.as_str()).unwrap_or("no key");
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if i == state.selected_provider_idx {
                            "❯"
                        } else {
                            " "
                        },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::styled(&p.name, style),
                    Span::raw("  "),
                    Span::styled(
                        format!("{} models", count),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("env: {}", env), Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Provider ")
            .style(Style::default().fg(Color::Cyan));

        f.render_widget(List::new(items).block(block), chunks[1]);
    }

        let filtered: Vec<&Provider> = all_providers
            .iter()
            .filter(|p| {
                state.provider_search_query.is_empty()
                    || p.name
                        .to_lowercase()
                        .contains(&state.provider_search_query.to_lowercase())
            })
            .collect();

        if filtered.is_empty() {
            return;
        }

        let dialog_w = 44.min(area.width.saturating_sub(4));
        let list_h = filtered.len() as u16;
        let search_h = if state.provider_search_mode {
            3u16
        } else {
            0u16
        };
        let dialog_h = (list_h + 4 + search_h).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(search_h), Constraint::Min(0)].as_ref())
            .split(dialog_area);

        // Search bar
        if state.provider_search_mode {
            let search_style = Style::default().fg(Color::Cyan);
            f.render_widget(
                Paragraph::new(state.provider_search_query.as_str())
                    .style(search_style)
                    .block(Block::default().borders(Borders::ALL).title("Search")),
                chunks[0],
            );
            let cursor_x = chunks[0].x + state.provider_search_cursor as u16 + 1;
            let cursor_y = chunks[0].y + 1;
            f.set_cursor_position((cursor_x, cursor_y));
        }

        // Provider list
        let items: Vec<ListItem> = filtered
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let style = if i == state.selected_provider_idx {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let count = p.models.len();
                let env = p.env.first().map(|e| e.as_str()).unwrap_or("no key");
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if i == state.selected_provider_idx {
                            "❯"
                        } else {
                            " "
                        },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::styled(&p.name, style),
                    Span::raw("  "),
                    Span::styled(
                        format!("{} models", count),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(format!("env: {}", env), Style::default().fg(Color::Yellow)),
                ]))
            })
            .collect();

        let list_area = if state.provider_search_mode {
            chunks[1]
        } else {
            chunks[0]
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Provider ")
            .style(Style::default().fg(Color::Cyan));

        f.render_widget(List::new(items).block(block), list_area);
    }

    fn render_status_bar(f: &mut Frame, area: Rect, state: &AppState) {
        let hint = match state.panel {
            Panel::Chat => {
                if state.show_provider_dialog {
                    if state.provider_search_mode {
                        "Type to search | Enter: confirm | Esc: cancel"
                    } else {
                        "/: search | j/k: navigate | Enter: select | Esc: cancel"
                    }
                } else {
                    "p: provider | Tab: models | 1: chat | 2: models | Ctrl+C: quit"
                }
            }
            Panel::Models => {
                if state.search_mode {
                    "Type to search | Enter: confirm | Esc: cancel"
                } else {
                    "/: search | j/k: navigate | Enter: select | Tab/Esc: back"
                }
            }
        };

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ])),
            area,
        );
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}
