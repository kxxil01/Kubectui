# KIND Cluster Testing Guide
**Date:** 2026-02-23 | **Version:** 0.1.0 MVP

This guide walks through testing KubecTUI against a local KIND (Kubernetes IN Docker) cluster.

---

## 📋 Prerequisites

```bash
# Check Rust installation
rustc --version    # Should be 1.93.1+
cargo --version

# Check Docker
docker version      # Should be running

# Check kubectl
kubectl version     # Should be installed

# Check KIND (install if needed)
kind version        # Should output "kind version X.X.X"
# If not installed: go install sigs.k8s.io/kind@latest
```

---

## 🚀 Setup

### Step 1: Create KIND Cluster

```bash
# Create a single-node cluster (perfect for testing)
kind create cluster --name kubectui-dev

# Output:
# Creating cluster "kubectui-dev" ...
# ...
# Set kubectl context to "kind-kubectui-dev"
```

### Step 2: Verify Cluster

```bash
# Check cluster is running
kubectl cluster-info --context kind-kubectui-dev

# Expected output:
# Kubernetes control plane is running at https://127.0.0.1:XXXXX
# CoreDNS is running at https://127.0.0.1:XXXXX/api/v1/...

# View nodes
kubectl get nodes --context kind-kubectui-dev

# Expected output:
# NAME                         STATUS   ROLES           AGE   VERSION
# kubectui-dev-control-plane   Ready    control-plane   2m    v1.35.0
```

### Step 3: Set Active Context

```bash
# Make it the default context
kubectl config use-context kind-kubectui-dev

# Verify
kubectl config current-context
# Output: kind-kubectui-dev
```

---

## 🧪 Test Scenarios

### Test 1: Empty Cluster

**Objective:** Verify KubecTUI handles minimal resources

```bash
# Start KubecTUI (with empty/system namespace only)
cargo run --release

# Expected behavior:
# ✅ Dashboard shows: 1 node (control-plane), 0 user pods
# ✅ Pods tab shows only kube-system pods
# ✅ No crashes or warnings
# ✅ Responsive to keyboard input

# Manual testing:
# - Tab through all tabs (Dashboard → Nodes → Pods → Services → Deployments)
# - Search for "kube" (should filter kube-system pods)
# - Select a pod and press Enter (detail view should show)
# - Press Esc (detail view should close)

# Exit: Press 'q'
```

### Test 2: Create Workloads

**Objective:** Verify KubecTUI displays deployed resources

```bash
# Terminal 1: Start KubecTUI
cargo run --release

# Terminal 2: Create test deployment
kubectl create namespace demo
kubectl apply -n demo -f - << 'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nginx-demo
  labels:
    app: web
spec:
  replicas: 3
  selector:
    matchLabels:
      app: web
  template:
    metadata:
      labels:
        app: web
    spec:
      containers:
      - name: nginx
        image: nginx:1.27
        ports:
        - containerPort: 80
        resources:
          requests:
            memory: "64Mi"
            cpu: "100m"
          limits:
            memory: "128Mi"
            cpu: "200m"
---
apiVersion: v1
kind: Service
metadata:
  name: nginx-service
spec:
  type: LoadBalancer
  selector:
    app: web
  ports:
  - port: 80
    targetPort: 80
EOF

# Watch KubecTUI update in real-time:
# ✅ Pods tab: 3 nginx pods appear in demo namespace
# ✅ Services tab: nginx-service shows LoadBalancer type
# ✅ Deployments tab: nginx-demo shows Ready: 3/3
# ✅ Dashboard: Pod count increases, shows running status
# ✅ No lag or UI freezes during updates

# Manual testing:
# - Search for "nginx" (should show 3 pods)
# - Navigate to Services tab
# - Select nginx-service, press Enter
# - Detail view should show: LoadBalancer, port 80, 3 endpoint pods
# - Verify Events section (should show Created/Started events)

# Terminal 2: Scale deployment
kubectl scale deployment nginx-demo -n demo --replicas=5

# Watch KubecTUI:
# ✅ Deployments tab: Ready updates from 3/3 → 5/5 (within 10 seconds)
# ✅ Pods tab: 5 nginx pods visible
# ✅ Pending pods yellow, Running pods green

# Terminal 2: Delete a pod (force crash)
kubectl delete pod -n demo -l app=web --grace-period=0

# Watch KubecTUI:
# ✅ Pods disappear
# ✅ New pods appear (replication controller replaces them)
# ✅ Restart count increases
# ✅ Dashboard alerts if any pod is in CrashLoopBackOff
```

### Test 3: Filter & Search

**Objective:** Verify filtering performance and accuracy

