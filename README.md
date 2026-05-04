# KubecTUI

A keyboard-driven terminal UI for Kubernetes operations.

![Rust](https://img.shields.io/badge/rust-1.85+-orange)
![License](https://img.shields.io/badge/license-MIT-blue)
[![CI](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml/badge.svg)](https://github.com/kxxil01/Kubectui/actions/workflows/ci.yml)

## Install

```bash
# Homebrew
brew install kxxil01/tap/kubectui

# From source
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui && cargo build --release
./target/release/kubectui
```

Requires a kubeconfig at `~/.kube/config` (or `KUBECONFIG` env var).

## Getting started

1. Launch `kubectui` in a shell that already targets the cluster you want to inspect.
2. Press `?` for in-app keybinding help and `:` for the action palette.
3. Use `c` to switch kube context, `~` to switch namespace, and `/` to filter the current view.
4. Open detail with `Enter`, then use the detail actions or the bottom workbench to inspect and mutate safely.

Persistent app preferences are written to `~/.kube/kubectui-config.json`.

Optional command extensions load from:

```text
$XDG_CONFIG_HOME/kubectui/extensions.yaml
# or, if XDG_CONFIG_HOME is unset:
~/.config/kubectui/extensions.yaml
```

Minimal example:

```yaml
actions:
  - id: describe_pod
    title: Describe with kubectl
    resource_kinds: ["Pod"]
    aliases: ["describe pod"]
    mode: background
    command:
      program: kubectl
      args: ["describe", "pod", "$NAME", "-n", "$NAMESPACE"]

```

Native AI actions load from `~/.kube/kubectui-config.json`:

```json
{
  "namespace": "all",
  "ai": {
    "providers": [
      { "provider": "codex_cli" },
      { "provider": "claude_cli" }
    ]
  }
}
```

Supported AI providers: `open_ai`, `anthropic`, `claude_cli`, and `codex_cli`.
When multiple providers are configured, the action palette is the picker: choose entries like `Explain Failure (Codex CLI)` or `Explain Failure (Claude CLI)`.
Legacy `ai:` blocks in `extensions.yaml` are not supported.

## What it does

**Browse** 50 resource views across 9 sidebar groups — workloads, network, config, storage, Helm, Flux, RBAC, diagnostics, and custom resources. The sidebar auto-collapses to show only the active group.

**Operate** without leaving the terminal — exec into pods, launch ephemeral debug containers, open guarded node debug shells, port-forward, scale, restart, pause/resume/undo rollouts, rollback Helm releases, delete, edit YAML in `$EDITOR`, trigger CronJobs, cordon/drain nodes, reconcile Flux resources, and apply built-in resource templates.

**Inspect** with a bottom workbench that holds persistent tabs — YAML, drift diffs, rollout control, Helm history, decoded Secrets, timelines, pod/workload logs, exec sessions, extension output, AI analysis, port-forwards, relationship graphs, NetworkPolicy analysis, pod reachability, and traffic debugging.

**Monitor** via a dashboard with health gauges, CPU/memory utilization, namespace breakdown, top consumers, alerts, a Health Report, sanitizer findings, and Trivy-backed vulnerability summaries. Live watch streams keep 10 core resources updated in real-time.

## Release highlights

- Rollout Control Center with revision-aware inspection, pause/resume, restart, and undo for Deployments, StatefulSets, and DaemonSets
- Helm release history, computed-values diff, and in-app rollback confirmation
- Advanced log investigation with presets, regex mode, time windows, jump-to-time, structured JSON summaries, and request correlation
- Workspaces, banks, and configurable hotkeys for repeatable operator layouts
- Health Report, sanitizer findings, vulnerability center, NetworkPolicy analysis, traffic debug, and AI-assisted analysis workflows
- Resource drift view, ephemeral pod debugging, node debug shell, and built-in apply templates

## Key features

| Category | Details |
|----------|---------|
| **Resources** | 50 views, CRD instances, Helm v3 releases, FluxCD, Health Report, vulnerability center, and extensions |
| **Actions** | Exec, logs, debug containers, node debug, port-forward, scale, restart, rollout control, Helm rollback, delete, YAML edit, CronJob trigger/pause, traffic debug |
| **Workbench** | 17 tab types: History, YAML, Drift, Rollout, Helm, Decoded Secret, Timeline, Pod Logs, Workload Logs, Exec, Extension, AI, Port-Forward, Relations, NetPol, Reachability, Traffic |
| **Diagnostics** | Dashboard, Issue Center, Health Report, sanitizer rules, Trivy vulnerability summaries, NetworkPolicy intent analysis |
| **Search** | `/` filters any list live. `:` opens the action palette for fuzzy navigation, actions, workspaces, banks, templates, extensions, and AI workflows |
| **RBAC-aware** | Actions filtered by resource capability and cluster permissions; forbidden reads degrade gracefully |
| **Themes & UX** | Dark, Nord, Dracula, Catppuccin Mocha, Light; 3 icon modes; saved workspaces and hotkeys |
| **Resilience** | Connection health indicator, backoff on failures, staleness display, watch auto-reconnect |

## Keybindings

Press `?` in-app for the full list. Summary:

### Navigation

| Key | Action |
|-----|--------|
| `j`/`k` or `↑`/`↓` | Move up/down |
| `Enter` | Select item / open detail |
| `Tab` / `Shift+Tab` | Next / previous view |
| `Esc` | Close current overlay or detail |

### Global

| Key | Action |
|-----|--------|
| `/` | Search (filters as you type) |
| `:` | Action palette (fuzzy search for views + actions) |
| `~` | Namespace picker (type to filter, arrows to navigate) |
| `c` | Context switcher (type to filter, arrows to navigate) |
| `r` | Refresh data |
| `T` | Cycle theme |
| `I` | Cycle icon mode |
| `b` | Toggle workbench |
| `H` | Action history |
| `W` | Save current workspace |
| `{` / `}` | Previous / next saved workspace |
| `Ctrl+y` / `Y` | Copy name / namespace+name |
| `Esc then Enter` | Quit |

### Resource actions

Press `Enter` on a resource, then:

| Key | Action | Resources |
|-----|--------|-----------|
| `y` | YAML | All |
| `l` / `L` | Logs | Pods, workloads, Jobs; CronJob detail history |
| `x` | Exec/shell | Pods |
| `f` | Port-forward | Pods |
| `s` | Scale | Deployments, StatefulSets |
| `R` | Rollout restart / Flux reconcile | Workloads / FluxCD |
| `O` | Rollout control center | Deployments, StatefulSets, DaemonSets |
| `h` | Helm history / rollback | Helm releases |
| `D` | Drift / drain | Most resources / Nodes |
| `g` | Debug shell | Pods / Nodes |
| `C` | Reachability check | Pods |
| `t` | Traffic debug | Pods, Services, Endpoints, Ingresses |
| `d` | Delete | Supported resources |
| `e` | Edit in `$EDITOR` | When YAML is loaded |
| `w` | Relationships | Relationship-capable resources |
| `B` | Bookmark | All |
| `o` | Decoded view | Secrets |
| `p` | Probe inspector | Pods |
| `T` / `S` | Trigger / Pause | CronJobs |

### Workbench

| Key | Action |
|-----|--------|
| `,` / `.` | Switch tabs |
| `Ctrl+W` | Close tab |
| `z` | Maximize |
| `Ctrl+↑`/`↓` | Resize |

### Log viewer

| Key | Action |
|-----|--------|
| `g` / `G` | Top / bottom |
| `f` | Follow mode |
| `P` | Previous logs (crashed containers) |
| `t` | Timestamps |
| `R` | Regex mode |
| `W` | Time window |
| `T` | Jump to timestamp |
| `C` | Request-id correlation |
| `J` | Structured JSON summary |
| `M` | Save current preset |
| `[` / `]` | Previous / next saved preset |
| `/` then `n`/`N` | Search + next/prev match |
| `y` / `S` | Copy / export to file |

## CLI

```
kubectui [OPTIONS]

  --theme <name>          dark | nord | dracula | catppuccin | light
  --profile-render        Enable render profiling
  --profile-output <dir>  Profile output directory
  -V, --version           Version
  -h, --help              Help
```

## Tips

- `:` discovers all available actions for the current resource — no need to memorize keys
- `/` search is live — filters as you type, `Ctrl+U` clears
- Sidebar groups auto-collapse — only the active group is expanded. `Enter` on a header temporarily expands it
- `l` on a Deployment/StatefulSet aggregates logs from all its pods
- `P` in log viewer shows previous logs from crashed containers
- `W`, `{`, and `}` let you save and cycle named workspaces for common operator flows
- AI workflows are opt-in and palette-driven; configure them in `kubectui-config.json`
- Vulnerability data appears when Trivy Operator CRDs are available in the cluster
- CPU/memory metrics require `metrics-server` in the cluster
- Data auto-refreshes every 30s (configurable via `refresh_interval_secs` in config)

## License

MIT
