# Milestone 6: Discoverability & Operator QoL — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the most impactful UX gaps that Lens/Freelens users expect — help overlay, previous logs, log search, timestamps, sidebar counts, Pod IP column.

**Architecture:** Each feature is independent and can ship individually. All features follow existing patterns: overlays use the modal pattern (like namespace_picker), log features extend stream_logs parameters and LogsViewerState, sidebar counts thread through the existing cached_sidebar_lines pipeline, Pod IP adds a column to the existing pods table.

**Tech Stack:** Rust, ratatui, kube-rs (LogParams), crossterm (KeyEvent), tokio (async log streaming)

---

## Task 1: Help Overlay — `?` key shows keybinding reference

**Files:**
- Create: `src/ui/components/help_overlay.rs`
- Modify: `src/ui/components/mod.rs` (add `pub mod help_overlay;` export)
- Modify: `src/app.rs` (add `HelpOverlay` field, `AppAction::OpenHelp`/`CloseHelp`, key routing)
- Modify: `src/events/input.rs` (handle `OpenHelp`/`CloseHelp` actions)
- Modify: `src/ui/mod.rs` (render overlay in overlay section)

### Step 1: Create HelpOverlay component

Create `src/ui/components/help_overlay.rs` with the full component:

```rust
//! Keybinding help overlay displayed with `?`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::components::default_theme;

#[derive(Debug, Clone, Default)]
pub struct HelpOverlay {
    is_open: bool,
    scroll: usize,
}

const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Global",
        &[
            ("?", "Toggle this help"),
            ("q", "Quit (with confirmation)"),
            ("Esc", "Back / close overlay"),
            ("Tab / Shift+Tab", "Next / previous view"),
            ("j / k / ↓ / ↑", "Navigate list"),
            ("Enter", "Open detail / activate"),
            ("/", "Search / filter"),
            ("~", "Namespace picker"),
            ("c", "Context picker"),
            (":", "Command palette"),
            ("r", "Refresh data"),
            ("T", "Cycle theme"),
            ("b", "Toggle workbench"),
            (", / .", "Previous / next workbench tab"),
            ("Ctrl+W", "Close workbench tab"),
            ("Ctrl+↑ / Ctrl+↓", "Resize workbench"),
        ],
    ),
    (
        "Detail View",
        &[
            ("y", "View YAML"),
            ("v", "View events"),
            ("l", "View logs"),
            ("x", "Exec into pod"),
            ("f", "Port forward"),
            ("s", "Scale replicas"),
            ("p", "Probe panel"),
            ("R", "Restart rollout"),
            ("e", "Edit YAML"),
            ("d", "Delete resource"),
        ],
    ),
    (
        "Sort (Pods)",
        &[
            ("n", "Sort by name"),
            ("a / 1", "Sort by age"),
            ("2", "Sort by status"),
            ("3", "Sort by restarts"),
            ("0", "Clear sort"),
        ],
    ),
    (
        "Workbench (focused)",
        &[
            ("z", "Maximize / restore"),
            ("j / k", "Scroll down / up"),
            ("g / G", "Jump to top / bottom"),
            ("PageDown / PageUp", "Scroll by page"),
            ("Esc", "Un-maximize or blur"),
        ],
    ),
    (
        "Logs",
        &[
            ("f", "Toggle follow mode"),
            ("P", "Toggle previous logs"),
            ("t", "Toggle timestamps"),
            ("/", "Search in logs"),
            ("n / N", "Next / previous match"),
        ],
    ),
    (
        "Workload Logs",
        &[
            ("f", "Toggle follow mode"),
            ("p", "Cycle pod filter"),
            ("c", "Cycle container filter"),
            ("/", "Text filter"),
        ],
    ),
];

impl HelpOverlay {
    pub fn open(&mut self) {
        self.is_open = true;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn toggle(&mut self) {
        if self.is_open {
            self.close();
        } else {
            self.open();
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn total_lines() -> usize {
        let mut count = 0;
        for (_, bindings) in SECTIONS {
            count += 1; // section header
            count += bindings.len();
            count += 1; // blank line
        }
        count
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let theme = default_theme();

        let popup_width = 60u16.min(area.width.saturating_sub(4));
        let popup_height = 30u16.min(area.height.saturating_sub(4));
        let popup = centered_rect(popup_width, popup_height, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .title(Span::styled(
                " Keybindings ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme.border_style())
            .style(Style::default().bg(theme.bg_surface));

        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let mut lines: Vec<Line> = Vec::new();
        for (section_name, bindings) in SECTIONS {
            lines.push(Line::from(Span::styled(
                format!("  {section_name}"),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for (key, desc) in *bindings {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {key:<24}"), Style::default().fg(theme.fg)),
                    Span::styled(*desc, Style::default().fg(theme.fg_dim)),
                ]));
            }
            lines.push(Line::from(""));
        }

        let visible_height = sections[0].height as usize;
        let max_scroll = lines.len().saturating_sub(visible_height);
        let scroll = self.scroll.min(max_scroll);
        let end = (scroll + visible_height).min(lines.len());
        let visible = if scroll < end {
            lines[scroll..end].to_vec()
        } else {
            vec![]
        };

        frame.render_widget(
            Paragraph::new(visible).wrap(Wrap { trim: false }),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " [?/Esc] close  [j/k] scroll ",
                Style::default().fg(theme.fg_dim),
            ))),
            sections[1],
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
```

