# Refactoring & Improvement Plan

## Completed (2026-03-18)

### Refactoring — Total: -2,039 dedup lines + 2 major file splits, zero regressions

| PR | What | Lines |
|----|------|-------|
| #39 | Cache unification (11 caches), fetch macros (30 methods), metadata extraction (21 converters) | -659 |
| #40 | Column registry `col()`/`col_fixed()`/`col_hidden()` const fn helpers (23 arrays) | -845 |
| #41 | Unified `view_info()` — single source of truth for view key + columns | -18 |
| #42 | `render_table_frame()` shared helper — migrated all 29 views (35 render functions) | -517 |
| #43 | Extract key handling → `app/input.rs` (1262 lines). `app/mod.rs`: 3877→2630 (-32%) | split |
| #44 | Extract helpers → `event_handlers.rs` (652) + `detail_fetch.rs` (201). `main.rs`: 4667→3866 (-17%) | split |

### Performance Investigation
- Row virtualization: already implemented (`table_window()` in 31 views)
- Per-view cached formatted cells: already implemented (`DerivedRowsCache<T>` in 26 views)
- Filter caching: already implemented (`cached_filter_indices_with_variant` with MRU fast path)
- `contains_ci`: benchmarked first-byte rejection — slower (breaks SIMD vectorization)
- Render profiler: all views under 0.25ms p50, total render 225ms/1920 frames — no bottleneck

---

## Remaining Refactoring

### `app/mod.rs` (now 2630 lines) — further opportunities
- Group 101 `AppAction` variants into sub-enums (WorkbenchAction, NavigationAction, etc.)
- Extract bookmarks methods (~180 lines) to `app/bookmarks.rs`
- Extract workbench tab-opening methods (~168 lines) to `app/workbench_tabs.rs`

### `main.rs` (now 3866 lines) — `run_app()` is still 3118 lines
- Extract action dispatch match arms (~1600 lines) to action submodules
- Extract event loop branches (~1200 lines) to event handler functions
- Risk: high (must preserve biased select! semantics)

---

## Future Features

### Nerd Font Icon System
Rich resource icons for all 46 views using Nerd Font glyphs (e.g. `󰠰` kubernetes, `󱃲` pod, `󰒍` node).
- Default: plain text/ASCII (safe for all terminals)
- Optional: Nerd Font mode toggled via settings (`icon_mode: "nerd"` / `"emoji"` / `"plain"`)
- Graceful degradation: detect terminal capability or let user choose
- Reference: k9s, lazydocker, starship patterns
- Scope: new milestone after refactoring is complete
