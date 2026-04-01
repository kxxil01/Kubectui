# AGENTS.md

## Scope
This file governs `/Users/ilham/Developer/Kubectui` and all subdirectories.

## Communication
- Be concise and direct. Lead with the answer, not the reasoning.
- Use bullet points for lists. Skip unnecessary preamble.
- Don't explain what you're about to do — just do it.

## Environment
- macOS, VS Code, zsh
- Rust toolchain (stable), cargo

## Primary Goals
1. Keep the app correct and stable (no functional regressions).
2. Improve render-path speed and responsiveness.
3. Keep UI/UX flow intact while optimizing.

## Code Style
- Prefer explicit, readable code over dense one-liners.
- No commented-out code — delete it.
- Self-documenting code over excessive comments.

## Engineering Standards (Rust)
- Prefer safe Rust; isolate any `unsafe` behind minimal, documented abstractions.
- Follow Rust idioms: ownership-first design, zero-cost abstractions, predictable allocation behavior.
- Maintain `cargo clippy --all-targets --all-features -- -D warnings` clean.
- Keep APIs panic-safe for runtime paths; fail fast on invalid invariants.
- Keep tests comprehensive (`unit`, `integration`, performance checks where relevant).

## Current Performance Checkpoints
- First performance checkpoint already shipped:
  - Commit: `c9c0b5f`
  - Focus: numeric-cell allocation reductions on hot table views.
- Second performance checkpoint already shipped:
  - Commit: `3cd45fd`
  - Focus: same optimization pattern for remaining numeric-heavy views.
- 5-run median comparison (patch off vs on for second pass):
  - `render`: `3348.976ms -> 3282.163ms` (`-66.813ms`, `-2.00%`).

## Performance Status

### Completed
- **Row Virtualization**: `table_window()` + `table_viewport_rows()` in all 31 view files.
  Only visible `window.start..window.end` rows are built. Tests cover top/middle/bottom/empty.
- **Per-View Cached Formatted Cells**: `DerivedRowsCache<T>` in 26 view files.
  Keyed by `(query, snapshot_version, data_fingerprint, variant, freshness_bucket)`.
  Filter indices cached via `cached_filter_indices_with_variant` with MRU fast path.
- **Filtering**: `contains_ci` uses SIMD-friendly `eq_ignore_ascii_case` — benchmarked at
  12µs for 2000 pods (0.09% of 16ms frame budget). First-byte rejection tested slower.
- **Shared ordinary-table render path**: `render_resource_table()` now owns the common
  empty-state, windowing, title, table-frame, and striped-row path for the high-overlap
  workload/resource tables instead of each view reassembling that flow independently.
- **Structural split pass**: `src/app/mod.rs` now carries app types plus module wiring while
  behavior lives in focused submodules (`core`, `navigation`, `preferences`, `workbench`), and
  startup/bootstrap helper code no longer lives inline in `src/main.rs`.
- **Frame-skip render caching**: the canonical render path now skips unchanged header, sidebar,
  status, and main-content regions when their render inputs are stable, with same-terminal
  regression coverage for theme, icon, search, and selection invalidation.

### Refactoring Completed (through 2026-03-29)
- PR #39: Unified 11 derived-cell caches, 30 fetch methods → 2 macros, metadata helper (-659 lines)
- PR #40: Column registry const fn helpers (-845 lines)
- PR #41: Unified view_key/columns_for_view into single view_info() (-18 lines)
- 2026-03-19: structural split of app state/tests/startup helpers into focused modules
- 2026-03-29: perf-gate parsing hardened so `header`/`status` can fall back to folded spans when they drop out of the summary
- Core refactor track shipped without changing the canonical runtime path

### Remaining Opportunities
1. **Split `main.rs` event-loop surface** (`src/main.rs` is ~5917 lines): extract async task
   orchestration, event routing, and workbench lifecycle out of the root binary entrypoint.
2. **Decompose `AppAction` and input routing**: `src/app/mod.rs` is small now, but the canonical
   action enum and keyboard routing surface are still broad; group them without forking behavior.
3. **Extend shared render helpers to the remaining specialized views**: Pods and split-pane
   overview/diagnostic surfaces still keep custom hot paths; consolidate repeated framing only
   where it preserves one canonical implementation.

## Performance Validation Protocol (Required)
Use median-based checks (not single-run decisions).

Required process:
1. Run profiling test 5 times for baseline and 5 times for candidate.
2. Compare medians per key metrics:
   - `render`, `sidebar`, `header`
   - Top hot views in current profile
3. Keep a change only if:
   - Global `render` median improves, and
   - No critical hotspot regresses materially without explicit reason.

Recommended command:
- `cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture`

## Quality Gate Before Commit
Run all of the following:
1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets --all-features`
4. Performance profiling check for changed render paths

## Git Rules
- **NEVER push directly to main.** All changes must go through: new branch → open PR → merge PR to main.
- **Never add `Co-Authored-By` lines to commit messages.** Commit as the user only.
- Branch naming: `fix/description`, `feat/description`, `chore/description`, `refactor/description`
- Use `gh pr create` to open PRs, `gh pr merge --squash --delete-branch` to merge.
- This ensures an accountable changelog via PR history.
- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `refactor:`, `perf:`, `docs:`, `test:`).
- Write commit messages in imperative mood ("add feature" not "added feature").
- Keep commit subject lines under 72 characters.
- Keep commits scoped to one measurable objective.
- Ask before any destructive git action.
- Ask before force-push (never default to force).
