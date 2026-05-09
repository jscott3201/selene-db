#!/usr/bin/env bash
# Validate committed iai-callgrind baseline lock files.

set -euo pipefail

ROOT="${1:-.}"
BASELINE_DIR="$ROOT/benches/baselines"
expected="
selene-graph.iai.txt
selene-persist.iai.txt
selene-gql.iai.txt
"

for file in $expected; do
  path="$BASELINE_DIR/$file"
  if [ ! -s "$path" ]; then
    echo "FAIL: missing non-empty iai baseline file: $path" >&2
    exit 1
  fi
  if ! awk -F '\t' '
    /^#/ || NF == 0 { next }
    NF != 4 { exit 2 }
    $2 !~ /^[0-9]+$/ { exit 3 }
    $3 !~ /^[0-9]+$/ { exit 4 }
    $4 !~ /^[0-9]+$/ { exit 5 }
  ' "$path"; then
    echo "FAIL: invalid iai baseline format in $path" >&2
    echo "Expected: bench_name<TAB>instructions<TAB>l1_misses<TAB>ll_misses" >&2
    exit 1
  fi
done

echo "OK: iai baseline lock files are present and parseable."
