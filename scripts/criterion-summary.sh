#!/usr/bin/env bash
# Summarize Criterion JSON artifacts after a run.
#
# Usage:
#   scripts/criterion-summary.sh core_vector_wgpu_prototype/cpu_rayon_score_topk/q8x100000x1024
#   scripts/criterion-summary.sh --root /tmp/criterion --phase base <criterion-id>

set -euo pipefail

ROOT="target/criterion"
PHASE="new"

usage() {
  sed -n '2,7p' "$0"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --root requires a directory" >&2
        exit 2
      fi
      ROOT="$2"
      shift 2
      ;;
    --phase)
      if [ "$#" -lt 2 ]; then
        echo "ERROR: --phase requires new or base" >&2
        exit 2
      fi
      PHASE="$2"
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    --) shift; break ;;
    -*) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    *) break ;;
  esac
done

if [ "$#" -eq 0 ]; then
  echo "ERROR: provide at least one Criterion id" >&2
  usage >&2
  exit 2
fi

case "$PHASE" in
  new|base) ;;
  *) echo "ERROR: --phase must be new or base" >&2; exit 2 ;;
esac

extract_point_estimate() {
  local section="$1" file="$2"
  compact_json "$file" |
    sed -E "s/.*\"$section\":\{\"confidence_interval\":\{[^}]*\},\"point_estimate\":([^,}]+).*/\1/"
}

extract_array() {
  local name="$1" file="$2"
  compact_json "$file" |
    sed -E "s/.*\"$name\":\[([^]]*)\].*/\1/"
}

compact_json() {
  awk '{ gsub(/[[:space:]]/, ""); printf "%s", $0 }' "$1"
}

ns_to_ms() {
  awk -v ns="$1" 'BEGIN { printf "%.3f", ns / 1000000.0 }'
}

require_number() {
  local label="$1" value="$2"
  if ! awk -v value="$value" 'BEGIN {
    exit(value ~ /^[-+]?[0-9]+([.][0-9]+)?([eE][-+]?[0-9]+)?$/ ? 0 : 1)
  }'; then
    echo "ERROR: expected numeric $label, got: $value" >&2
    exit 1
  fi
}

sample_count() {
  awk -v values="$1" 'BEGIN { print split(values, parts, ",") }'
}

p95_sample_ns() {
  local iters_csv="$1" times_csv="$2"
  awk -v iters_csv="$iters_csv" -v times_csv="$times_csv" '
    BEGIN {
      n_iters = split(iters_csv, iters, ",")
      n_times = split(times_csv, times, ",")
      if (n_iters == 0 || n_iters != n_times) {
        exit 1
      }
      for (i = 1; i <= n_times; i++) {
        if (iters[i] <= 0) {
          exit 1
        }
        values[i] = times[i] / iters[i]
      }
      for (i = 1; i <= n_times; i++) {
        for (j = i + 1; j <= n_times; j++) {
          if (values[j] < values[i]) {
            tmp = values[i]
            values[i] = values[j]
            values[j] = tmp
          }
        }
      }
      idx = int(0.95 * n_times)
      if (idx < 0.95 * n_times) {
        idx += 1
      }
      if (idx < 1) {
        idx = 1
      }
      print values[idx]
    }
  '
}

printf "criterion_id\tsamples\tmedian_ms\tmean_ms\tstddev_ms\tp95_sample_ms\n"
for criterion_id in "$@"; do
  estimates="$ROOT/$criterion_id/$PHASE/estimates.json"
  sample="$ROOT/$criterion_id/$PHASE/sample.json"
  if [ ! -f "$estimates" ]; then
    echo "ERROR: missing estimates file: $estimates" >&2
    exit 1
  fi
  if [ ! -f "$sample" ]; then
    echo "ERROR: missing sample file: $sample" >&2
    exit 1
  fi

  median_ns="$(extract_point_estimate median "$estimates")"
  mean_ns="$(extract_point_estimate mean "$estimates")"
  stddev_ns="$(extract_point_estimate std_dev "$estimates")"
  iters_csv="$(extract_array iters "$sample")"
  times_csv="$(extract_array times "$sample")"
  samples="$(sample_count "$times_csv")"
  p95_ns="$(p95_sample_ns "$iters_csv" "$times_csv")"
  require_number "median point_estimate" "$median_ns"
  require_number "mean point_estimate" "$mean_ns"
  require_number "std_dev point_estimate" "$stddev_ns"
  require_number "p95 sample" "$p95_ns"

  printf "%s\t%s\t%s\t%s\t%s\t%s\n" \
    "$criterion_id" \
    "$samples" \
    "$(ns_to_ms "$median_ns")" \
    "$(ns_to_ms "$mean_ns")" \
    "$(ns_to_ms "$stddev_ns")" \
    "$(ns_to_ms "$p95_ns")"
done
