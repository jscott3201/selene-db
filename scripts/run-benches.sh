#!/usr/bin/env bash
# Sequential benchmark runner. This is the sanctioned entry point for running
# benchmark binaries; direct `cargo bench --workspace` execution is forbidden
# because Cargo may dispatch multiple bench binaries concurrently.

set -euo pipefail

PROFILE="quick"
LAYER="both"
SAVE_BASELINE=""
FILTER=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --layer)
      LAYER="$2"
      shift 2
      ;;
    --save-baseline)
      SAVE_BASELINE="$2"
      shift 2
      ;;
    --filter)
      FILTER="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

case "$PROFILE" in
  quick|full|stress) ;;
  *)
    echo "ERROR: --profile must be quick, full, or stress" >&2
    exit 2
    ;;
esac

case "$LAYER" in
  criterion|iai|both) ;;
  *)
    echo "ERROR: --layer must be criterion, iai, or both" >&2
    exit 2
    ;;
esac

if [ "${SELENE_BENCH_FORCE_CONFLICT:-0}" = "1" ] || pgrep -f "cargo bench" 2>/dev/null | grep -v "$$" >/dev/null 2>&1; then
  echo "ERROR: detected another cargo bench process; benchmark execution is strictly sequential." >&2
  exit 1
fi

if ! command -v valgrind >/dev/null 2>&1; then
  case "$LAYER" in
    iai)
      echo "ERROR: --layer iai requires valgrind, which is not installed (macOS does not ship valgrind)." >&2
      echo "       Use --layer criterion on this platform, or run on a Linux host." >&2
      exit 1
      ;;
    both)
      echo "WARN: valgrind not found; running --layer criterion only (iai layer is Linux-only)." >&2
      LAYER="criterion"
      ;;
  esac
fi

export SELENE_BENCH_PROFILE="$PROFILE"

BENCHES="
selene-graph:single_graph:criterion
selene-graph:bulk_mutation:criterion
selene-graph:concurrent_read:criterion
selene-graph:bfs:criterion
selene-graph:iai_gates:iai
selene-persist:wal:criterion
selene-persist:snapshot:criterion
selene-persist:iai_gates:iai
selene-gql:parse:criterion
selene-gql:analyze:criterion
selene-gql:plan_optimize:criterion
selene-gql:iai_gates:iai
selene-algorithms-pack:algo_pack:criterion
selene-vector-pack:vector_pack:criterion
selene-vector:recall:criterion
selene-vector:quant_recall:criterion
selene-vector:ivfpq_recall:criterion
selene-vector:composition_replay:criterion
"

echo "==> Compiling bench binaries (parallel compilation is allowed)"
cargo bench --no-run --workspace --all-features

printf '%s\n' "$BENCHES" | while IFS=: read -r crate bench bench_layer; do
  [ -n "$crate" ] || continue
  if [ "$LAYER" != "both" ] && [ "$LAYER" != "$bench_layer" ]; then
    continue
  fi

  echo "==> [$bench_layer] $crate :: $bench (profile=$PROFILE)"
  args=(bench --bench "$bench" -p "$crate" --all-features)
  bench_args=()
  if [ -n "$SAVE_BASELINE" ]; then
    bench_args+=(--save-baseline "$SAVE_BASELINE")
  fi
  if [ -n "$FILTER" ]; then
    bench_args+=("$FILTER")
  fi
  if [ "${#bench_args[@]}" -gt 0 ]; then
    args+=(-- "${bench_args[@]}")
  fi
  cargo "${args[@]}"
done
