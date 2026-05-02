# Native AI Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move AI action discovery/configuration onto a native Kubectui path and add local Claude/Codex CLI providers.

**Architecture:** Introduce a native AI action registry separate from shell command extensions. Reuse existing AI prompt building, redaction, async execution, action history, and workbench rendering. Extensions become external command actions only; legacy `extensions.yaml.ai` support is removed.

**Tech Stack:** Rust, serde JSON/YAML, tokio `spawn_blocking`, `std::process::Command`, existing Ratatui workbench.

---

### Task 1: Native AI Model And Registry

**Files:**
- Create: `src/ai_actions.rs`
- Modify: `src/lib.rs`
- Modify: `src/extensions.rs`
- Modify: `src/app/config_io.rs`
- Modify: `src/app/mod.rs`
- Test: `src/ai_actions.rs`

- [x] **Step 1: Write registry types and tests**

Create native types:
- `AiProviderKind`: `OpenAi`, `Anthropic`, `ClaudeCli`, `CodexCli`
- `AiProviderConfig`: provider/model/api_key_env/endpoint/timeout/max tokens/temperature/command/args/action
- `AiActionConfig`: optional custom `Ask AI` action metadata
- `LoadedAiAction`: native palette/runtime action
- `AiActionRegistry`: lookup and resource filtering
- `validate_ai_actions()`: one configured `Ask AI` plus four default workflows

Run:
```bash
cargo test ai_actions -- --nocapture
```
Expected: native AI action tests pass.

- [x] **Step 2: Remove AI from extension registry**

Delete AI provider/action/workflow types from `src/extensions.rs` and make `extensions.yaml` command-only. Add native `ai` config to `~/.kube/kubectui-config.json`.

Run:
```bash
cargo test extension -- --nocapture
```
Expected: existing extension tests pass.

### Task 2: Palette And Runbook Wiring

**Files:**
- Modify: `src/ui/components/command_palette.rs`
- Modify: `src/main.rs`
- Modify: `src/runbooks.rs`
- Test: `src/ui/components/command_palette.rs`
- Test: `src/main_tests.rs`

- [x] **Step 1: Add native AI palette section source**

Palette keeps rendering as extension-like action entries for now, but source becomes native AI registry plus command extension registry.

Run:
```bash
cargo test command_palette_ai -- --nocapture
```
Expected: AI actions appear when configured.
Actual validation: `cargo test palette_enter_can_execute_native_ai_action -- --nocapture`

- [x] **Step 2: Runbooks execute native AI directly**

`LoadedRunbookStepKind::AiWorkflow` maps to `AppAction::ExecuteAi`, not `ExecuteExtension`.

Run:
```bash
cargo test runbook_ai -- --nocapture
```
Expected: runbook AI step does not require extension action lookup.
Actual validation: covered by `cargo test --all-targets --all-features`.

### Task 3: CLI Providers

**Files:**
- Modify: `src/ai.rs`
- Test: `src/ai.rs`

- [x] **Step 1: Add Claude CLI provider**

Use configured command or default `claude`, args default `["-p", "$PROMPT"]`. Feed rendered prompt into arg substitution. Parse structured JSON same as HTTP providers.

- [x] **Step 2: Add Codex CLI provider**

Use configured command or default `codex`, args default `["exec", "$PROMPT"]`. Parse structured JSON same as HTTP providers.

Run:
```bash
cargo test ai_cli -- --nocapture
```
Expected: CLI command construction tests pass without launching real CLIs.
Actual validation: `cargo test ai_cli_defaults_use_prompt_argument -- --nocapture` and `cargo test codex_cli_defaults_to_exec -- --nocapture`

### Task 4: Validation And Merge

Run:
```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

No render-path profile required unless UI rendering logic changes materially.
