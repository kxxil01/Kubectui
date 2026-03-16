#!/usr/bin/env bash
set -euo pipefail

# Run the render profiling test 5 times and report the median render total.
# Extracts the "render" span total from render-frame-summary.txt after each run.

PROFILE_DIR="target/profiles/tests"
SUMMARY="$PROFILE_DIR/render-frame-summary.txt"
RUNS=5
values=()

for i in $(seq 1 $RUNS); do
    cargo test --test performance profile_render_path_and_emit_reports -- --ignored --nocapture 2>/dev/null
    # Extract the render span total (e.g., "total=303.792ms")
    val=$(grep -E '^\- render ' "$SUMMARY" | sed -E 's/.*total=([0-9.]+)ms.*/\1/')
    values+=("$val")
    echo "Run $i: render=${val}ms"
done

# Compute median (sort and pick middle)
sorted=($(printf '%s\n' "${values[@]}" | sort -n))
median_idx=$(( RUNS / 2 ))
median="${sorted[$median_idx]}"

echo ""
echo "METRIC: render_total_ms=$median"
