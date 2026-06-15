use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    // Global
    Quit,
    CancelResponse,

    // Navigation
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    ScrollUp,
    ScrollDown,
    ScrollToTop,
    ScrollToBottom,

    // Dialog
    Confirm,
    Cancel,
    SwitchPanel,

    // Input
    TabComplete,
    SendMessage,
    TypeChar(char),
    DeleteChar,
    HistoryPrev,
    HistoryNext,
    CommandPrev,
    CommandNext,

    // Global actions
    OpenProviderPicker,
    OpenCommandPicker,

    /// Open the agent detail popup for the selected tree item.
    InspectAgent,
    /// Restore conversation context from the previous session.
    RestoreContext,

    // No match
    None,
}

pub struct Keymap {
    global: Vec<(KeyEvent, Action)>,
}

impl Keymap {
    pub fn new() -> Self {
        Self { global: Vec::new() }
    }

    pub fn bind(&mut self, key: KeyEvent, action: Action) {
        self.global.push((key, action));
    }

    pub fn resolve(&self, key: KeyEvent) -> Action {
        // First check for exact modifier+key match
        for (k, action) in &self.global {
            if k.code == key.code && k.modifiers == key.modifiers {
                return action.clone();
            }
        }
        // Fallback: if a non-modified key is typed and no exact match, it's a char input
        if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT {
            if let KeyCode::Char(c) = key.code {
                return Action::TypeChar(c);
            }
        }
        Action::None
    }

    pub fn all_bindings(&self) -> Vec<(String, Action)> {
        let mut result = Vec::new();
        for (key, action) in &self.global {
            let key_str = format_key(key);
            result.push((key_str, action.clone()));
        }
        result
    }
}

impl Default for Keymap {
    fn default() -> Self {
        let mut km = Self::new();

        // Global
        km.bind(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Quit);
        km.bind(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            Action::CancelResponse,
        );
        km.bind(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), Action::TabComplete);

        // Navigation
        km.bind(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), Action::MoveUp);
        km.bind(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), Action::MoveDown);
        km.bind(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), Action::MoveLeft);
        km.bind(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE), Action::MoveRight);

        // Dialog
        // Note: Enter is intentionally NOT bound here — dialogs handle Enter
        // in their own handle_key().  The keymap-level Confirm would shadow
        // SendMessage (bound below) because resolve() returns the first match.
        km.bind(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), Action::Cancel);
        km.bind(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            Action::DeleteChar,
        );

        // Chat input
        km.bind(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), Action::SendMessage);

        // Agent tree
        km.bind(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL),
            Action::InspectAgent,
        );
        km.bind(
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Action::RestoreContext,
        );

        km
    }
}

fn format_key(key: &KeyEvent) -> String {
    let mut s = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        s.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        s.push_str("Shift+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        s.push_str("Alt+");
    }
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                s.push(c.to_ascii_lowercase());
            } else {
                s.push(c);
            }
        }
        KeyCode::Enter => s.push_str("Enter"),
        KeyCode::Esc => s.push_str("Esc"),
        KeyCode::Tab => s.push_str("Tab"),
        KeyCode::Backspace => s.push_str("Backspace"),
        KeyCode::Up => s.push_str("Up"),
        KeyCode::Down => s.push_str("Down"),
        KeyCode::Left => s.push_str("Left"),
        KeyCode::Right => s.push_str("Right"),
        _ => s.push_str(&format!("{:?}", key.code)),
    }
    s
}

/// Format an `Action` as a human-readable description string.
pub fn format_action(action: &Action) -> String {
    match action {
        Action::Quit => "Quit the application",
        Action::CancelResponse => "Cancel current response",
        Action::MoveUp => "Move up / Previous item",
        Action::MoveDown => "Move down / Next item",
        Action::MoveLeft => "Move cursor left",
        Action::MoveRight => "Move cursor right",
        Action::ScrollUp => "Scroll chat up",
        Action::ScrollDown => "Scroll chat down",
        Action::ScrollToTop => "Scroll to top",
        Action::ScrollToBottom => "Scroll to bottom",
        Action::Confirm => "Confirm selection",
        Action::Cancel => "Cancel / Close dialog",
        Action::SwitchPanel => "Switch panel",
        Action::TabComplete => "Complete command / Toggle sidebar",
        Action::SendMessage => "Send message",
        Action::TypeChar(_) => "Type character",
        Action::DeleteChar => "Delete character",
        Action::HistoryPrev => "Previous input history",
        Action::HistoryNext => "Next input history",
        Action::CommandPrev => "Previous command",
        Action::CommandNext => "Next command",
        Action::OpenProviderPicker => "Open provider picker",
        Action::OpenCommandPicker => "Open command picker",
        Action::InspectAgent => "Inspect agent detail",
        Action::RestoreContext => "Restore previous context",
        Action::None => "",
    }
    .to_string()
}
