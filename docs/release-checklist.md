# Release Checklist

## Preconditions

- worktree is clean
- branch changes have already merged to `main`
- local checkout is fast-forwarded to `origin/main`
- disposable local `kind` validation environment is available for smoke coverage

## Required validation

1. `cargo fmt --all`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-targets --all-features`
4. `cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture`
5. `scripts/kind_smoke.sh`

## Smoke coverage

The `kind` smoke lane currently validates:

- rollout pause / resume / undo against a live Deployment
- ephemeral debug container launch against a live Pod
- Helm history and rollback against a disposable local Helm release
- NetworkPolicy analysis and pod-to-pod connectivity verdicts against live cluster fixtures

Optional follow-up checks when credentials are present:

- AI provider smoke with `OPENAI_API_KEY` or Anthropic credentials
- release artifact sanity on a clean machine

## Release flow

1. Single one-shot flow (recommended):
   - `scripts/release.sh patch`
   - `scripts/release.sh minor`
   - `scripts/release.sh major`
   - `scripts/release.sh <exact-version>`

The script handles everything automatically: quality gate, release PR, direct merge, and tag publish.
