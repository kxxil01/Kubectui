#!/usr/bin/env bash
set -euo pipefail

RUNS="${PERF_GATE_RUNS:-5}"
OUT_DIR="${PERF_GATE_OUT_DIR:-target/perf-gate}"
SUMMARY_PATH="target/profiles/tests/render-frame-summary.txt"

RENDER_MAX_MS="${PERF_GATE_RENDER_MAX_MS:-4500}"
SIDEBAR_MAX_MS="${PERF_GATE_SIDEBAR_MAX_MS:-1100}"
HEADER_MAX_MS="${PERF_GATE_HEADER_MAX_MS:-260}"
STATUS_MAX_MS="${PERF_GATE_STATUS_MAX_MS:-180}"
PODS_MAX_MS="${PERF_GATE_PODS_MAX_MS:-240}"
REPLICASETS_MAX_MS="${PERF_GATE_REPLICASETS_MAX_MS:-230}"
REPLICATION_CONTROLLERS_MAX_MS="${PERF_GATE_REPLICATION_CONTROLLERS_MAX_MS:-230}"
SERVICE_ACCOUNTS_MAX_MS="${PERF_GATE_SERVICE_ACCOUNTS_MAX_MS:-230}"
DEPLOYMENTS_MAX_MS="${PERF_GATE_DEPLOYMENTS_MAX_MS:-180}"

METRICS=(
  render
  sidebar
  header
  status
  view.pods
  view.replicasets
  view.replication_controllers
  view.service_accounts
  view.deployments
)

metric_file() {
  local metric="$1"
  echo "$OUT_DIR/metric-$(echo "$metric" | tr '. ' '__').txt"
}

extract_total_ms() {
  local metric="$1"
  local summary_file="$2"
  awk -v metric="$metric" '
    $1 == "-" && $2 == metric {
      for (i = 1; i <= NF; i++) {
        if ($i ~ /^total=/) {
          value = $i
          sub(/^total=/, "", value)
          sub(/ms$/, "", value)
          print value
          exit
        }
      }
    }
  ' "$summary_file"
}

median_from_file() {
  local values_file="$1"
  sort -n "$values_file" | awk '
    {
      vals[NR] = $1
    }
    END {
      if (NR == 0) {
        exit 1
      }
      if (NR % 2 == 1) {
        print vals[(NR + 1) / 2]
      } else {
        printf "%.3f\n", (vals[NR / 2] + vals[(NR / 2) + 1]) / 2
      }
    }
  '
}

assert_threshold() {
  local name="$1"
  local value="$2"
  local max="$3"

  if awk -v value="$value" -v max="$max" 'BEGIN { exit !(value <= max) }'; then
    printf "PASS %-30s median=%8.3fms  max=%sms\n" "$name" "$value" "$max"
    return 0
  fi

  printf "FAIL %-30s median=%8.3fms  max=%sms\n" "$name" "$value" "$max" >&2
  return 1
}

mkdir -p "$OUT_DIR"
for metric in "${METRICS[@]}"; do
  : > "$(metric_file "$metric")"
done

for run in $(seq 1 "$RUNS"); do
  echo ">>> performance gate run ${run}/${RUNS}"
  cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture \
    >"$OUT_DIR/test-run-${run}.log" 2>&1

  if [[ ! -f "$SUMMARY_PATH" ]]; then
    echo "missing profiling summary at $SUMMARY_PATH" >&2
    exit 1
  fi

  cp "$SUMMARY_PATH" "$OUT_DIR/render-frame-summary-run-${run}.txt"

  for metric in "${METRICS[@]}"; do
    value="$(extract_total_ms "$metric" "$SUMMARY_PATH")"
    if [[ -z "$value" ]]; then
      echo "failed to parse metric '$metric' from $SUMMARY_PATH" >&2
      exit 1
    fi
    echo "$value" >> "$(metric_file "$metric")"
  done

done

render_median="$(median_from_file "$(metric_file render)")"
sidebar_median="$(median_from_file "$(metric_file sidebar)")"
header_median="$(median_from_file "$(metric_file header)")"
status_median="$(median_from_file "$(metric_file status)")"
pods_median="$(median_from_file "$(metric_file view.pods)")"
replicasets_median="$(median_from_file "$(metric_file view.replicasets)")"
replication_controllers_median="$(median_from_file "$(metric_file view.replication_controllers)")"
service_accounts_median="$(median_from_file "$(metric_file view.service_accounts)")"
deployments_median="$(median_from_file "$(metric_file view.deployments)")"

{
  echo "KubecTUI performance gate medians (${RUNS} runs)"
  echo "render=${render_median}ms"
  echo "sidebar=${sidebar_median}ms"
  echo "header=${header_median}ms"
  echo "status=${status_median}ms"
  echo "view.pods=${pods_median}ms"
  echo "view.replicasets=${replicasets_median}ms"
  echo "view.replication_controllers=${replication_controllers_median}ms"
  echo "view.service_accounts=${service_accounts_median}ms"
  echo "view.deployments=${deployments_median}ms"
} | tee "$OUT_DIR/median-summary.txt"

failures=0
assert_threshold "render" "$render_median" "$RENDER_MAX_MS" || failures=$((failures + 1))
assert_threshold "sidebar" "$sidebar_median" "$SIDEBAR_MAX_MS" || failures=$((failures + 1))
assert_threshold "header" "$header_median" "$HEADER_MAX_MS" || failures=$((failures + 1))
assert_threshold "status" "$status_median" "$STATUS_MAX_MS" || failures=$((failures + 1))
assert_threshold "view.pods" "$pods_median" "$PODS_MAX_MS" || failures=$((failures + 1))
assert_threshold "view.replicasets" "$replicasets_median" "$REPLICASETS_MAX_MS" || failures=$((failures + 1))
assert_threshold "view.replication_controllers" "$replication_controllers_median" "$REPLICATION_CONTROLLERS_MAX_MS" || failures=$((failures + 1))
assert_threshold "view.service_accounts" "$service_accounts_median" "$SERVICE_ACCOUNTS_MAX_MS" || failures=$((failures + 1))
assert_threshold "view.deployments" "$deployments_median" "$DEPLOYMENTS_MAX_MS" || failures=$((failures + 1))

if [[ "$failures" -gt 0 ]]; then
  echo "performance gate failed with $failures threshold violation(s)" >&2
  exit 1
fi

echo "performance gate passed"
