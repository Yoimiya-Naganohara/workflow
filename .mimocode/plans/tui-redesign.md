# TUI Redesign Plan

## Goal
Redesign the TUI to be more modern and user-friendly, inspired by tools like opencode and Claude Code. Focus on:
- Cleaner visual hierarchy
- Better tool/agent call visualization
- Improved proposal panel
- More responsive status bar
- Better input experience

## Design Principles
1. **Minimal chrome** - Reduce visual noise, let content breathe
2. **Clear hierarchy** - User messages, agent responses, tool calls should be visually distinct
3. **Context-aware** - Show relevant info based on current state
4. **Keyboard-first** - Efficient keyboard navigation

## Changes

### 1. Chat Panel Redesign (`src/tui/chat.rs`, `src/tui/chat_lines.rs`)
- **User messages**: Left-aligned, subtle background, no heavy borders
- **Agent responses**: Clear but not overwhelming, proper markdown rendering
- **Tool calls**: Compact inline display with expandable details
  - Show tool name + key args inline
  - Expand on hover/select to show full args + result
  - Color-coded by tool type (read=green, write=orange, bash=blue)
- **Thinking indicators**: Subtle pulsing dots, not blocking

### 2. Proposal Panel Redesign (`src/tui/proposal.rs`)
- **Dynamic content**: Show current plan status, recent changes
- **Collapsible sections**: Budget, pool stats, active tasks
- **Visual indicators**: Progress bars, status icons
- **Compact by default**: Expand on demand

### 3. Status Bar Redesign (`src/tui/status.rs`)
- **Left**: Model name + context usage (e.g., "claude-sonnet-4 • 45% context")
- **Center**: Key metrics (tokens in/out, cost)
- **Right**: Quick actions (Ctrl+A, /, etc.)
- **Color coding**: Green=healthy, Yellow=warning, Red=critical

### 4. Input Area Redesign (`src/tui/chat.rs`)
- **Cleaner prompt**: Just `>` or `❯` without heavy borders
- **Multi-line support**: Better handling of long inputs
- **History navigation**: Visual history indicator
- **Command palette**: Ctrl+K for quick commands

### 5. Dialog Improvements
- **Provider picker**: Grid layout with status badges
- **Model picker**: Grouped by provider, with capability indicators
- **Key input**: Masked input with toggle visibility

### 6. Color Scheme Update (`src/tui/style.rs`)
- Adopt a more modern color palette (Catppuccin Mocha or similar)
- Consistent use of semantic colors (success, warning, error)
- Better contrast ratios for readability

### 7. Layout Improvements (`src/tui/render.rs`)
- **Responsive**: Adapt to terminal size
- **Panel resizing**: Allow resizing panels with mouse/keys
- **Focus indicators**: Clear which panel has focus

## Implementation Order
1. Style system update (colors, themes)
2. Chat message rendering improvements
3. Tool call visualization
4. Status bar redesign
5. Proposal panel cleanup
6. Input area polish
7. Dialog improvements
8. Layout/responsive fixes

## Files to Modify
- `src/tui/style.rs` - New color palette and style helpers
- `src/tui/chat_lines.rs` - Message rendering logic
- `src/tui/chat.rs` - Chat panel layout
- `src/tui/proposal.rs` - Proposal panel
- `src/tui/status.rs` - Status bar
- `src/tui/render.rs` - Main layout
- `src/tui/handler.rs` - Input handling
- `src/tui/popup.rs` - Popup rendering
- `src/tui/state.rs` - State management (if needed)

## Verification
1. Run `cargo check` to ensure compilation
2. Run `cargo test` to ensure no regressions
3. Manual testing in terminal to verify visual changes
