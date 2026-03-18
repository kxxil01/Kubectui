# kube 3.x Migration Record

## Status

- Completed on 2026-03-18.
- This file is a migration record, not the active roadmap.
- The canonical forward-looking plan now lives in `plan.md`.

## Baseline And Result

Pre-migration baseline:

- `kube = "0.94"` with `["client", "ws", "runtime", "gzip"]`
- `k8s-openapi = "0.22"` with `v1_30`
- `chrono = "0.4"` used across DTOs, state, timeline, alerts, and UI

Current shipped state:

- `kube = "3.0.1"` in `Cargo.toml`
- lockfile resolved to `kube v3.1.0`
- `k8s-openapi = "0.27.1"` with `v1_35`
- `jiff = "0.2"` with `serde`
- no standalone `chrono` dependency remains

## Relevant Version Breaks

### 0.92 → 0.94

- same `k8s-openapi 0.22`
- `default_backoff()` free function removal did not affect this repo

### 0.94 → 0.99

- `backoff` replaced by `backon`
- `json-patch` bumped to v4
- `rand` removed as dependency
- `k8s-openapi` bumped to `0.24`

### 0.99 → 1.0

- `CELSchema` renamed to `KubeSchema`
- `Event::into_iter_applied()` / `into_iter_touched()` removed
- `k8s-openapi` bumped to `0.25`

### 1.0 → 2.0

- schemars bumped to `1.0`
- MSRV raised
- Rust 2024 edition required
- `k8s-openapi` bumped to `0.26`

### 2.0 → 3.0

- Kubernetes timestamps moved from `chrono` to `jiff`
- `kube::Error::Api` moved from `ErrorResponse`-style matching to `Status`
- predicate filter API gained a config parameter
- subresource write methods now take serializable objects instead of raw bytes

## What Shipped

### Time Model

- Canonical time helpers live in `src/time.rs`
- App timestamps now use `jiff::Timestamp` end-to-end
- DTOs, state, timeline, alerts, detail rendering, benches, and tests were migrated

### Kubernetes Boundary

- `src/k8s/conversions.rs`, `src/k8s/events.rs`, and `src/k8s/client.rs` now use canonical app timestamps directly
- `kube::Error::Api` handling now uses boxed `Status` helpers such as `is_forbidden()` and `is_not_found()`
- generated API shape changes from `k8s-openapi 0.27` were applied where needed

### Watch Path

- cluster-version-aware `streaming_lists()` is enabled only for Kubernetes `v1.34+`
- `Namespace` now uses `metadata_watcher()` on the canonical watch path
- namespace polling now uses metadata-only list calls
- namespace status is derived canonically from metadata: `Active` vs `Terminating`
- watched resource stores suppress no-op DTO updates, identical reconnect relists, and already-missing deletes

### Client Request Path

- the canonical kube client now builds through `ClientBuilder`
- non-watch request traffic now uses kube's built-in `client::retry::RetryPolicy`
- transient `429`, `503`, and `504` responses back off and retry automatically
- watch streams keep their separate watch backoff behavior

## Notes On Remaining Optional kube 3.x Features

- `Api::namespace()` was not needed for this repo
- further raw-object predicate tuning is still possible, but the canonical watch layer already suppresses no-op DTO-visible updates
- additional `metadata_watcher()` adoption should only happen for surfaces that are truly metadata-only

## Verification Completed

- `cargo check`
- `cargo fmt --all`
- `cargo test --all-targets --all-features`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Validation Still Recommended

- verify behavior on a live cluster `< v1.34`
- verify behavior on a live cluster `>= v1.34`
- exercise context switch and namespace switch under transient API failures
- verify watch reconnect behavior and event/timeline timestamp correctness
- verify age columns and CronJob schedule behavior against real cluster time data
