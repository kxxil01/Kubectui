# Action Palette v2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extend the command palette (`:`) to show context-aware resource actions alongside view navigation, so users can discover and execute actions from a single surface.

**Architecture:** Replace the static `Command` struct with a `PaletteEntry` enum supporting both `Navigate(AppView)` and `Action(DetailAction)`. The palette resolves the current resource context (detail view or list selection) at open time, filters available actions via `ResourceRef::supports_detail_action()`, and returns a new `AppAction::PaletteAction` variant that main.rs dispatches by opening the detail view if needed and triggering the action.

**Tech Stack:** Rust, ratatui, crossterm, existing policy.rs capability model

---

### Task 1: Add PaletteEntry enum and action aliases

**Files:**
- Modify: `src/ui/components/command_palette.rs:16-27` (replace Command struct, extend CommandPaletteAction)

**Step 1: Write the failing test**

In `src/ui/components/command_palette.rs`, add to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn palette_entry_action_aliases_match() {
    use crate::policy::DetailAction;
    let entries = action_entries_for_resource(None);
    assert!(entries.is_empty(), "No actions without resource");
}

#[test]
fn palette_entry_action_aliases_pod() {
    use crate::app::ResourceRef;
    use crate::policy::DetailAction;
    let resource = ResourceRef::Pod("test".into(), "default".into());
    let entries = action_entries_for_resource(Some(&resource));
    assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
    assert!(entries.iter().any(|e| e.action == DetailAction::Exec));
    assert!(!entries.iter().any(|e| e.action == DetailAction::Scale));
}

#[test]
fn palette_entry_action_aliases_deployment() {
    use crate::app::ResourceRef;
    use crate::policy::DetailAction;
    let resource = ResourceRef::Deployment("api".into(), "default".into());
    let entries = action_entries_for_resource(Some(&resource));
    assert!(entries.iter().any(|e| e.action == DetailAction::Scale));
    assert!(entries.iter().any(|e| e.action == DetailAction::Restart));
    assert!(entries.iter().any(|e| e.action == DetailAction::Logs));
    assert!(!entries.iter().any(|e| e.action == DetailAction::Exec));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib command_palette::tests::palette_entry_action`
Expected: FAIL — `action_entries_for_resource` not found

**Step 3: Write the data model and action_entries_for_resource**

Replace the existing `Command` struct and add new types at the top of `command_palette.rs`:

```rust
use crate::app::{AppView, ResourceRef};
use crate::policy::DetailAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEntry {
    Navigate(AppView),
    Action(DetailAction),
}

#[derive(Debug, Clone)]
pub struct ActionEntry {
    pub action: DetailAction,
    pub aliases: &'static [&'static str],
}

const ACTION_ALIASES: &[(DetailAction, &[&str])] = &[
    (DetailAction::ViewYaml, &["yaml", "manifest"]),
    (DetailAction::ViewEvents, &["events", "event"]),
    (DetailAction::Logs, &["logs", "log"]),
    (DetailAction::Exec, &["exec", "shell", "terminal"]),
    (DetailAction::PortForward, &["port-forward", "forward", "tunnel", "pf"]),
    (DetailAction::Probes, &["probes", "health", "probe"]),
    (DetailAction::Scale, &["scale", "replicas"]),
    (DetailAction::Restart, &["restart", "rollout"]),
    (DetailAction::FluxReconcile, &["reconcile", "flux"]),
    (DetailAction::EditYaml, &["edit", "modify"]),
    (DetailAction::Delete, &["delete", "remove"]),
    (DetailAction::Trigger, &["trigger", "run"]),
];

pub fn action_entries_for_resource(resource: Option<&ResourceRef>) -> Vec<ActionEntry> {
    let Some(resource) = resource else {
        return Vec::new();
    };
    ACTION_ALIASES
        .iter()
        .filter(|(action, _)| resource.supports_detail_action(*action))
        .map(|(action, aliases)| ActionEntry {
            action: *action,
            aliases,
        })
        .collect()
}
```

Keep the existing `COMMANDS` array and `Command` struct for navigation entries (rename to `NavCommand` for clarity internally, or keep as-is — the `COMMANDS` array stays unchanged).

**Step 4: Run test to verify it passes**

Run: `cargo test --lib command_palette::tests::palette_entry_action`
Expected: PASS

**Step 5: Commit**

```
feat(palette): add PaletteEntry model and action alias catalog
```

---

### Task 2: Extend CommandPaletteAction and CommandPalette state

**Files:**
- Modify: `src/ui/components/command_palette.rs:16-21` (CommandPaletteAction enum)
- Modify: `src/ui/components/command_palette.rs:178-184` (CommandPalette struct)

**Step 1: Write the failing test**

```rust
#[test]
fn palette_set_context_enables_actions() {
    let mut palette = CommandPalette::default();
    let resource = ResourceRef::Pod("test".into(), "default".into());
    palette.open_with_context(Some(resource.clone()));
    assert!(palette.is_open());
    assert!(palette.resource_context().is_some());
}

