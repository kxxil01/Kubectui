# AGENTS.md

## Scope
This file governs `/Users/ilham/Developer/Kubectui` and all subdirectories.

## Primary Goals
1. Keep the app correct and stable (no functional regressions).
2. Improve render-path speed and responsiveness.
3. Keep UI/UX flow intact while optimizing.

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

## Tomorrow Plan (Performance Phase)

### Phase 1: Row Virtualization / Windowing
Build and render only visible rows instead of full filtered datasets.

Implementation plan:
1. Add a shared visible-window helper for table views:
   - Inputs: `total_items`, `selected_idx`, `content_height`.
   - Outputs: `start..end` visible range and padding metadata if needed.
2. Apply to high-cardinality views first:
   - `Pods`, `ReplicaSets`, `ReplicationControllers`, `Deployments`, `Services`, `Nodes`.
3. Preserve behavior:
   - Selected row always visible.
   - Scrollbar position uses full dataset length.
   - Search/filter semantics unchanged.
4. Add tests:
   - Top/middle/bottom selection windowing.
   - Very small terminal heights.
   - Empty and single-item views.

Acceptance criteria:
- No UI behavior regressions.
- Median `render` improves versus pre-virtualization state.

### Phase 2: Per-View Cached Formatted Cells
Cache expensive derived strings/cells keyed by `(view, query, snapshot_version)`.

Implementation plan:
1. Introduce per-view formatted-row caches:
   - Key: `(view, query, snapshot_version, data_fingerprint)`.
   - Value: preformatted/derived cell payloads.
2. Handle time-sensitive fields (`Age`) explicitly:
   - Use a minute-bucket invalidation or render-time lightweight update path.
3. Bound memory:
   - Limit entries per view or use simple LRU/eviction policy.
4. Keep logic centralized to avoid duplicated per-view cache behavior.

Acceptance criteria:
- Correct invalidation on snapshot/query changes.
- No stale data rendering.
- Net positive median improvement.

### Phase 3: Regression Cleanup (Targeted)
Address known lagging areas from profiling.

Targets:
1. `Service Accounts`
2. `Pods`
3. `Deployments`
4. Shared `header` path

Method:
- Profile each target path before/after micro-patches.
- Keep only patches with positive median delta.
- Revert changes that regress total or hotspot metrics.

Acceptance criteria:
- Targeted paths no longer regress in median comparison.
- Shared path (`header`) is neutral or improved.

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
- Branch naming: `fix/description`, `feat/description`, `chore/description`, `refactor/description`
- Use `gh pr create` to open PRs, `gh pr merge --squash --delete-branch` to merge.
- This ensures an accountable changelog via PR history.
- Use Conventional Commits (`feat:`, `fix:`, `chore:`, `refactor:`, `perf:`, `docs:`, `test:`).
- Keep commits scoped to one measurable objective.
- Ask before any destructive git action.
- Ask before force-push (never default to force).

