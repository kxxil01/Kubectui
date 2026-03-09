# Milestone 7: Clipboard & Data Export — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Let users get data out of the TUI (resource names, log content) via system clipboard and file export.

**Architecture:** A new `src/clipboard.rs` module provides OSC 52 terminal escape sequence support for clipboard writes — no external crates needed. A new `src/export.rs` module handles log buffer file export. Both are thin modules called from `main.rs` action handlers. Key bindings use `Ctrl+y` (copy resource name), `Y` (copy namespace/name), and `S` (save logs to file). Actions flow through the existing `AppAction` enum and `apply_action` dispatch.

**Tech Stack:** Rust, crossterm (stdout write), OSC 52 escape sequences, base64 encoding (std), tokio::fs for async file write

---

## Task 1: OSC 52 Clipboard Module

**Files:**
- Create: `src/clipboard.rs`
- Modify: `src/main.rs` (add `mod clipboard;`)

### Step 1: Create clipboard module

Create `src/clipboard.rs`:

```rust
//! System clipboard integration via OSC 52 terminal escape sequence.
//!
//! OSC 52 is supported by most modern terminals (iTerm2, Alacritty, kitty,
//! tmux 3.3+, WezTerm, foot, Windows Terminal). It writes to the system
//! clipboard without needing external crates or platform-specific code.

use std::io::Write;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

/// Copies `text` to the system clipboard via the OSC 52 escape sequence.
///
/// Returns `Ok(())` if the escape sequence was written to stdout.
/// The terminal is responsible for actually placing data on the clipboard.
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let encoded = BASE64.encode(text.as_bytes());
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{encoded}\x07")?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_encoding_works() {
        let encoded = BASE64.encode(b"hello");
        assert_eq!(encoded, "aGVsbG8=");
    }

    #[test]
    fn empty_string_does_not_panic() {
        let encoded = BASE64.encode(b"");
        assert_eq!(encoded, "");
    }
}
```

**WAIT** — this uses the `base64` crate. We need to add it to Cargo.toml, OR use the standard library's approach. Actually, Rust stdlib doesn't have base64. Let's use a simple inline base64 encoder to avoid adding a dependency, OR add the `base64` crate.

**Decision:** Add `base64` crate — it's small, well-maintained, and avoids reimplementing encoding.

### Step 2: Add base64 dependency

In `Cargo.toml`, add under `[dependencies]`:
```toml
base64 = "0.22"
```

### Step 3: Register the module

In `src/main.rs`, add near the top with other `mod` declarations:
```rust
mod clipboard;
```

### Step 4: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/clipboard.rs Cargo.toml src/main.rs
git commit -m "feat: add OSC 52 clipboard module"
```

---

## Task 2: Copy Resource Name Actions

**Files:**
- Modify: `src/app.rs` (add `CopyResourceName` and `CopyResourceFullName` to AppAction, add key bindings)
- Modify: `src/events/input.rs` (handle new actions)
- Modify: `src/main.rs` (call clipboard::copy_to_clipboard, set status_message)

### Step 1: Add action variants

In `src/app.rs`, add to `AppAction` enum (after `CloseHelp`):
```rust
CopyResourceName,
CopyResourceFullName,
```

### Step 2: Add key bindings

In `src/app.rs` `handle_key_event`, in the main key matching section (the section that handles global/content keys, around where `KeyCode::Char('y')` is already mapped to `OpenResourceYaml`):

Add `Ctrl+y` for copying resource name. This must be checked BEFORE the search input handler (which ignores Ctrl keys). Add in the content-focus keybinding area:

```rust
KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && self.detail_view.is_none() && self.focus == Focus::Content => {
    AppAction::CopyResourceName
}
KeyCode::Char('Y') if self.detail_view.is_none() && self.focus == Focus::Content => {
    AppAction::CopyResourceFullName
}
```

**IMPORTANT:** The `Y` binding conflicts with `OpenResourceYaml` which is bound to lowercase `y`. The existing `y` binding has a guard that also matches when `detail_view.is_none() && focus == Content`. We need `Y` (uppercase/shift) to copy full name. Check that the existing `y` binding only matches lowercase — crossterm sends `KeyCode::Char('y')` for lowercase and `KeyCode::Char('Y')` for Shift+y. They are distinct, so `Y` is safe to add.

Also add in the detail_view active section: when detail_view is open, `Ctrl+y` copies the detail resource name.
```rust
KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && self.detail_view.is_some() => {
    AppAction::CopyResourceName
}
```

### Step 3: Handle actions in events/input.rs

```rust
AppAction::CopyResourceName | AppAction::CopyResourceFullName => {
    // Handled in main.rs (needs cluster snapshot to resolve selected resource)
    true
}
```

### Step 4: Handle in main.rs event loop

In the main action match, add:

```rust
AppAction::CopyResourceName => {
    let name = app
        .detail_view
        .as_ref()
        .and_then(|d| d.resource.as_ref())
        .map(|r| r.name().to_string())
        .or_else(|| selected_resource(&app, &cached_snapshot).map(|r| r.name().to_string()));
    if let Some(name) = name {
        if let Err(e) = clipboard::copy_to_clipboard(&name) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied: {name}"));
        }
    }
}
AppAction::CopyResourceFullName => {
    let full = selected_resource(&app, &cached_snapshot).map(|r| {
        match r.namespace() {
            Some(ns) => format!("{ns}/{}", r.name()),
            None => r.name().to_string(),
        }
    });
    if let Some(full) = full {
        if let Err(e) = clipboard::copy_to_clipboard(&full) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied: {full}"));
        }
    }
}
```

### Step 5: Update help overlay

In `src/ui/components/help_overlay.rs`, add to the "Global" section:
```rust
("Ctrl+y", "Copy resource name"),
("Y", "Copy namespace/name"),
```

### Step 6: Tests

In `src/app.rs` tests:
```rust
#[test]
fn ctrl_y_returns_copy_resource_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
    assert_eq!(action, AppAction::CopyResourceName);
}

