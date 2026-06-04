#!/usr/bin/env bash
# Smoke tests for run-benches.sh guardrails and vector scale presets.

set -euo pipefail

if SELENE_BENCH_FORCE_CONFLICT=1 scripts/run-benches.sh --profile quick --filter __nope__ >/dev/null 2>&1; then
  echo "FAIL: run-benches.sh did not detect an existing cargo bench process" >&2
  exit 1
fi

dry_run="$(scripts/run-benches.sh --profile quick --bench single_graph --filter graph_exact_vector_scan --vector-scales large --dry-run)"
if ! grep -q "SELENE_VECTOR_BENCH_SCALES=250000,1000000" <<< "$dry_run"; then
  echo "FAIL: --vector-scales large did not set vector bench scales in dry-run output" >&2
  exit 1
fi
if ! grep -q "SELENE_VECTOR_REBUILD_BENCH_SCALES=250000,1000000" <<< "$dry_run"; then
  echo "FAIL: --vector-scales large did not set vector rebuild scales in dry-run output" >&2
  exit 1
fi

custom_dry_run="$(scripts/run-benches.sh --profile quick --bench vector_index_rebuild --vector-scales 10000,1000,10000 --dry-run)"
if ! grep -q "SELENE_VECTOR_BENCH_SCALES=1000,10000" <<< "$custom_dry_run"; then
  echo "FAIL: custom --vector-scales were not sorted and deduplicated" >&2
  exit 1
fi

recommended_dry_run="$(scripts/run-benches.sh --profile quick --bench vector_index_rebuild --filter graph_vector_index_recommended_rebuild/ivf --dry-run)"
if ! grep -q "SELENE_VECTOR_REBUILD_GROUP_FILTER=recommended_rebuild" <<< "$recommended_dry_run"; then
  echo "FAIL: recommended rebuild filter did not select the recommended_rebuild group" >&2
  exit 1
fi

stress_dry_run="$(scripts/run-benches.sh --profile quick --bench single_graph --vector-scales stress --dry-run)"
if ! grep -q "SELENE_VECTOR_BENCH_SCALES=1000,10000,50000,100000,250000" <<< "$stress_dry_run"; then
  echo "FAIL: --vector-scales stress did not mirror the stress profile scales" >&2
  exit 1
fi

if scripts/run-benches.sh --profile quick --bench single_graph --vector-scales 0,abc --dry-run >/dev/null 2>&1; then
  echo "FAIL: invalid --vector-scales value was accepted" >&2
  exit 1
fi

echo "OK: run-benches.sh guardrail and vector scale smoke tests passed."
