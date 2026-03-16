# Autoresearch: Render Performance

## Objective

Optimize the render hot path of KubecTUI's ratatui-based terminal UI. The profiling test renders all 46 views (1920 frames total) against a 1200-item dataset. We target the `render` span total — the cumulative time spent inside `ui::render()` across all frames.

## Metric

- **Primary**: render span total (ms) — lower is better
- **Secondary**: sidebar span total (ms), header span total (ms), top-5 view totals (ms)

## Command

```bash
./autoresearch.sh
```

Runs the profile test 5 times and reports the median `render` total from `render-frame-summary.txt`.

## Checks Command

```bash
./autoresearch.checks.sh
```

Runs `cargo fmt --check`, `cargo clippy`, and `cargo test`.

## Files in Scope

- `src/ui/` — view renderers, components, theme, profiling
- `src/app/` — AppState, sidebar, sort, config
- `src/columns.rs` — ColumnDef registries
- `src/workbench.rs` — tab states
- `src/state/` — ClusterSnapshot, refresh pipeline

## Off Limits

- `src/k8s/` — Kubernetes client (not in render path)
- `tests/` — test infrastructure (only modify if needed for profiling)
- `src/main.rs` — event loop (not in render path)
- `.github/` — CI/CD

## Constraints

- All tests must pass (`cargo test --all-targets --all-features`)
- Clippy must be clean (`cargo clippy --all-targets --all-features -- -D warnings`)
- No UI behavior regressions (visual output must remain identical)
- No removing views, columns, or features to "optimize"

## Baseline

- **Value**: 316.756ms (5-run median)
- **Date**: 2026-03-16

## Initial Analysis (Pre-Baseline)

Hot spots from profiling (single run, 1920 frames):
- `render` total: 303.8ms (100%)
- `sidebar`: 94.4ms (31%) — cache invalidates on cursor move, rebuilds all lines
- `header`: 13.8ms (5%)
- `view.issues`: 8.9ms — highest single view
- `view.services`: 7.7ms
- `view.cronjobs`: 7.4ms
- `view.statefulsets`: 7.3ms
- `view.pods`: 6.8ms
- `status`: 7.0ms

Key bottlenecks identified:
1. Sidebar cache invalidates on every cursor move (cursor is part of cache key)
2. Status bar rebuilds format strings unconditionally every frame
3. Pod metrics HashMap rebuilt every frame
4. Sidebar item counts Vec rebuilt every frame
5. Filter indices recomputed even when query unchanged
6. Column resolution rebuilt every frame
7. cluster_summary() computed fresh every frame

## What's Been Tried

(will be filled as experiments run)

## Dead Ends

(approaches that were tried and reverted)