```bash
# From previous test (3+ nginx pods should exist)

# In KubecTUI - Pods tab:
# Press '/'
# Type "nginx" (should filter to show only nginx pods)
# ✅ Search highlights results
# ✅ Lag <300ms
# ✅ Filter updates as you type

# Press Backspace multiple times (clear search)
# ✅ All pods reappear
# ✅ Responsive

# Press Esc (exit search mode)
# ✅ Search field cleared

# Nodes tab:
# Press '/'
# Type "kubectui" (should match control-plane node)
# ✅ Shows: kubectui-dev-control-plane

# Type "worker" (should show 0 results on single-node cluster)
# ✅ Shows: "No nodes found"

# Services tab:
# Search "nginx" (should find nginx-service)
# ✅ Shows: nginx-service
```

### Test 4: Detail Inspector

**Objective:** Verify detail view accuracy and error handling

```bash
# Navigate to Pods tab (demo/nginx pods)
# Select first nginx pod
# Press Enter

# Detail view should show:
# ✅ Pod name: nginx-demo-XXXXX
# ✅ Namespace: demo
# ✅ Status: Running (Ready 1/1)
# ✅ Node: kubectui-dev-control-plane
# ✅ IP: 10.244.X.X (K8s internal)
# ✅ Container: nginx image (nginx:1.27)
# ✅ Resources: 100m CPU / 64Mi Memory requested, 200m/128Mi limited
# ✅ Ports: 80/TCP
# ✅ YAML section shows full resource YAML
# ✅ Events section shows recent pod events

# Test YAML truncation:
# If YAML is >10KB, verify "... (truncated)" appears

# Press Esc
# ✅ Detail view closes
# ✅ Back to pod list

# Try detail on a Node:
# Navigate to Nodes tab
# Select control-plane node
# Press Enter
# ✅ Shows node details: IP, roles, conditions, allocatable resources
# ✅ Conditions should show: Ready, MemoryPressure, DiskPressure, etc.
# ✅ RBAC-friendly (events shown if available, otherwise "unavailable")

# Try detail on Services/Deployments similarly
```

### Test 5: Tab Navigation

**Objective:** Verify smooth navigation between all views

```bash
# Rapid tab switching
# Press Tab 10 times quickly
# ✅ Dashboard → Nodes → Pods → Services → Deployments → Dashboard...
# ✅ No lag or stuttering
# ✅ All tabs render correctly

# Reverse: Press Shift+Tab 5 times
# ✅ Cycles backward correctly

# Navigation stress test:
# Alternate Tab and Shift+Tab rapidly for 30 seconds
# ✅ Performance remains consistent
# ✅ No memory leak (memory usage stable)
# ✅ No crashes
```

### Test 6: Real-time Updates

**Objective:** Verify auto-refresh and data sync

```bash
# Terminal 2: While KubecTUI is running
# Create a new deployment (auto-refresh should pick it up)

kubectl apply -n demo -f - << 'EOF'
apiVersion: apps/v1
kind: Deployment
metadata:
  name: redis-demo
spec:
  replicas: 1
  selector:
    matchLabels:
      app: cache
  template:
    metadata:
      labels:
        app: cache
    spec:
      containers:
      - name: redis
        image: redis:7-alpine
        ports:
        - containerPort: 6379
EOF

# Watch KubecTUI:
# ✅ Deployments tab: redis-demo appears within 10 seconds
# ✅ Pods tab: redis-demo pod appears
# ✅ No manual refresh needed (auto-refresh at 10s intervals)

# Terminal 2: Scale back the nginx deployment
kubectl scale deployment nginx-demo -n demo --replicas=1

# Watch KubecTUI:
# ✅ Deployments tab: Ready updates from 5/5 → 1/1
# ✅ Pods tab: 4 nginx pods disappear
# ✅ Dashboard pod count decreases
```

### Test 7: Error Handling

**Objective:** Verify graceful error handling

```bash
# Terminal 2: Simulate errors

# Test 1: RBAC restriction (if applicable to your cluster)
# kubectl delete clusterrolebinding kubeadmin  # Don't do this, just example
# KubecTUI should:
# ✅ Show connection error in status bar
# ✅ Not crash
# ✅ Provide clear error message

# Test 2: Restart the KIND cluster
kind delete cluster --name kubectui-dev
# KubecTUI should:
# ✅ Show "Cluster offline" or connection error
# ✅ Not crash
# ✅ Attempt to reconnect when cluster comes back

# Recreate the cluster
kind create cluster --name kubectui-dev
# KubecTUI should:
# ✅ Automatically reconnect (within 30 seconds)
# ✅ Show data when cluster is back

# Test 3: Large pod name
kubectl run "test-pod-with-very-very-very-long-name-that-exceeds-normal-limits-abcdefghijklmnopqrstuvwxyz" -n demo --image=nginx

# KubecTUI should:
# ✅ Display truncated name without crashing
# ✅ Show full name in detail view
```

### Test 8: Performance

**Objective:** Verify app stays responsive

