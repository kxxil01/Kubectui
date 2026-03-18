# Refactoring & Improvement Plan

## Completed (2026-03-18)

### Refactoring — Total: -2,039 lines, zero regressions

| PR | What | Lines |
|----|------|-------|
| #39 | Cache unification (11 caches), fetch macros (30 methods), metadata extraction (21 converters) | -659 |
| #40 | Column registry `col()`/`col_fixed()`/`col_hidden()` const fn helpers (23 arrays) | -845 |
| #41 | Unified `view_info()` — single source of truth for view key + columns | -18 |
| #42 | `render_table_frame()` shared helper — migrated all 29 views (35 render functions) | -517 |

### Performance Investigation
- Row virtualization: already implemented (`table_window()` in 31 views)
- Per-view cached formatted cells: already implemented (`DerivedRowsCache<T>` in 26 views)
- Filter caching: already implemented (`cached_filter_indices_with_variant` with MRU fast path)
- `contains_ci`: benchmarked first-byte rejection — slower (breaks SIMD vectorization)
- Render profiler: all views under 0.25ms p50, total render 225ms/1920 frames — no bottleneck

---

## Remaining Refactoring

### Priority 1: Split `app/mod.rs` (3877 lines)
- Group 101 `AppAction` variants into sub-enums (WorkbenchAction, NavigationAction, etc.)
- Extract `handle_key_event` to `app/key_routing.rs`
- Risk: medium (central state machine, many callers)

### Priority 2: Split `main.rs` (4667 lines)
- Extract async task spawning patterns
- Separate event routing from state mutation
- Risk: high (event loop is the orchestration heart)

---

## Future Features

### Nerd Font Icon System
Rich resource icons for all 46 views using Nerd Font glyphs (e.g. `󰠰` kubernetes, `󱃲` pod, `󰒍` node).
- Default: plain text/ASCII (safe for all terminals)
- Optional: Nerd Font mode toggled via settings (`icon_mode: "nerd"` / `"emoji"` / `"plain"`)
- Graceful degradation: detect terminal capability or let user choose
- Reference: k9s, lazydocker, starship patterns
- Scope: new milestone after refactoring is complete
