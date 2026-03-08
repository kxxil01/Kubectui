Run app: cargo run --release
Format: cargo fmt --all
Lint: cargo clippy --all-targets --all-features -- -D warnings
Test: cargo test --all-targets --all-features
Performance check: cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture
General shell utilities on Darwin: git, ls, cd, rg, fd, sed, awk, jq.