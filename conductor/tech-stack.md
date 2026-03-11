# Tech Stack

## Language

- **Rust** (edition 2024, stable toolchain)

## Core Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| ratatui | latest | Terminal UI framework |
| kube-rs | 0.92 | Kubernetes API client |
| k8s-openapi | 0.22 (v1_30) | Kubernetes type definitions |
| tokio | latest | Async runtime |
| crossterm | latest | Terminal backend |

## Build & Test

- **Build:** `cargo build`
- **Test:** `cargo test --all-targets --all-features`
- **Lint:** `cargo clippy --all-targets --all-features -- -D warnings`
- **Format:** `cargo fmt --all`
- **Benchmarks:** Criterion (`benches/`)

## Infrastructure

- **CI:** GitHub Actions (fmt, clippy, test with kind cluster, build)
- **Release:** Tag-triggered (`v*`), 4-platform cross-compilation (x86_64/aarch64 x linux/macos)
- **Distribution:** GitHub Releases + Homebrew tap (`kxxil01/homebrew-tap`)

## Architecture

- Keyboard-first, workbench-backed TUI
- Async event loop with `tokio::select!`
- 23 resource views with column registries
- Capability-based detail/view action policies
