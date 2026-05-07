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
2. `src/ui/mod.rs`
3. `src/ui/components/`
4. `src/app/mod.rs`
5. `src/app/input.rs`

Read these additional files when the task crosses their boundary:

- `src/app/` for app state, view selection, workbench state, and sort state
- `src/events/input.rs` for action routing
- `src/policy.rs` for view and detail action policy
- `src/columns.rs` for shared column definitions
- `src/state/` for snapshot and refresh state

## Canonical Paths

Keep a single source of truth.

- Shared filtering and cache helpers live under `src/ui/`
- Shared table helpers live in `src/ui/mod.rs`
- Shared columns live in `src/columns.rs`
- View-specific rendering lives in `src/ui/mod.rs` and focused UI/component modules
- Reusable overlays and dialogs live in `src/ui/components/*`
- Terminal setup and teardown live in `src/terminal.rs`
- Event-loop orchestration lives in `src/main.rs`; route pure helpers into focused modules when safe

Do not introduce parallel render paths, duplicate filtering code, or ad hoc column registries.

## Serena Workflow

Use Serena MCP first for repo exploration and symbol-level edits.

- Prefer `get_symbols_overview`, `find_symbol`, `find_referencing_symbols`,
  `find_implementations`, and `search_for_pattern` before broad file reads
- Prefer Serena symbol edits for whole functions, methods, types, or modules
- Use shell commands for build/test/git/gh, non-code files, exact text search, and cases where
  Serena cannot answer directly
- Avoid full reads of large files like `src/main.rs`, `src/workbench.rs`, and `src/k8s/client.rs`
  unless narrow symbol search is insufficient
- Still run cargo validation after Serena-assisted edits

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
