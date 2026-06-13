#!/usr/bin/env bash
# Smoke tests for check-bench-invocation.sh.

set -euo pipefail

SCRIPT_UNDER_TEST="$(pwd)/.github/scripts/check-bench-invocation.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

mkdir -p "$TMPDIR/.github/workflows" "$TMPDIR/scripts"

cat > "$TMPDIR/scripts/run-benches.sh" <<'SCRIPT'
cargo bench --no-run --workspace
SCRIPT

"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

cat > "$TMPDIR/.github/workflows/bad.yml" <<'SCRIPT'
name: bad
jobs:
  bench:
    steps:
      - run: cargo bench --workspace
SCRIPT

if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject workflow cargo bench --workspace" >&2
  exit 1
fi
rm "$TMPDIR/.github/workflows/bad.yml"

cat > "$TMPDIR/Makefile" <<'SCRIPT'
bench:
	cargo bench --bench single_graph -p selene-db-graph &
SCRIPT

if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject backgrounded cargo bench" >&2
  exit 1
fi

echo "OK: benchmark invocation lint smoke tests passed."
