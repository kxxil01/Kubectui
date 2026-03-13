# Migration Plan: kube 0.92 → 3.0.1

## Current State

- `kube = "0.94"` features: `["client", "ws", "runtime", "gzip"]`
- `k8s-openapi = "0.22"` feature: `v1_30`
- `chrono = "0.4"` — 185 occurrences across 21 files

## Target State

- `kube = "3"` features: `["client", "ws", "runtime", "gzip"]`
- `k8s-openapi = "0.27"` feature: `v1_35`
- `jiff = "0.2"` features: `["serde"]`
- `chrono` — keep as standalone dep if needed for non-k8s logic, otherwise remove

## Breaking Changes by Version

### 0.92 → 0.94 (zero code changes)

- Same k8s-openapi 0.22
- Adds `DeserializeGuard`, `Resource` derive macro
- Standalone `default_backoff()` fn removed in 0.93 (we use the trait method — unaffected)

### 0.94 → 0.99

- `backoff` crate replaced with `backon` (unmaintained dep swap)
- `json-patch` bumped to v4
- `rand` removed as dependency
- k8s-openapi bumped: 0.22 → 0.24

### 0.99 → 1.0.0

- `CELSchema` renamed to `KubeSchema` (no impact)
- `Event::into_iter_applied()` / `into_iter_touched()` removed (we don't use these)
- k8s-openapi bumped: 0.24 → 0.25

### 1.0.0 → 2.0.0

- schemars bumped to 1.0 (only matters for CRD derives — not used)
- MSRV raised to 1.85.0
- Rust edition 2024 required (we already use edition 2024)
- k8s-openapi bumped: 0.25 → 0.26

### 2.0.0 → 3.0.0 (biggest impact)

1. **chrono → jiff**: k8s-openapi 0.27 uses `jiff::Timestamp` for all k8s timestamps
2. **ErrorResponse → Status**: `kube::Error::Api(ErrorResponse { code, .. })` becomes `kube::Error::Api(s) if s.is_forbidden()`
3. **predicate_filter** gains a config parameter (we don't use predicates yet)
4. **Subresource write methods** take `&K: Serialize` instead of `Vec<u8>`

## Migration Steps

### Step 1: Bump to 0.94 (safe, zero changes)

```toml
kube = { version = "0.94", features = ["client", "ws", "runtime", "gzip"] }
```

### Step 2: Bump to 0.99

```toml
kube = { version = "0.99", features = ["client", "ws", "runtime"] }
k8s-openapi = { version = "0.24", features = ["v1_32"] }
```

Audit for any `backoff` or `json-patch` direct usage.

### Step 3: Bump to 2.0

```toml
kube = { version = "2", features = ["client", "ws", "runtime"] }
k8s-openapi = { version = "0.26", features = ["v1_34"] }
```

Check MSRV ≥ 1.85.0, confirm edition 2024.

### Step 4: chrono → jiff migration + bump to 3.0

```toml
kube = { version = "3", features = ["client", "ws", "runtime"] }
k8s-openapi = { version = "0.27", features = ["v1_35"] }
jiff = { version = "0.2", features = ["serde"] }
```

#### Affected files (chrono usage)

| File | Occurrences | Notes |
|------|-------------|-------|
| `src/k8s/client.rs` | 57 | Timestamp access on all fetched resources |
| `src/k8s/dtos.rs` | 40 | `DateTime<Utc>` fields on all DTOs |
| `src/main.rs` | 15 | `Utc::now()` for timers, status messages |
| `src/k8s/events.rs` | 10 | Event timestamps |
| `src/cronjob.rs` | 9 | Next schedule computation |
| `src/timeline.rs` | 7 | Timeline entry timestamps |
| `src/state/issues.rs` | 6 | Age-based issue detection |
| `src/ui/mod.rs` | 6 | Age formatting in views |
| `src/state/alerts.rs` | 5 | Alert timestamps |
| `src/action_history.rs` | 5 | Action log timestamps |
| Others (11 files) | 25 | Various timestamp usage |

#### Migration strategy for chrono → jiff

**Option A: Full jiff migration** — Replace all `DateTime<Utc>` with `jiff::Timestamp`, `Utc::now()` with `jiff::Timestamp::now()`, duration math with jiff equivalents. Clean but large diff.

**Option B: Boundary conversion** — Keep chrono for internal app logic. Convert k8s-openapi `jiff::Timestamp` to `chrono::DateTime<Utc>` at the DTO boundary (in `src/k8s/client.rs` and `src/k8s/conversions.rs`). Smaller diff, two timestamp libs in deps.

**Recommended: Option A** — One timestamp library, no conversion overhead, cleaner long-term.

#### ErrorResponse → Status migration

Grep for `ErrorResponse` and `kube::Error::Api` patterns. Replace code-based matching with `Status` helper methods (`is_forbidden()`, `is_not_found()`, etc.).

## Watcher API — No Changes Required

The watch API is stable across all versions:
- `watcher::watcher(api, Config::default())` — unchanged
- `Event::{Init, InitApply, InitDone, Apply, Delete}` — unchanged
- `WatchStreamExt::default_backoff()` — unchanged
- `Api::namespaced()` / `Api::all()` — unchanged
- `ResourceExt::uid()` — unchanged

## Useful New Features to Adopt

- **`metadata_watcher()`** — lower IO for resources where only metadata is needed
- **`StreamingList` strategy** — avoids paginated initial list (K8s 1.27+)
- **Predicate filters** — skip redundant reconciles on status-only changes
- **`Api::namespace()` accessor** (2.0+)
- **Client `RetryPolicy`** (3.0+) — built-in exponential backoff for client requests

## Verification

1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets --all-features`
4. Manual test: connect to cluster, verify all views render correctly
5. Manual test: context switch, namespace switch, watch reconnect
6. Manual test: verify age columns display correctly after jiff migration
