# Plan: Remove Dialog System, Use Inline Popups

## Goal
Remove all modal dialogs (Provider, Key, ModelPicker, CustomWizard, RoleWizard).
Everything uses inline popups like command autocomplete — appears above input box.

## Architecture Change

### Before
- `ActiveDialog` enum → modal overlays block chat
- Each dialog has own render/handle_key/scroll
- Main input synced with dialog search fields

### After
- No dialog system at all
- `popup.rs` handles ALL popups (commands, providers, key, models)
- Main input IS the search field for everything
- Popups render above input box, non-blocking

## Files to Modify

| File | Change |
|------|--------|
| `src/tui/mod.rs` | Remove dialog dispatch, route all keys through handler |
| `src/tui/handler.rs` | Handle all popup navigation directly |
| `src/tui/popup.rs` | Add provider/key/model rendering |
| `src/tui/state.rs` | Replace `ActiveDialog` with `PopupMode` enum |
| `src/tui/render.rs` | Remove overlay dialog rendering |
| `src/tui/chat.rs` | Remove dialog cursor handling |
| `src/tui/dialogs/` | **Delete entire directory** |

## New State

```rust
pub enum PopupMode {
    None,
    Commands,
    Providers,
    KeyInput { provider_id: String },
    ModelPicker,
}
```

## New Popup Rendering

```rust
// popup.rs handles ALL popups
fn render_popup(f, area, state) {
    match state.popup_mode {
        PopupMode::Commands => render_command_popup(...),
        PopupMode::Providers => render_provider_popup(...),
        PopupMode::KeyInput { .. } => render_key_popup(...),
        PopupMode::ModelPicker => render_model_popup(...),
        PopupMode::None => {},
    }
}
```

## Keyboard Flow

```
Input → handler resolves key
  ├─ if popup active: navigate popup (↑↓ Enter Esc)
  └─ if no popup: normal input handling
```

## Verification
1. `cargo check`
2. `cargo test`
3. Manual testing of each popup type
