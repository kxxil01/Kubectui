---
name: kubectui-ratatui-engineer
description: Use when working on Kubectui's Ratatui and Crossterm UI, including render-path optimization, table views, scrolling, filtering, windowing, dialogs, and terminal event handling. This complements the generic rust-engineer skill and should be preferred for repo-local TUI work.
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
---

# Kubectui Ratatui Engineer

Repo-local skill for Kubectui UI work. This augments the global `rust-engineer` skill instead
of replacing it.

## Use This Skill For

- Ratatui layout, tables, widgets, scrollbars, overlays, and dialogs
- Crossterm input or terminal lifecycle changes
- Render-path optimization and hot-path allocation reduction
- Filtering, sorting, selection, windowing, and visible-row behavior
- Kubectui view additions or UI refactors that must preserve current UX flow

## First Read

1. `AGENTS.md`
2. `CLAUDE.md`
3. `src/ui/mod.rs`
4. `src/ui/views/`
5. `src/ui/components/`

Read these additional files when the task crosses their boundary:

- `src/app.rs` for app state, view selection, and sort state
- `src/events/input.rs` for action routing
- `src/policy.rs` for view and detail action policy
- `src/columns.rs` for shared column definitions
- `src/state/` for snapshot and refresh state

## Canonical Paths

Keep a single source of truth.

- Shared filtering logic lives in `src/ui/views/filtering.rs`
- Shared table helpers live in `src/ui/mod.rs`
- Shared columns live in `src/columns.rs`
- View-specific rendering lives in `src/ui/views/*`
- Reusable overlays and dialogs live in `src/ui/components/*`
- Terminal setup and teardown live in `src/main.rs`

Do not introduce parallel render paths, duplicate filtering code, or ad hoc column registries.

## Table View Rules

For any high-cardinality table:

- Use `table_viewport_rows(area)` to compute visible capacity
- Use `table_window(total, selected, viewport_rows)` to compute the visible range
- Keep the selected row visible at all times
- Keep scrollbar position based on the full filtered dataset length
- Use `responsive_table_widths_vec()` or `responsive_table_widths()` for width fitting
- Keep filtering and sort semantics unchanged when adding windowing

If you change one table view, look for the shared helper first. Extend the canonical helper rather
than cloning logic into another module.

## Render-Path Expectations

Kubectui is already optimized around centralized filtering and hot-path allocation reduction.
Preserve that direction.

- Prefer cached filtered indices over rebuilding filtered vectors in multiple places
- Reuse derived row data for hot views instead of formatting the same values repeatedly
- Preallocate row storage with `Vec::with_capacity`
- Prefer borrowed data or `Cow` where it avoids unnecessary allocation
- Avoid building strings in the render loop unless the value is genuinely per-frame
- Handle time-sensitive cells such as age with explicit invalidation or lightweight recompute
- Keep side effects and network work out of render code

When working on tables, inspect existing helpers such as:

- `cached_filter_indices_with_variant`
- `data_fingerprint`
- `cached_pod_derived`
- `format_small_int`

## Interaction Rules

- Keep rendering pure: no mutation, I/O, or coordinator side effects in widget builders
- Route state transitions through existing app and event paths
- Preserve current keyboard flow unless the user explicitly asks to change it
- If mouse support is touched, account for scroll offsets and visible window offsets
- Fail fast on invalid UI invariants instead of silently desynchronizing selection state

## Ratatui Guidance

Prefer first-class Ratatui and Crossterm usage over wrappers.

- Reuse Ratatui primitives directly when the library already models the behavior cleanly
- Extract a helper or component only when it removes repeated logic or stabilizes a real invariant
- Keep markup nesting shallow; extract repeated cell or block construction
- Consult official Ratatui examples before inventing new widget behavior

See `references/source-notes.md` for the external material this skill borrows from and what was
intentionally excluded.

## Change Checklist

- Is the implementation on the primary codepath?
- Is business logic still centralized?
- Are filtering, selection, and scrollbar semantics preserved?
- Did render-path allocations stay flat or improve?
- Did the change avoid new duplicate helpers?

## Validation

Always run:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

If the change touches rendering, windowing, or hot views, also run the required median-based
performance check from `AGENTS.md`:

```bash
cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture
```
