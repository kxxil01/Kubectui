# CLAUDE.md

This is a Rust TUI application for Kubernetes operations (ratatui + kube-rs).

## Quick Reference

- **Build**: `cargo build`
- **Test**: `cargo test --all-targets --all-features`
- **Lint**: `cargo clippy --all-targets --all-features -- -D warnings`
- **Format**: `cargo fmt --all`
- **Performance**: `cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture`
- **Release**: `./scripts/release.sh patch|minor|major`

## Key Paths

- `src/app.rs` — AppState, AppAction enum, key handling
- `src/main.rs` — Event loop, async session management
- `src/workbench.rs` — WorkbenchState, tab states (logs, exec, yaml, events)
- `src/state/mod.rs` — ClusterSnapshot, refresh pipeline
- `src/policy.rs` — DetailAction, ViewAction capability policies
- `src/columns.rs` — ColumnDef registries for 23 views
- `src/events/input.rs` — Action routing/application
- `src/k8s/` — Kubernetes client, exec, port-forward, scaling, logs
- `src/ui/` — View renderers, components, theme

## Rules

@AGENTS.md
