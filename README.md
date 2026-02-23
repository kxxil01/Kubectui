# KubecTUI

Terminal UI for Kubernetes with real-time cluster views and in-app operations.

![Rust](https://img.shields.io/badge/rust-1.93.1+-orange)
![Tests](https://img.shields.io/badge/tests-157%20passing-brightgreen)
![Coverage](https://img.shields.io/badge/coverage-73.8%25-brightgreen)

## Phase 4 (v0.3.0)

Phase 4 focuses on operator workflows: logs, port-forwarding, scaling, and probes, plus reliability hardening.

### What’s new

- Detail-view workflow actions for:
  - **Logs viewer** (open + scroll + follow state)
  - **Port-forward dialog** (form state + tunnel registry state)
  - **Scale dialog** (replica validation + submission state)
  - **Probe panel** (container probe state + navigation)
- Refresh pipeline hardening:
  - per-resource timeout guard
  - graceful degradation on partial API failures
  - stable handling for empty resource sets
- Test/quality baseline:
  - **157 tests passing**
  - **Coverage (llvm-cov): 75.41% regions / 73.82% lines**

---

## Installation

### Prerequisites

- Rust 1.93.1+
- `kubectl` configured
- Kubernetes cluster (KIND/minikube/managed cluster)

### Build

```bash
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui
cargo build --release
```

### Quick start

```bash
./target/release/kubectui --kubeconfig ~/.kube/config
```

---

## Keyboard Shortcuts

### Global

- `Tab` / `Shift+Tab` → switch top-level view
- `↑` / `↓` → move selection
- `Enter` → open selected resource detail
- `/` → search mode
- `r` → refresh
- `Esc` → close active overlay/detail (or quit from main view)
- `q` → quit

### Detail actions (Phase 4)

- `L` → open **Logs** workflow
- `F` / `f` → open **Port Forward** workflow
- `S` / `s` → open **Scale** workflow
- `P` / `p` → open **Probe** workflow

### Logs workflow

- `j` / `↓` → scroll down
- `k` / `↑` → scroll up
- `f` → toggle follow mode
- `Esc` → close logs workflow

---

## Testing & Coverage

```bash
# Unit/integration tests
cargo test --lib

# Coverage summary
cargo llvm-cov --summary-only

# Coverage HTML report
cargo llvm-cov --html --output-dir /tmp/phase4-coverage
```

Current baseline:
- **157 tests passed**
- **Region coverage: 75.41%**
- **Line coverage: 73.82%**

---

## Known Limitations (current v0.3.0 runtime)

Phase 4 state machines and keyboard routing are covered by tests. During live KIND manual checks, the following runtime gaps were observed and should be addressed in the next patch release:

- Logs/Port Forward/Scale/Probe overlays are not consistently rendered in live runtime sessions.
- Cluster-side effects for Scale and Port Forward were not observed from UI interaction in current binary.
- Probe open shortcut (`P`) is documented as intended workflow, but live open behavior may depend on pending runtime wiring.

If you hit these issues, use `kubectl` equivalents as a temporary fallback:

- Logs: `kubectl logs -n <ns> <pod> -f`
- Port-forward: `kubectl port-forward -n <ns> svc/<service> <local>:<remote>`
- Scale: `kubectl scale deploy/<name> -n <ns> --replicas=<n>`
- Probes: `kubectl describe pod -n <ns> <pod>`

---

## License

MIT