### Step 2: Export the module

In `src/ui/components/mod.rs`, add after existing module declarations:

```rust
pub mod help_overlay;
```

### Step 3: Add HelpOverlay field and actions to AppState

In `src/app.rs`:

1. Add to `AppAction` enum:
   ```rust
   OpenHelp,
   CloseHelp,
   ```

2. Add field to `AppState`:
   ```rust
   pub help_overlay: crate::ui::components::help_overlay::HelpOverlay,
   ```

3. Add `?` key routing — insert BEFORE existing overlay checks in `handle_key_event`:
   ```rust
   if self.help_overlay.is_open() {
       return match key.code {
           KeyCode::Esc | KeyCode::Char('?') => AppAction::CloseHelp,
           KeyCode::Char('j') | KeyCode::Down => {
               self.help_overlay.scroll_down();
               AppAction::None
           }
           KeyCode::Char('k') | KeyCode::Up => {
               self.help_overlay.scroll_up();
               AppAction::None
           }
           _ => AppAction::None,
       };
   }
   ```

4. Add `?` binding in main keybindings:
   ```rust
   KeyCode::Char('?') => AppAction::OpenHelp,
   ```

### Step 4: Handle actions in events/input.rs

Add to the `apply_action` match:
```rust
AppAction::OpenHelp => {
    app_state.help_overlay.toggle();
    true
}
AppAction::CloseHelp => {
    app_state.help_overlay.close();
    true
}
```

### Step 5: Render overlay in ui/mod.rs

Add after the `confirm_quit` rendering block (after line 753):
```rust
if app.help_overlay.is_open() {
    let _help_scope = profiling::span_scope("overlay.help");
    app.help_overlay.render(frame, frame.area());
}
```

### Step 6: Tests

Add tests in `src/ui/components/help_overlay.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_overlay_toggle() {
        let mut overlay = HelpOverlay::default();
        assert!(!overlay.is_open());
        overlay.toggle();
        assert!(overlay.is_open());
        overlay.toggle();
        assert!(!overlay.is_open());
    }

    #[test]
    fn help_overlay_scroll() {
        let mut overlay = HelpOverlay::default();
        overlay.open();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_down();
        assert_eq!(overlay.scroll, 1);
        overlay.scroll_up();
        assert_eq!(overlay.scroll, 0);
        overlay.scroll_up(); // should not underflow
        assert_eq!(overlay.scroll, 0);
    }

    #[test]
    fn total_lines_is_nonzero() {
        assert!(HelpOverlay::total_lines() > 20);
    }
}
```