```bash
# Create a test with many pods
kubectl apply -n demo -f - << 'EOF'
apiVersion: v1
kind: Pod
metadata:
  name: test-pod-1
spec:
  containers:
  - name: app
    image: alpine:latest
    command: ['sleep', '3600']
---
apiVersion: v1
kind: Pod
metadata:
  name: test-pod-2
spec:
  containers:
  - name: app
    image: alpine:latest
    command: ['sleep', '3600']
EOF

# (Repeat to create 50-100 pods if desired)

# Test KubecTUI performance:
# ✅ Dashboard loads: <2 seconds
# ✅ Tab switch: <500ms
# ✅ Search filter: <300ms (e.g., filter 100 pods by name)
# ✅ Detail view: <1 second
# ✅ Idle CPU: <2%
# ✅ Memory usage: <50MB

# Measure with system tools:
# In another terminal:
# watch 'ps aux | grep kubectui'  # Check memory, CPU
```

---

## 🎯 Manual Checklist

Use this checklist while testing:

### UI Rendering
- [ ] All 5 tabs render correctly
- [ ] Colors are visible and match schema (green/yellow/red)
- [ ] Tables have proper spacing and alignment
- [ ] Status indicators show correctly
- [ ] No text overflow or clipping

### Navigation
- [ ] Tab/Shift+Tab cycles through all views
- [ ] Arrow keys select items in lists
- [ ] Search mode filters correctly
- [ ] Detail view opens and closes smoothly
- [ ] Modal doesn't interfere with background

### Data Accuracy
- [ ] Pod counts match `kubectl get pods`
- [ ] Node status matches `kubectl get nodes`
- [ ] Service types display correctly
- [ ] Deployment replicas show correct status
- [ ] Ages/timestamps are accurate

### Performance
- [ ] Initial load <2 seconds
- [ ] Tab switch responsive (<500ms)
- [ ] Search smooth and fast (<300ms)
- [ ] Auto-refresh every ~10 seconds
- [ ] No lag during heavy typing

### Error Handling
- [ ] RBAC errors don't crash
- [ ] Disconnection shows error (not panic)
- [ ] Invalid resources handled gracefully
- [ ] Long names truncated properly
- [ ] Unicode in pod names works

### Edge Cases
- [ ] Empty cluster: no crashes
- [ ] Many pods (100+): still responsive
- [ ] Rapid navigation: no memory leaks
- [ ] Rapid filtering: no lag
- [ ] Cluster restart: auto-reconnect

---

## 📊 Expected Results

After running all tests, you should observe:

✅ **71/71 tests pass** (run `cargo test`)  
✅ **All 8 scenarios complete successfully**  
✅ **No crashes or panics**  
✅ **Responsive UI throughout**  
✅ **Accurate data display**  
✅ **Graceful error handling**  
✅ **Performance meets targets**  

---

## 🐛 Known Issues

- **Pods initially "Pending":** KIND takes a few seconds to schedule pods (normal)
- **Node "NotReady":** May take 30-60 seconds for CNI to initialize (normal during startup)
- **RBAC restrictions:** Some clusters may not allow event reading (gracefully handled)
- **Large datasets (1000+ pods):** UI may need optimization (Phase 2)

---

## 🧹 Cleanup

After testing, clean up:

```bash
# Delete the test deployment
kubectl delete namespace demo

# Delete the KIND cluster
kind delete cluster --name kubectui-dev

# Verify cleanup
kind get clusters  # Should show empty or other clusters
```

---

## 📝 Test Results Template

Use this to document your test run:

```
Date: 2026-02-23
Tester: [Your Name]
Platform: [OS, Kubernetes version]
KubecTUI Version: 0.1.0

Test 1 (Empty Cluster):        ✅ PASS
Test 2 (Create Workloads):     ✅ PASS
Test 3 (Filter & Search):      ✅ PASS
Test 4 (Detail Inspector):     ✅ PASS
Test 5 (Tab Navigation):       ✅ PASS
Test 6 (Real-time Updates):    ✅ PASS
Test 7 (Error Handling):       ✅ PASS
Test 8 (Performance):          ✅ PASS

Overall Result: ✅ ALL TESTS PASSED

Notes:
- [Any observations or issues found]
- [Performance metrics if tested]
- [Suggestions for improvement]
```

---

## 🚀 Next Steps

After successful testing:

1. **Phase 2 Features:**
   - Pod logs viewer
   - Port forwarding
   - Resource scaling
   - Health diagnostics

2. **Performance Optimization:**
   - Large dataset handling (10k+ pods)
   - Memory profiling
   - UI rendering optimization

3. **Real Cluster Testing:**
   - Test against minikube
   - Test against managed K8s (EKS, GKE, AKS)
   - Test with RBAC-restricted clusters

---

**Happy testing! Report any issues on GitHub.** 🎉

*Last updated: 2026-02-23 | Version: 0.1.0*