#[test]
fn shift_y_returns_copy_full_name() {
    let mut app = AppState::default();
    app.view = AppView::Pods;
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT));
    assert_eq!(action, AppAction::CopyResourceFullName);
}
```

### Step 7: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/app.rs src/events/input.rs src/main.rs src/ui/components/help_overlay.rs
git commit -m "feat: add Ctrl+y/Y to copy resource name to clipboard"
```

---

## Task 3: Copy Log Content to Clipboard

**Files:**
- Modify: `src/app.rs` (add `CopyLogContent` action, key binding in PodLogs/WorkloadLogs)
- Modify: `src/events/input.rs` (handle action)
- Modify: `src/main.rs` (join log lines, call clipboard)

### Step 1: Add action variant

In `src/app.rs` AppAction:
```rust
CopyLogContent,
```

### Step 2: Add key binding in PodLogs handler

In the `WorkbenchTabState::PodLogs(tab)` match arm, add before `_ =>`:
```rust
KeyCode::Char('y') if !tab.viewer.searching && !tab.viewer.picking_container => AppAction::CopyLogContent,
```

### Step 3: Add key binding in WorkloadLogs handler

In the `WorkbenchTabState::WorkloadLogs(tab)` match arm, add:
```rust
KeyCode::Char('y') if !tab.editing_text_filter => AppAction::CopyLogContent,
```

### Step 4: Handle in events/input.rs

```rust
AppAction::CopyLogContent => {
    // Handled in main.rs (needs log lines from workbench tab)
    true
}
```

### Step 5: Handle in main.rs

```rust
AppAction::CopyLogContent => {
    let content = app.workbench().active_tab().and_then(|tab| {
        match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                if logs_tab.viewer.lines.is_empty() {
                    None
                } else {
                    Some(logs_tab.viewer.lines.join("\n"))
                }
            }
            WorkbenchTabState::WorkloadLogs(wl_tab) => {
                if wl_tab.lines.is_empty() {
                    None
                } else {
                    Some(
                        wl_tab
                            .lines
                            .iter()
                            .map(|l| format!("{}:{} {}", l.pod_name, l.container_name, l.content))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    )
                }
            }
            _ => None,
        }
    });
    if let Some(content) = content {
        let line_count = content.lines().count();
        if let Err(e) = clipboard::copy_to_clipboard(&content) {
            app.set_error(format!("Clipboard error: {e}"));
        } else {
            app.status_message = Some(format!("Copied {line_count} log lines"));
        }
    }
}
```

### Step 6: Update help overlay

In the "Logs" section of `help_overlay.rs`:
```rust
("y", "Copy log content"),
```

### Step 7: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/app.rs src/events/input.rs src/main.rs src/ui/components/help_overlay.rs
git commit -m "feat: add y to copy log content to clipboard"
```

---

## Task 4: Log Export to File

**Files:**
- Create: `src/export.rs`
- Modify: `src/main.rs` (add `mod export;`, handle action)
- Modify: `src/app.rs` (add `ExportLogs` action, key binding)
- Modify: `src/events/input.rs` (handle action)
- Modify: `src/ui/components/workbench.rs` (add `[S] save` hint)

### Step 1: Create export module

Create `src/export.rs`:

```rust
//! Log export to local files.