### Step 7: Verify and commit

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git add src/ui/components/help_overlay.rs src/ui/components/mod.rs src/app.rs src/events/input.rs src/ui/mod.rs
git commit -m "feat: add ? help overlay with keybinding reference"
```

---

## Task 2: Previous Logs Toggle — `P` key in pod logs

**Files:**
- Modify: `src/app.rs` (add `previous_logs` field to LogsViewerState, `AppAction::LogsViewerTogglePrevious`)
- Modify: `src/events/input.rs` (handle new action)
- Modify: `src/coordinator/logs.rs` (add `previous` parameter to stream_logs)
- Modify: `src/main.rs` (pass previous flag when starting log streams, handle re-fetch)
- Modify: `src/ui/components/workbench.rs` (show "previous" indicator in logs status bar)

### Step 1: Add previous field to LogsViewerState

In `src/app.rs`, add to `LogsViewerState`:
```rust
pub previous_logs: bool,
```

Initialize to `false` in `Default` impl.

### Step 2: Add action variant

In `src/app.rs`, add to `AppAction`:
```rust
LogsViewerTogglePrevious,
```

### Step 3: Add `P` key binding in pod logs handler

In `handle_workbench_key_event`, PodLogs match arm, add:
```rust
KeyCode::Char('P') => AppAction::LogsViewerTogglePrevious,
```

### Step 4: Add previous parameter to stream_logs

In `src/coordinator/logs.rs`, update `stream_logs` and `stream_logs_internal` signatures:
```rust
pub async fn stream_logs(
    client: Arc<K8sClient>,
    pod_ref: PodRef,
    container_name: String,
    follow: bool,
    previous: bool,   // <-- new
    update_tx: mpsc::Sender<UpdateMessage>,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
)
```

In `stream_logs_internal`, update LogParams:
```rust
let params = LogParams {
    container: Some(container_name.to_string()),
    follow: if previous { false } else { follow },  // previous logs can't follow
    previous,
    tail_lines: if follow && !previous { Some(100) } else { Some(500) },
    timestamps: false,
    ..Default::default()
};
```

### Step 5: Handle action in events/input.rs

```rust
AppAction::LogsViewerTogglePrevious => {
    // Handled in main.rs (needs async log re-fetch)
    true
}
```

### Step 6: Handle in main.rs event loop

When `LogsViewerTogglePrevious` is received:
1. Toggle `previous_logs` on the active PodLogs tab's viewer
2. Clear existing lines
3. Cancel current log stream
4. Start new stream with updated `previous` flag

### Step 7: Update workbench renderer

In `src/ui/components/workbench.rs` `render_logs_tab`, update status line to show "previous" when active:
```rust
let status = if viewer.loading {
    "loading"
} else if viewer.previous_logs {
    "previous"
} else if viewer.picking_container {
    "select container"
} else if viewer.follow_mode {
    "following"
} else {
    "paused"
};
```

Add `[P] previous` to the keybind hints.

### Step 8: Update all stream_logs call sites in main.rs

Add `false` (or the viewer's `previous_logs` value) as the `previous` argument to every existing `stream_logs` call.

### Step 9: Tests

```rust
#[test]
fn logs_viewer_previous_toggle() {
    let mut viewer = LogsViewerState::default();
    assert!(!viewer.previous_logs);
    viewer.previous_logs = true;
    assert!(viewer.previous_logs);
}
```

### Step 10: Verify and commit

```
cargo fmt --all && cargo clippy && cargo test
git add src/app.rs src/events/input.rs src/coordinator/logs.rs src/main.rs src/ui/components/workbench.rs
git commit -m "feat: add P key to toggle previous logs for crashed containers"
```

---

## Task 3: Log Search and Highlight — `/` in pod logs, `n`/`N` for next/prev

**Files:**
- Modify: `src/app.rs` (add search fields to LogsViewerState, new actions)
- Modify: `src/events/input.rs` (handle new actions)
- Modify: `src/ui/components/workbench.rs` (render search input and highlighted matches)

### Step 1: Add search fields to LogsViewerState

In `src/app.rs`, add to `LogsViewerState`:
```rust
pub search_query: String,
pub search_input: String,
pub searching: bool,
```

### Step 2: Add action variants

```rust
LogsViewerSearchOpen,
LogsViewerSearchClose,
LogsViewerSearchNext,
LogsViewerSearchPrev,
```

### Step 3: Add key bindings in PodLogs handler

```rust
KeyCode::Char('/') if !tab.viewer.searching => AppAction::LogsViewerSearchOpen,
KeyCode::Esc if tab.viewer.searching => AppAction::LogsViewerSearchClose,
KeyCode::Enter if tab.viewer.searching => AppAction::LogsViewerSearchClose,  // apply and close
KeyCode::Char('n') if !tab.viewer.searching => AppAction::LogsViewerSearchNext,
KeyCode::Char('N') if !tab.viewer.searching => AppAction::LogsViewerSearchPrev,
KeyCode::Backspace if tab.viewer.searching => {
    tab.viewer.search_input.pop();
    AppAction::None
}
KeyCode::Char(c) if tab.viewer.searching => {
    tab.viewer.search_input.push(c);
    AppAction::None
}
```

### Step 4: Handle actions in events/input.rs

```rust
AppAction::LogsViewerSearchOpen => {
    if let Some(tab) = app_state.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
    {
        logs_tab.viewer.searching = true;
        logs_tab.viewer.search_input = logs_tab.viewer.search_query.clone();
        return true;
    }
    false
}
AppAction::LogsViewerSearchClose => {
    if let Some(tab) = app_state.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
    {
        logs_tab.viewer.search_query = logs_tab.viewer.search_input.clone();
        logs_tab.viewer.searching = false;
        return true;
    }
    false
}
AppAction::LogsViewerSearchNext => {
    if let Some(tab) = app_state.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
        && !logs_tab.viewer.search_query.is_empty()
    {
        let query = logs_tab.viewer.search_query.to_ascii_lowercase();
        let start = logs_tab.viewer.scroll_offset + 1;
        if let Some(pos) = logs_tab.viewer.lines.iter().skip(start)
            .position(|l| l.to_ascii_lowercase().contains(&query))
        {
            logs_tab.viewer.scroll_offset = start + pos;
            logs_tab.viewer.follow_mode = false;
        }
        return true;
    }
    false
}
AppAction::LogsViewerSearchPrev => {
    if let Some(tab) = app_state.workbench_mut().active_tab_mut()
        && let WorkbenchTabState::PodLogs(logs_tab) = &mut tab.state
        && !logs_tab.viewer.search_query.is_empty()
    {
        let query = logs_tab.viewer.search_query.to_ascii_lowercase();
        let end = logs_tab.viewer.scroll_offset;
        if let Some(pos) = logs_tab.viewer.lines[..end].iter().rev()
            .position(|l| l.to_ascii_lowercase().contains(&query))
        {
            logs_tab.viewer.scroll_offset = end - 1 - pos;
            logs_tab.viewer.follow_mode = false;
        }
        return true;
    }
    false
}
```

### Step 5: Update workbench log rendering

In `render_logs_tab`, when `search_query` is not empty, highlight matching substrings in each log line using `Span::styled()` with a highlight style (e.g., `theme.selection_bg`).

When `searching` is true, show search input bar:
```rust
if viewer.searching {
    // render search input line: "Search: {search_input}_"
}
```

Add `[/] search  [n/N] next/prev` to keybind hints.

### Step 6: Tests

```rust
#[test]
fn logs_viewer_search_state() {
    let mut viewer = LogsViewerState::default();
    assert!(!viewer.searching);
    assert!(viewer.search_query.is_empty());
    viewer.searching = true;
    viewer.search_input = "error".to_string();
    viewer.search_query = viewer.search_input.clone();
    viewer.searching = false;
    assert_eq!(viewer.search_query, "error");
}
```

### Step 7: Verify and commit

```
cargo fmt --all && cargo clippy && cargo test
git add src/app.rs src/events/input.rs src/ui/components/workbench.rs
git commit -m "feat: add / search with n/N navigation in pod logs"
```

---

## Task 4: Timestamp Toggle — `t` in log tabs

**Files:**
- Modify: `src/app.rs` (add `show_timestamps` to LogsViewerState, action variant)
- Modify: `src/events/input.rs` (handle action)
- Modify: `src/coordinator/logs.rs` (add `timestamps` parameter)
- Modify: `src/main.rs` (pass timestamps flag, handle re-fetch)
- Modify: `src/ui/components/workbench.rs` (show indicator)

### Step 1: Add field and action

In LogsViewerState: `pub show_timestamps: bool,` (default false)

In AppAction: `LogsViewerToggleTimestamps,`

### Step 2: Add key binding

In PodLogs handler:
```rust
KeyCode::Char('t') if !tab.viewer.searching => AppAction::LogsViewerToggleTimestamps,
```

### Step 3: Update stream_logs

Add `timestamps: bool` parameter. Update LogParams:
```rust
timestamps,
```

### Step 4: Handle in main.rs

When `LogsViewerToggleTimestamps`:
1. Toggle `show_timestamps`
2. Clear lines
3. Cancel current stream
4. Restart with new `timestamps` flag

### Step 5: Update workbench rendering

Add `[t] timestamps` to hint bar. Show "timestamps" badge when active.

### Step 6: Update all stream_logs call sites

Pass `false` (or the viewer's `show_timestamps` value) for every call.

### Step 7: Tests

```rust
#[test]
fn logs_viewer_timestamp_toggle() {
    let mut viewer = LogsViewerState::default();
    assert!(!viewer.show_timestamps);
    viewer.show_timestamps = true;
    assert!(viewer.show_timestamps);
}
```

### Step 8: Verify and commit

```
cargo fmt --all && cargo clippy && cargo test
git add src/app.rs src/events/input.rs src/coordinator/logs.rs src/main.rs src/ui/components/workbench.rs
git commit -m "feat: add t key to toggle timestamps in pod logs"
```

---

## Task 5: Sidebar Resource Counts

**Files:**
- Modify: `src/ui/components/mod.rs` (pass ClusterSnapshot to render_sidebar, show counts)
- Modify: `src/ui/mod.rs` (pass cluster to render_sidebar call)
- Modify: `src/app.rs` (add `AppView::resource_count` method on ClusterSnapshot)
- Modify: `src/state/mod.rs` (add `resource_count(view: AppView) -> Option<usize>` to ClusterSnapshot)

### Step 1: Add resource_count to ClusterSnapshot

In `src/state/mod.rs`, add method to `ClusterSnapshot`:

```rust
pub fn resource_count(&self, view: AppView) -> Option<usize> {
    match view {
        AppView::Pods => Some(self.pods.len()),
        AppView::Deployments => Some(self.deployments.len()),
        AppView::Services => Some(self.services.len()),
        AppView::Nodes => Some(self.nodes.len()),
        // ... all other views with stored data
        AppView::Dashboard => None,  // not a resource list
        _ => None,  // views not yet loaded return None
    }
}
```

### Step 2: Update render_sidebar signature

In `src/ui/components/mod.rs`, add `cluster: &ClusterSnapshot` parameter to `render_sidebar`:
```rust
pub fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    active: AppView,
    sidebar_cursor: usize,
    collapsed: &HashSet<NavGroup>,
    focus: crate::app::Focus,
    cluster: &ClusterSnapshot,
)
```

### Step 3: Update cached_sidebar_lines

Add `counts: &[(AppView, usize)]` to the cache key and the function. When rendering a `SidebarItem::View`, append the count:
```rust
SidebarItem::View(view) => {
    let count_suffix = counts.iter()
        .find(|(v, _)| v == view)
        .map(|(_, c)| format!(" {c}"))
        .unwrap_or_default();
    let line = format!("{}{count_suffix}", view.sidebar_text());
    // ... rest of styling
}
```

### Step 4: Update call site in ui/mod.rs

Pass `cluster` to the render_sidebar call:
```rust
components::render_sidebar(
    frame,
    body[0],
    app.view(),
    app.sidebar_cursor,
    &app.collapsed_groups,
    app.focus,
    cluster,
);
```

### Step 5: Tests

```rust
#[test]
fn resource_count_returns_some_for_pods() {
    let mut snapshot = ClusterSnapshot::default();
    snapshot.pods.push(PodInfo::default());
    assert_eq!(snapshot.resource_count(AppView::Pods), Some(1));
}

