# Refactoring & Performance Plan

## Completed (2026-03-18)

### Phase 1: Deduplication (PR #39) — -659 lines
- [x] Unified 11 derived-cell caches → shared `DerivedRowsCache<T>` + `cached_derived_rows()`
- [x] Replaced 30 fetch methods → `fetch_namespaced!` / `fetch_cluster!` macros
- [x] Added `extract_common_metadata()` helper for 21 conversion functions
- [x] Normalized all converters to consistent `"<unknown>"` / `"default"` defaults
- [x] 3 review passes, all clean

### Phase 2a: Column Registries (PR #40) — -845 lines
- [x] Added `col()`, `col_fixed()`, `col_hidden()` const fn helpers
- [x] Compressed 23 column arrays from 6-line structs to 1-line calls
- [x] Review verified all 148 column entries field-by-field

### Phase 2b: Unified View Registry (PR #41) — -18 lines
- [x] Merged `view_key()` + `columns_for_view()` into single `view_info()`
- [x] Single source of truth for AppView → (key, columns) mapping
- [x] Review verified all 48 variant mappings

### Performance Investigation
- [x] Row virtualization: **already implemented** — `table_window()` in 31 views
- [x] Per-view cached formatted cells: **already implemented** — `DerivedRowsCache<T>` in 26 views
- [x] Filter caching: **already implemented** — `cached_filter_indices_with_variant` with MRU fast path
- [x] `contains_ci` optimization: benchmarked first-byte rejection — **slower** (breaks SIMD vectorization)
- [x] Conclusion: filtering at 12µs/2000 pods is 0.09% of frame budget — no improvement needed

**Total savings: -1,522 lines, zero behavior regressions**

---

## Remaining Opportunities

### Priority 1: Generic View Render Template (~500-800 lines)
15+ workload views share ~80% render boilerplate:
- `table_window` computation, scrollbar, header construction, row styling
- Could extract shared `render_resource_table()` helper
- Risk: medium (touches many view files, render functions are the UI-critical path)

### Priority 2: Split `app/mod.rs` (3877 lines)
- Group 101 `AppAction` variants into sub-enums (WorkbenchAction, NavigationAction, etc.)
- Extract `handle_key_event` to `app/key_routing.rs`
- Risk: medium (central state machine, many callers)

### Priority 3: Split `main.rs` (4667 lines)
- Extract async task spawning patterns
- Separate event routing from state mutation
- Risk: high (event loop is the orchestration heart)
