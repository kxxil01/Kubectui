#!/usr/bin/env bash
set -euo pipefail

CLUSTER_NAME="${KUBECTUI_KIND_CLUSTER_NAME:-kubectui-smoke}"
SKIP_CLUSTER_CREATE="${KUBECTUI_KIND_SMOKE_REUSE_CONTEXT:-0}"
SKIP_HELM="${KUBECTUI_SKIP_HELM_SMOKE:-0}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

require_cmd kubectl
require_cmd cargo

CURRENT_CONTEXT="$(kubectl config current-context 2>/dev/null || true)"
if [[ "$CURRENT_CONTEXT" != kind-* ]]; then
  if [[ "$SKIP_CLUSTER_CREATE" == "1" ]]; then
    echo "current kubectl context is not a kind context and reuse was requested" >&2
    exit 1
  fi
  require_cmd kind
  if ! kind get clusters 2>/dev/null | grep -Fxq "$CLUSTER_NAME"; then
    kind create cluster --name "$CLUSTER_NAME"
  fi
  kubectl config use-context "kind-${CLUSTER_NAME}" >/dev/null
fi

if [[ "$SKIP_HELM" != "1" ]]; then
  require_cmd helm
fi

kubectl wait --for=condition=Ready nodes --all --timeout=180s
kubectl cluster-info

export KUBECTUI_KIND_SMOKE=1
cargo test --test kind_smoke -- --ignored --nocapture --test-threads=1