#[test]
fn resource_count_returns_none_for_dashboard() {
    let snapshot = ClusterSnapshot::default();
    assert_eq!(snapshot.resource_count(AppView::Dashboard), None);
}
```

### Step 6: Verify and commit

```
cargo fmt --all && cargo clippy && cargo test
git add src/ui/components/mod.rs src/ui/mod.rs src/state/mod.rs
git commit -m "feat: show resource counts in sidebar"
```

---

## Task 6: Pod IP Column in Pods Table

**Files:**
- Modify: `src/ui/mod.rs` (add IP column to pods table header, rows, and constraints)

### Step 1: Add IP column to header

In `render_pods_widget`, add after the Namespace column:
```rust
Cell::from(Span::styled("IP", theme.header_style())),
```

### Step 2: Add IP cell to rows

After the namespace cell in each row:
```rust
Cell::from(Span::styled(
    pod.pod_ip.as_deref().unwrap_or("-"),
    dim_style,
)),
```

### Step 3: Update column constraints

Add `Constraint::Length(16)` for the IP column (IPv4 max 15 chars + padding):
```rust
[
    Constraint::Min(28),     // Name
    Constraint::Length(18),  // Namespace
    Constraint::Length(16),  // IP       <-- new
    Constraint::Length(20),  // Status
    Constraint::Length(22),  // Node
    Constraint::Length(10),  // Restarts
    Constraint::Length(9),   // Age
]
```

### Step 4: Tests

Existing render smoke tests should still pass. Add test that PodInfo.pod_ip is rendered:

```rust
#[test]
fn pod_ip_renders_in_table() {
    let mut snapshot = ClusterSnapshot::default();
    snapshot.pods.push(PodInfo {
        name: "test-pod".to_string(),
        namespace: "default".to_string(),
        pod_ip: Some("10.0.0.1".to_string()),
        ..PodInfo::default()
    });
    // verify render doesn't panic
    draw_with_size(&app_with_view(AppView::Pods), &snapshot, 120, 30);
}
```

### Step 5: Verify and commit

```
cargo fmt --all && cargo clippy && cargo test
git add src/ui/mod.rs
git commit -m "feat: add Pod IP column to pods table"
```

---

## Quality Gate

After all 6 tasks, run the full quality gate:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

All must pass with zero warnings.

---

## Task Summary

| Task | Feature | Key Files | Estimated Complexity |
|------|---------|-----------|---------------------|
| 1 | Help overlay (`?`) | help_overlay.rs, app.rs, input.rs, mod.rs | Medium |
| 2 | Previous logs (`P`) | app.rs, logs.rs, main.rs, workbench.rs | Medium |
| 3 | Log search (`/`, `n`/`N`) | app.rs, input.rs, workbench.rs | Medium |
| 4 | Timestamps (`t`) | app.rs, logs.rs, main.rs, workbench.rs | Small |
| 5 | Sidebar counts | components/mod.rs, state/mod.rs, ui/mod.rs | Medium |
| 6 | Pod IP column | ui/mod.rs | Small |

Each task is independent and can be committed separately. Tasks 2 and 4 share similar patterns (toggle flag → cancel stream → restart stream) and should be implemented together or sequentially.
