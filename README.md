# KubecTUI

Terminal UI for Kubernetes with real-time cluster views and in-app operations.

![Rust](https://img.shields.io/badge/rust-1.93.1+-orange)
![Tests](https://img.shields.io/badge/tests-157%20passing-brightgreen)
![Coverage](https://img.shields.io/badge/coverage-73.8%25-brightgreen)

## v0.3.0 Highlights (Phase 4)

- ✅ End-to-end validation on KIND (`kubectui-dev`)
- ✅ Logs stream workflow integrated (`L` from detail view)
- ✅ Port-forward dialog + tunnel list (`f` from detail view)
- ✅ Deployment scaling dialog with validation (`s` from detail view)
- ✅ Probe panel and real-time probe updates
- ✅ Production hardening for data refresh:
  - per-resource timeout guard
  - graceful degradation on partial Kubernetes API failures
  - stable behavior on empty resource sets
- ✅ Test coverage increased above target:
  - **Lines: 73.80%**
  - **157 unit tests passing**

---

## Install & Run

```bash
git clone https://github.com/kxxil01/Kubectui.git
cd Kubectui
cargo build --release
./target/release/kubectui --kubeconfig ~/.kube/config
```

## Prerequisites

- Rust 1.93.1+
- `kubectl` configured
- Kubernetes cluster (KIND/minikube/real cluster)

---

## Keyboard Shortcuts

### Global

- `Tab` / `Shift+Tab` → Switch top-level view
- `↑` / `↓` → Move selection
- `/` → Search mode
- `Enter` → Open detail view
- `r` → Refresh
- `q` → Quit
- `Esc` → Close active modal / detail

### Detail View Actions

- `L` → Open Logs viewer
- `f` → Open Port Forward dialog
- `s` → Open Scale dialog
- `Esc` → Close active component

### Logs Viewer

- `j` / `↓` → Scroll down
- `k` / `↑` → Scroll up
- `f` → Toggle follow mode
- `Esc` → Close logs viewer

### Port Forward Dialog

- `Tab` / `Shift+Tab` → Next/previous field
- `Enter` → Create tunnel
- `F2` → Tunnel list mode
- `F1` → Back to create mode
- `d` / `Delete` → Stop selected tunnel
- `r` / `F5` → Refresh tunnel list
- `Esc` → Close dialog

### Scale Dialog

- `0-9` → Enter desired replicas
- `Backspace` → Remove digit
- `+` / `-` (or increment/decrement actions) → Adjust value
- `Enter` → Submit if valid
- `Esc` → Cancel

---

## E2E Testing (KIND)

### Manual Run Command

```bash
./target/release/kubectui --kubeconfig ~/.kube/config
```

### Validation Checklist

1. **Logs**
   - Navigate to a pod detail
   - Press `L`
   - Verify log panel opens and supports scroll/follow

2. **Port Forward**
   - Navigate to target detail
   - Press `f`
   - Enter namespace/pod/ports and create tunnel
   - Verify active tunnel appears in list mode

3. **Scaling**
   - Navigate to deployment detail
   - Press `s`
   - Change replicas and submit
   - Verify replica count updates in cluster

4. **Probes**
   - Open pod detail with probes
   - Verify probe health panel renders
   - Verify updates are reflected without crash

---

## Testing & Coverage

```bash
# Unit + integration test suite
cargo test

# Coverage summary
cargo llvm-cov --summary-only

# HTML coverage report
cargo llvm-cov --html
```

Current baseline:
- **157 tests passed**
- **Line coverage: 73.80%**

---

## Troubleshooting

### Cannot connect to Kubernetes

```bash
kubectl config current-context
kubectl cluster-info
kubectl get nodes
```

If this fails, fix kubeconfig/context first.

### Refresh shows partial data

KubecTUI now degrades gracefully on partial API failures. This can happen with restricted RBAC or transient API issues. Check:

```bash
kubectl auth can-i list nodes
kubectl auth can-i list pods --all-namespaces
kubectl auth can-i list services --all-namespaces
kubectl auth can-i list deployments --all-namespaces
```

### Slow or unstable clusters

Phase 4 added timeout guards to prevent indefinite refresh hangs. If data is delayed:

- retry with `r`
- verify API server responsiveness (`kubectl get pods -A`)
- check cluster resource pressure

### Empty resources / no services / no probes

This is supported. UI should remain stable and render empty states instead of crashing.

---

## License

MIT