#[test]
fn palette_open_without_context_has_no_actions() {
    let mut palette = CommandPalette::default();
    palette.open_with_context(None);
    assert!(palette.is_open());
    assert!(palette.resource_context().is_none());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib command_palette::tests::palette_set_context`
Expected: FAIL — `open_with_context` not found

**Step 3: Implement state changes**

Extend the `CommandPaletteAction` enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteAction {
    None,
    Navigate(AppView),
    Execute(DetailAction, ResourceRef),
    Close,
}
```

Extend the `CommandPalette` struct:

```rust
#[derive(Debug, Clone, Default)]
pub struct CommandPalette {
    query: String,
    selected_index: usize,
    is_open: bool,
    cached_filtered: RefCell<Option<Vec<PaletteEntry>>>,
    resource_context: Option<ResourceRef>,
}
```

Add methods:

```rust
pub fn open_with_context(&mut self, resource: Option<ResourceRef>) {
    self.query.clear();
    self.selected_index = 0;
    self.is_open = true;
    self.resource_context = resource;
    self.invalidate_cache();
}

pub fn resource_context(&self) -> Option<&ResourceRef> {
    self.resource_context.as_ref()
}
```

Update the existing `open()` method to call `open_with_context(None)` and `close()` to clear `resource_context`.

**Step 4: Run test to verify it passes**

Run: `cargo test --lib command_palette::tests::palette_set_context`
Expected: PASS

**Step 5: Commit**

```
feat(palette): extend state with resource context and Execute action
```

---

### Task 3: Rewrite filtered() to return PaletteEntry with sections

**Files:**
- Modify: `src/ui/components/command_palette.rs:250-270` (filtered method)

**Step 1: Write the failing test**

```rust
#[test]
fn filtered_returns_actions_then_navigation() {
    let mut palette = CommandPalette::default();
    let resource = ResourceRef::Deployment("api".into(), "default".into());
    palette.open_with_context(Some(resource));
    let entries = palette.filtered();
    // Actions should come first
    let first_action_idx = entries.iter().position(|e| matches!(e, PaletteEntry::Action(_)));
    let first_nav_idx = entries.iter().position(|e| matches!(e, PaletteEntry::Navigate(_)));
    assert!(first_action_idx.is_some());
    assert!(first_nav_idx.is_some());
    assert!(first_action_idx.unwrap() < first_nav_idx.unwrap());
}

#[test]
fn filtered_with_query_matches_actions_and_views() {
    let mut palette = CommandPalette::default();
    let resource = ResourceRef::Deployment("api".into(), "default".into());
    palette.open_with_context(Some(resource));
    palette.set_query("scl");
    let entries = palette.filtered();
    assert!(entries.iter().any(|e| matches!(e, PaletteEntry::Action(DetailAction::Scale))));
}

#[test]
fn filtered_no_context_has_no_actions() {
    let mut palette = CommandPalette::default();
    palette.open_with_context(None);
    let entries = palette.filtered();
    assert!(entries.iter().all(|e| matches!(e, PaletteEntry::Navigate(_))));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib command_palette::tests::filtered_returns`
Expected: FAIL

**Step 3: Rewrite filtered()**

```rust
pub fn filtered(&self) -> Vec<PaletteEntry> {
    if let Some(cached) = self.cached_filtered.borrow().as_ref() {
        return cached.clone();
    }

    let mut result = Vec::new();

    // Actions section (only if resource context exists)
    if let Some(resource) = &self.resource_context {
        let actions = action_entries_for_resource(Some(resource));
        for entry in &actions {
            if self.query.is_empty()
                || entry
                    .aliases
                    .iter()
                    .any(|alias| fuzzy_match(alias, &self.query))
            {
                result.push(PaletteEntry::Action(entry.action));
            }
        }
    }

    // Navigation section
    for cmd in COMMANDS {
        if self.query.is_empty()
            || cmd
                .aliases
                .iter()
                .any(|alias| fuzzy_match(alias, &self.query))
        {
            result.push(PaletteEntry::Navigate(cmd.view));
        }
    }

    *self.cached_filtered.borrow_mut() = Some(result.clone());
    result
}
```

Add a `set_query` helper for tests (or use existing input handling).

**Step 4: Run test to verify it passes**

Run: `cargo test --lib command_palette::tests::filtered_`
Expected: PASS

**Step 5: Commit**

```
feat(palette): rewrite filtered() to return PaletteEntry with actions first
```

---

### Task 4: Update handle_key() to return Execute action

**Files:**
- Modify: `src/ui/components/command_palette.rs:202-248` (handle_key method)

**Step 1: Write the failing test**

```rust
#[test]
fn handle_key_enter_on_action_returns_execute() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut palette = CommandPalette::default();
    let resource = ResourceRef::Pod("test".into(), "default".into());
    palette.open_with_context(Some(resource.clone()));
    // First entry should be an action (e.g., ViewYaml)
    let result = palette.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    match result {
        CommandPaletteAction::Execute(action, ref res) => {
            assert_eq!(*res, resource);
        }
        _ => panic!("Expected Execute, got {:?}", result),
    }
}

#[test]
fn handle_key_enter_on_nav_returns_navigate() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut palette = CommandPalette::default();
    palette.open_with_context(None);
    // No actions, first entry is navigation
    let result = palette.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(matches!(result, CommandPaletteAction::Navigate(_)));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib command_palette::tests::handle_key_enter`
Expected: FAIL

**Step 3: Update handle_key Enter branch**

In the `KeyCode::Enter` handler, replace the current logic with:

```rust
KeyCode::Enter => {
    let entries = self.filtered();
    if let Some(entry) = entries.get(self.selected_index) {
        match entry {
            PaletteEntry::Navigate(view) => CommandPaletteAction::Navigate(*view),
            PaletteEntry::Action(action) => {
                if let Some(resource) = &self.resource_context {
                    CommandPaletteAction::Execute(*action, resource.clone())
                } else {
                    CommandPaletteAction::None
                }
            }
        }
    } else {
        CommandPaletteAction::None
    }
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test --lib command_palette::tests::handle_key_enter`
Expected: PASS

**Step 5: Commit**

```
feat(palette): handle_key returns Execute for action entries
```

---

### Task 5: Update render() to show section headers and key hints

**Files:**
- Modify: `src/ui/components/command_palette.rs:272-412` (render method)

**Step 1: No test needed** — rendering is visual. Verify manually.

**Step 2: Update render()**

Replace the result list rendering section. Key changes:

1. Update header text from `"⌘ Command Palette · type to jump"` to `"⌘ Action Palette · type to filter"`
2. When iterating filtered entries, track section transitions:
   - Before the first `PaletteEntry::Action`, render `"── Actions ──"` as a dim section header
   - Before the first `PaletteEntry::Navigate`, render `"── Navigate ──"` as a dim section header
   - Section headers are not selectable (skip them in index counting)
3. For action entries, render: `"  {label}  {key_hint}"` using `DetailAction::label()` and `DetailAction::key_hint()`
4. For nav entries, keep current rendering: `"  {view_label}  {group_label}"`
5. The `▶` selector still tracks `selected_index` but skips section header lines

Implementation approach: build a `Vec<RenderLine>` enum with `SectionHeader(String)`, `ActionItem(DetailAction, bool)`, `NavItem(AppView, bool)` where `bool` is `is_selected`. Map `selected_index` to the Nth selectable item (skip headers).

**Step 3: Commit**

```
feat(palette): render section headers and key hints for actions
```

---

### Task 6: Add AppAction::PaletteAction variant

**Files:**
- Modify: `src/app.rs:1271-1354` (AppAction enum — add variant)
- Modify: `src/events/input.rs:59-72` (handle new action)

**Step 1: Write the failing test**

In `src/events/input.rs` tests:

```rust
#[test]
fn apply_action_palette_action_closes_palette() {
    use crate::policy::DetailAction;
    let mut app = AppState::default();
    app.command_palette.open_with_context(Some(
        ResourceRef::Pod("test".into(), "default".into()),
    ));
    let changed = apply_action(
        AppAction::PaletteAction {
            action: DetailAction::ViewYaml,
            resource: ResourceRef::Pod("test".into(), "default".into()),
        },
        &mut app,
    );
    assert!(changed);
    assert!(!app.command_palette.is_open());
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test --lib events::input::tests::apply_action_palette`
Expected: FAIL — `PaletteAction` variant not found

**Step 3: Add the variant and handler**

In `src/app.rs`, add to `AppAction`:
```rust
PaletteAction {
    action: DetailAction,
    resource: ResourceRef,
},
```

Add import for `DetailAction` in app.rs if needed.

In `src/events/input.rs`, add handler:
```rust
AppAction::PaletteAction { .. } => {
    app_state.command_palette.close();
    true
}
```

The actual action dispatch (opening detail, triggering scale, etc.) happens in main.rs, not in apply_action. apply_action just closes the palette.

**Step 4: Run test to verify it passes**

Run: `cargo test --lib events::input::tests::apply_action_palette`
Expected: PASS

**Step 5: Commit**

```
feat(palette): add PaletteAction variant to AppAction
```

---

### Task 7: Wire palette context in main.rs (open with resource)

**Files:**
- Modify: `src/main.rs` — where `OpenCommandPalette` is handled and where `:` triggers

**Step 1: No unit test** — integration behavior.

**Step 2: Update palette open to pass resource context**

Find where `AppAction::OpenCommandPalette` is handled in main.rs. It currently calls `apply_action(OpenCommandPalette, &mut app)` which calls `app.command_palette.open()`.

Change the flow: in the keyboard input section of main.rs where `:` is detected, instead of returning `AppAction::OpenCommandPalette`, resolve the resource context first:

```rust
KeyCode::Char(':') if self.detail_view.is_none() && !self.is_search_mode() => {
    // handled in main.rs directly
    AppAction::OpenCommandPalette
}
```

In main.rs, change the `OpenCommandPalette` handler:

```rust
AppAction::OpenCommandPalette => {
    let resource_ctx = app
        .detail_view
        .as_ref()
        .and_then(|d| d.resource.clone())
        .or_else(|| selected_resource(&app, &cached_snapshot));
    app.command_palette.open_with_context(resource_ctx);
    needs_redraw = true;
}
```

Remove the `OpenCommandPalette` case from `events/input.rs` `apply_action` since main.rs handles it directly now.

**Step 3: Commit**

```
feat(palette): pass resource context when opening palette
```

---

### Task 8: Handle PaletteAction dispatch in main.rs

**Files:**
- Modify: `src/main.rs` — action match block (around line 1150+)

**Step 1: No unit test** — async integration handled by existing patterns.

**Step 2: Add PaletteAction handler in main.rs**

In the main action match block, add a case before the `other => apply_action(other, &mut app)` fallthrough:

```rust
AppAction::PaletteAction { action, resource } => {
    app.command_palette.close();

    // Map DetailAction to the corresponding AppAction
    let mapped_action = match action {
        DetailAction::ViewYaml => Some(AppAction::OpenResourceYaml),
        DetailAction::ViewEvents => Some(AppAction::OpenResourceEvents),
        DetailAction::Logs => Some(AppAction::LogsViewerOpen),
        DetailAction::Exec => Some(AppAction::OpenExec),
        DetailAction::PortForward => Some(AppAction::PortForwardOpen),
        DetailAction::Probes => Some(AppAction::ProbePanelOpen),
        DetailAction::Scale => Some(AppAction::ScaleDialogOpen),
        DetailAction::Restart => Some(AppAction::RolloutRestart),
        DetailAction::FluxReconcile => Some(AppAction::FluxReconcile),
        DetailAction::EditYaml => Some(AppAction::EditYaml),
        DetailAction::Delete => Some(AppAction::DeleteResource),
        DetailAction::Trigger => Some(AppAction::TriggerCronJob),
    };

    if let Some(target_action) = mapped_action {
        // Ensure detail view is open for actions that need it
        let needs_detail = !matches!(
            action,
            DetailAction::Logs | DetailAction::ViewEvents
        );

        if needs_detail && app.detail_view.is_none() {
            // Open detail view first, then queue the action
            open_detail_for_resource(
                &mut app,
                &cached_snapshot,
                &client,
                &detail_tx,
                resource,
            );
            // Store pending action to execute after detail loads
            pending_palette_action = Some(target_action);
        } else {
            // Detail already open or not needed — execute directly
            // Re-enter the action match (use a helper or goto)
            // Simplest: just set the action and loop
            pending_palette_action = Some(target_action);
        }
    }
}
```

Add `pending_palette_action: Option<AppAction>` to the local state near the top of `run_app()`.

In the detail_rx handler (where detail fetch completes), check for pending action:

```rust
// After detail view is populated successfully:
if let Some(queued_action) = pending_palette_action.take() {
    // Re-dispatch the queued action now that detail is ready
    // (handle it in the next loop iteration by re-assigning)
    deferred_action = Some(queued_action);
}
```

At the top of the main loop, before the select!, check for deferred actions:

```rust
if let Some(action) = deferred_action.take() {
    // Process this action as if it came from keyboard input
    match action {
        AppAction::ScaleDialogOpen => { /* existing handler */ }
        // ... etc
        other => { apply_action(other, &mut app); }
    }
    needs_redraw = true;
    continue;
}
```

This is the most complex task. The key insight: some actions (Scale, Restart, Edit, Delete, Probes) require a detail view to be open first. The palette must open the detail view and then queue the action for after the async detail fetch completes.

For actions that DON'T need detail view (Logs, Events), dispatch immediately.

**Step 3: Commit**

```
feat(palette): dispatch PaletteAction with detail-open-then-act pattern
```

---

### Task 9: Allow palette to open from detail view too

**Files:**
- Modify: `src/app.rs:2863` — remove `self.detail_view.is_none()` guard on `:`

**Step 1: Write the failing test**

```rust
#[test]
fn colon_opens_palette_from_detail_view() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = AppState::default();
    app.detail_view = Some(DetailViewState {
        resource: Some(ResourceRef::Pod("test".into(), "default".into())),
        ..DetailViewState::default()
    });
    let action = app.handle_key_event(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
    assert_eq!(action, AppAction::OpenCommandPalette);
}
```

**Step 2: Run test to verify it fails**

Expected: FAIL — current guard blocks `:` when detail_view is open

**Step 3: Remove the guard**

Change `app.rs:2863`:
```rust
// Before:
KeyCode::Char(':') if self.detail_view.is_none() => AppAction::OpenCommandPalette,
// After:
KeyCode::Char(':') => AppAction::OpenCommandPalette,
```

But keep the guard that blocks `:` during search mode, namespace picker, etc. Check the surrounding conditions.

**Step 4: Run test to verify it passes**

Run: `cargo test --lib app::tests::colon_opens_palette`
Expected: PASS

**Step 5: Commit**

```
feat(palette): allow opening action palette from detail view
```

---

### Task 10: Update help overlay with palette changes

**Files:**
- Modify: wherever the `?` help overlay keybinding list is defined

**Step 1: Find the help overlay data**

Search for the help overlay keybinding entries (likely in `src/ui/views/` or `src/app.rs`).

**Step 2: Update the `:` entry**

Change from "Command Palette" to "Action Palette" in the help overlay text. Ensure the description mentions both navigation and resource actions.

**Step 3: Commit**

```
docs(palette): update help overlay for action palette
```

---

### Task 11: Final integration test and cleanup

**Files:**
- All modified files

**Step 1: Run full quality gate**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

**Step 2: Verify no regressions**

- Existing palette tests still pass
- Navigation-only palette (no resource selected) works as before
- Policy tests unchanged

**Step 3: Update plan.md**

Mark Milestone 9 as completed in `plan.md`.

**Step 4: Commit**

```
docs: mark milestone 9 complete
```

---

## Task Dependency Graph

```
Task 1 (PaletteEntry + aliases)
  → Task 2 (state extension)
    → Task 3 (filtered rewrite)
      → Task 4 (handle_key update)
        → Task 5 (render update)
Task 6 (AppAction variant) — parallel with Tasks 1-5
  → Task 7 (wire context in main.rs) — needs Tasks 2 + 6
    → Task 8 (PaletteAction dispatch) — needs Tasks 4 + 7
      → Task 9 (palette from detail)
        → Task 10 (help overlay)
          → Task 11 (integration + cleanup)
```

Tasks 1-5 and Task 6 can be done in parallel tracks. Task 7 merges them.
