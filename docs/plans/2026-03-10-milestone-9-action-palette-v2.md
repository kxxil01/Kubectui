# Milestone 9: Action Palette v2 — Design

## Status

Approved

## Goal

Turn the command palette into the main discoverability and action surface. Users can navigate views AND execute resource actions from a single `:` palette.

## Decisions

- **Unified palette**: One entry point (`:`) mixes navigation + context-aware actions
- **Hide unavailable**: Only show actions the user can execute right now
- **List selection works**: Actions available from highlighted list row, not just open detail view
- **Actions first**: Section headers separate "Actions" (top) from "Navigate" (bottom)
- **No namespace/context**: Palette does not include namespace or context switching

## Architecture

### Data Model

Replace the current `Command` struct with a `PaletteEntry` enum:

```rust
enum PaletteEntry {
    Navigate(AppView),
    Action(DetailAction),
}
```

Each entry has:
- `label` — display text
- `aliases` — fuzzy match keywords
- `key_hint` — optional shortcut display (actions only, e.g., `[s]`)
- `section` — Actions or Navigate

### Context Resolution

When the palette opens, resolve the current resource context:

1. If `detail_view.is_some()` → `ResourceRef` from `detail.resource`
2. Else → `selected_resource()` from list highlight (may be None)

Actions are filtered by `ResourceRef::supports_detail_action()` from policy.rs.

### New AppAction Variant

```rust
AppAction::PaletteAction {
    action: DetailAction,
    resource: ResourceRef,
}
```

Carries both the action and target resource so main.rs can open detail view if needed, then dispatch the action in one step.

### Data Flow

```
User presses `:`
  → palette opens
  → resolve context (detail_view or list selection)
  → build entries:
      if ResourceRef is Some → filter DetailAction::ORDER by supports_detail_action()
      always → append 46 AppView navigation entries
  → render with section headers

User selects entry:
  Navigate(view) → AppAction::NavigateTo(view)
  Action(action) → AppAction::PaletteAction { action, resource }

main.rs handles PaletteAction:
  if detail_view.is_none() → open detail first, then trigger action
  if detail_view.is_some() → trigger action directly
```

### Rendering

Same popup dimensions as v1. Layout:

1. Header: `⌘ Action Palette · type to filter`
2. Search box: `: <query>█` or placeholder
3. Results: section header `── Actions ──` then action entries, `── Navigate ──` then view entries
   - Action entry: `▶ Scale             [s]`
   - Nav entry: `▶ Deployments        Workloads`
   - Sections with 0 matches hidden entirely
4. Footer: `[↑↓/jk] navigate  [Enter] select  [Esc] close`

### Fuzzy Matching

Same subsequence matching as v1. Applied to both action aliases and view aliases. Actions use `DetailAction::label()` plus additional aliases:

- Scale → "scale", "replicas"
- Restart → "restart", "rollout"
- Logs → "logs", "log"
- Exec → "exec", "shell"
- Delete → "delete", "remove"
- etc.

### Action Aliases

```
ViewYaml    → ["yaml", "manifest"]
ViewEvents  → ["events"]
Logs        → ["logs", "log"]
Exec        → ["exec", "shell", "terminal"]
PortForward → ["port-forward", "forward", "tunnel"]
Probes      → ["probes", "health"]
Scale       → ["scale", "replicas"]
Restart     → ["restart", "rollout"]
FluxReconcile → ["reconcile", "flux"]
EditYaml    → ["edit", "modify"]
Delete      → ["delete", "remove"]
Trigger     → ["trigger", "run"]
```

## Testing

- Action availability with Pod → shows Logs/Exec/PortForward/Probes, not Scale
- Action availability with Deployment → shows Scale/Restart/Logs, not Exec
- No resource selected → no actions section, only navigation
- Fuzzy matching: "scl" matches Scale, "dply" matches Deployments
- PaletteAction dispatch: Scale from palette on list-highlighted Deployment opens detail + scale dialog
- Section headers hidden when section has 0 matches
- Empty query shows all actions + all views
- Palette closes after action selection

## Files to Modify

- `src/ui/components/command_palette.rs` — core rewrite
- `src/app.rs` — add PaletteAction variant, pass context to palette
- `src/events/input.rs` — handle PaletteAction dispatch
- `src/main.rs` — handle PaletteAction (open detail if needed, trigger action)
- `src/policy.rs` — no changes needed (already has all capability queries)

## Risks

- Action list becoming noisy → mitigated by hiding unavailable actions
- Conflicting shortcuts between palette and direct keys → palette actions map 1:1 to existing shortcuts
- Detail view open race → palette captures ResourceRef at open time, stale ref is harmless (action will no-op)