use std::path::PathBuf;

/// Writes `content` to a log file and returns the path.
///
/// Default location: `/tmp/kubectui-logs-{label}-{timestamp}.log`
pub fn save_logs_to_file(label: &str, content: &str) -> std::io::Result<PathBuf> {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let safe_label: String = label
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let filename = format!("kubectui-logs-{safe_label}-{timestamp}.log");
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, content)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_logs_creates_file() {
        let path = save_logs_to_file("test-pod", "line 1\nline 2\n").unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line 1"));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn label_sanitization() {
        let path = save_logs_to_file("ns/pod:container", "data").unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
        std::fs::remove_file(path).ok();
    }
}
```

### Step 2: Register module

In `src/main.rs`:
```rust
mod export;
```

### Step 3: Add action variant

In `src/app.rs` AppAction:
```rust
ExportLogs,
```

### Step 4: Add key binding in PodLogs handler

In `WorkbenchTabState::PodLogs(tab)` match arm, add before `_ =>`:
```rust
KeyCode::Char('S') if !tab.viewer.searching && !tab.viewer.picking_container => AppAction::ExportLogs,
```

### Step 5: Add key binding in WorkloadLogs handler

```rust
KeyCode::Char('S') if !tab.editing_text_filter => AppAction::ExportLogs,
```

### Step 6: Handle in events/input.rs

```rust
AppAction::ExportLogs => {
    // Handled in main.rs (needs log buffer access)
    true
}
```

### Step 7: Handle in main.rs

```rust
AppAction::ExportLogs => {
    let export_data = app.workbench().active_tab().and_then(|tab| {
        match &tab.state {
            WorkbenchTabState::PodLogs(logs_tab) => {
                if logs_tab.viewer.lines.is_empty() {
                    None
                } else {
                    let label = format!(
                        "{}-{}",
                        logs_tab.viewer.pod_name,
                        logs_tab.viewer.container_name,
                    );
                    Some((label, logs_tab.viewer.lines.join("\n")))
                }
            }
            WorkbenchTabState::WorkloadLogs(wl_tab) => {
                if wl_tab.lines.is_empty() {
                    None
                } else {
                    let label = tab.state.title().replace(' ', "-");
                    let content = wl_tab
                        .lines
                        .iter()
                        .map(|l| format!("{}:{} {}", l.pod_name, l.container_name, l.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    Some((label, content))
                }
            }
            _ => None,
        }
    });
    if let Some((label, content)) = export_data {
        match export::save_logs_to_file(&label, &content) {
            Ok(path) => {
                app.status_message = Some(format!("Saved to {}", path.display()));
            }
            Err(e) => {
                app.set_error(format!("Export error: {e}"));
            }
        }
    }
}
```

### Step 8: Update workbench hints

In `src/ui/components/workbench.rs` `render_logs_tab`, update the keybind hints line to include `[S] save`:
```rust
"[Esc] back  [f] follow  [P] previous  [/] search  [n/N] next/prev  [t] timestamps  [y] copy  [S] save"
```

Also update help overlay "Logs" section:
```rust
("S", "Save logs to file"),
```

### Step 9: Verify and commit

```
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
git add src/export.rs src/main.rs src/app.rs src/events/input.rs src/ui/components/workbench.rs src/ui/components/help_overlay.rs
git commit -m "feat: add S to export log buffer to file"
```

---

## Quality Gate

After all 4 tasks, run:

```bash
cargo fmt --all
command cargo clippy --all-targets --all-features -- -D warnings
command cargo test --all-targets --all-features
cargo build --release
```

All must pass with zero warnings.

---

## Task Summary

| Task | Feature | Key Files | Complexity |
|------|---------|-----------|------------|
| 1 | OSC 52 clipboard module | clipboard.rs, Cargo.toml | Small |
| 2 | Copy resource name (Ctrl+y / Y) | app.rs, main.rs, input.rs | Medium |
| 3 | Copy log content (y in logs) | app.rs, main.rs, input.rs | Small |
| 4 | Log export to file (S) | export.rs, app.rs, main.rs, workbench.rs | Medium |

Tasks 1→2→3 are sequential (each builds on clipboard module). Task 4 is independent of 2-3 but depends on 1 being committed (for the pattern). In practice, execute sequentially: 1, 2, 3, 4.
