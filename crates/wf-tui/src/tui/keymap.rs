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
    SendMessage,
    TypeChar(char),
    DeleteChar,
    HistoryPrev,
    HistoryNext,

    // Global actions
    OpenProviderPicker,
    RestoreContext,
    OpenCommandPicker,

    /// Open the agent detail popup for the selected tree item.
    InspectAgent,

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
        if (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT)
            && let KeyCode::Char(c) = key.code
        {
            return Action::TypeChar(c);
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
        km.bind(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Action::Quit,
        );
        km.bind(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL),
            Action::CancelResponse,
        );

        // Navigation
        km.bind(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            Action::MoveUp,
        );
        km.bind(
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            Action::MoveDown,
        );
        km.bind(
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            Action::MoveLeft,
        );
        km.bind(
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            Action::MoveRight,
        );

        // Dialog
        // Note: Enter is intentionally NOT bound here — dialogs handle Enter
        // in their own handle_key().  The keymap-level Confirm would shadow
        // SendMessage (bound below) because resolve() returns the first match.
        km.bind(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            Action::Cancel,
        );
        km.bind(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            Action::DeleteChar,
        );

        // Chat input
        km.bind(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            Action::SendMessage,
        );

        // Agent tree
        km.bind(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL),
            Action::InspectAgent,
        );

        // Command palette
        km.bind(
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
            Action::OpenCommandPicker,
        );

        km
    }
}

/// Format an `Action` as a human-readable description string.
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

        Action::SendMessage => "Send message",
        Action::TypeChar(_) => "Type character",
        Action::DeleteChar => "Delete character",
        Action::HistoryPrev => "Previous input history",
        Action::HistoryNext => "Next input history",
        Action::OpenProviderPicker => "Open provider picker",
        Action::RestoreContext => "Restore previous context",
        Action::OpenCommandPicker => "Open command picker",
        Action::InspectAgent => "Inspect agent detail",

        Action::None => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_quit() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn test_default_arrow_keys() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Action::MoveUp
        );
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Action::MoveDown
        );
    }

    #[test]
    fn test_default_enter_sends() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Action::SendMessage
        );
    }

    #[test]
    fn test_default_esc_cancels() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Action::Cancel
        );
    }

    #[test]
    fn test_unbound_key_returns_none() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            Action::None
        );
    }

    #[test]
    fn test_char_input_fallback() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            Action::TypeChar('a')
        );
    }

    #[test]
    fn test_shift_char_input() {
        let km = Keymap::default();
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Action::TypeChar('A')
        );
    }

    #[test]
    fn test_ctrl_c_does_not_fallback() {
        let km = Keymap::default();
        // Ctrl+C is Quit, not TypeChar('c')
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Action::Quit
        );
        // plain 'c' is TypeChar
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE)),
            Action::TypeChar('c')
        );
    }

    #[test]
    fn test_format_ctrl_c() {
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            "Ctrl+c"
        );
    }

    #[test]
    fn test_format_enter() {
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            "Enter"
        );
    }

    #[test]
    fn test_format_esc() {
        assert_eq!(
            format_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            "Esc"
        );
    }

    #[test]
    fn test_format_f1() {
        let result = format_key(&KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE));
        // KeyCode::F(1) formats as "F(1)" via Debug
        assert!(
            result.contains("F"),
            "expected F key in format, got: {}",
            result
        );
    }

    #[test]
    fn test_action_quit() {
        assert_eq!(format_action(&Action::Quit), "Quit the application");
    }

    #[test]
    fn test_action_none() {
        assert_eq!(format_action(&Action::None), "");
    }

    #[test]
    fn test_bindings_count() {
        let km = Keymap::default();
        assert!(km.all_bindings().len() >= 10);
    }

    #[test]
    fn test_custom_bind() {
        let mut km = Keymap::new();
        km.bind(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            Action::Quit,
        );
        assert_eq!(
            km.resolve(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Quit
        );
    }
}
