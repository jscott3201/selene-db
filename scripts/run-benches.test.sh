#!/usr/bin/env bash
# Smoke test for run-benches.sh concurrent-bench detection.

set -euo pipefail

if SELENE_BENCH_FORCE_CONFLICT=1 scripts/run-benches.sh --profile quick --layer criterion --filter __nope__ >/dev/null 2>&1; then
  echo "FAIL: run-benches.sh did not detect an existing cargo bench process" >&2
  exit 1
fi

echo "OK: run-benches.sh detects concurrent cargo bench processes."
