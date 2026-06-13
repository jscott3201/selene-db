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

compile_only_dry_run="$(scripts/run-benches.sh --profile quick --bench vector_graph_retrieval --filter graph_vector_omlx_embedding_pressure --compile-only --dry-run)"
if ! grep -q "cargo bench -p selene-db-algorithms --bench vector_graph_retrieval --no-run" <<< "$compile_only_dry_run"; then
  echo "FAIL: --compile-only dry-run did not resolve to a cargo bench --no-run invocation" >&2
  exit 1
fi
if grep -q " -- graph_vector_omlx_embedding_pressure" <<< "$compile_only_dry_run"; then
  echo "FAIL: --compile-only dry-run leaked a Criterion filter into the compile invocation" >&2
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

ivf_rebuild_target_dry_run="$(scripts/run-benches.sh --profile quick --bench vector_index_rebuild --filter graph_vector_index_ivf_target_centroid_rebuild --dry-run)"
if ! grep -q "SELENE_VECTOR_REBUILD_GROUP_FILTER=ivf_target_centroids" <<< "$ivf_rebuild_target_dry_run"; then
  echo "FAIL: IVF target-centroid rebuild filter did not select the ivf_target_centroids group" >&2
  exit 1
fi

ivf_target_dry_run="$(scripts/run-benches.sh --profile quick --bench vector_ivf_pressure --filter graph_ivf_target_centroids --dry-run)"
if ! grep -q "SELENE_VECTOR_IVF_PRESSURE_GROUP_FILTER=target_centroids" <<< "$ivf_target_dry_run"; then
  echo "FAIL: IVF target-centroid filter did not select the target_centroids group" >&2
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

criterion_tmp="$(mktemp -d)"
trap 'rm -rf "$criterion_tmp"' EXIT
mkdir -p "$criterion_tmp/core/example/q1/new"
cat > "$criterion_tmp/core/example/q1/new/estimates.json" <<'JSON'
{"mean":{"confidence_interval":{"confidence_level":0.95,"lower_bound":2000000.0,"upper_bound":3000000.0},"point_estimate":2500000.0,"standard_error":1.0},"median":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1000000.0,"upper_bound":3000000.0},"point_estimate":2000000.0,"standard_error":1.0},"median_abs_dev":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1.0,"upper_bound":1.0},"point_estimate":1.0,"standard_error":1.0},"slope":null,"std_dev":{"confidence_interval":{"confidence_level":0.95,"lower_bound":1.0,"upper_bound":1.0},"point_estimate":500000.0,"standard_error":1.0}}
JSON
cat > "$criterion_tmp/core/example/q1/new/sample.json" <<'JSON'
{"sampling_mode":"Flat","iters":[1.0,2.0,3.0,4.0],"times":[1000000.0,4000000.0,9000000.0,16000000.0]}
JSON
criterion_summary="$(scripts/criterion-summary.sh --root "$criterion_tmp" core/example/q1)"
if ! grep -q $'core/example/q1\t4\t2.000\t2.500\t0.500\t4.000' <<< "$criterion_summary"; then
  echo "FAIL: criterion-summary.sh did not report expected summary values" >&2
  echo "$criterion_summary" >&2
  exit 1
fi

echo "OK: run-benches.sh guardrail and vector scale smoke tests passed."
