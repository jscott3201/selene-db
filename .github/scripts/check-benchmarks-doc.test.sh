#!/usr/bin/env bash
# Smoke tests for check-benchmarks-doc.sh.

set -euo pipefail

SCRIPT_UNDER_TEST="$(pwd)/.github/scripts/check-benchmarks-doc.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

mkdir -p "$TMPDIR/scripts"

cat > "$TMPDIR/scripts/run-benches.sh" <<'SCRIPT'
REGISTRY="
selene-core|value_clone|0
selene-graph|single_graph|0
selene-gql|parse|1
"
SMOKE="
selene-graph|single_graph|node_fetch
"
SCRIPT

# Passes when every registered bench + the footprint section are present.
cat > "$TMPDIR/BENCHMARKS.md" <<'DOC'
# benchmarks
## Hardware footprint
Rows: value_clone, single_graph, parse.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

# Fails when a registered bench is undocumented.
cat > "$TMPDIR/BENCHMARKS.md" <<'DOC'
# benchmarks
## Hardware footprint
Rows: value_clone, single_graph.
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject doc missing the 'parse' bench" >&2
  exit 1
fi

# Fails when the Hardware footprint section is absent.
cat > "$TMPDIR/BENCHMARKS.md" <<'DOC'
# benchmarks
Rows: value_clone, single_graph, parse.
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject doc missing the Hardware footprint section" >&2
  exit 1
fi

# The SMOKE block must NOT be treated as the registry (single_graph already
# covered above; ensure parsing stops at the REGISTRY closing quote).
cat > "$TMPDIR/scripts/run-benches.sh" <<'SCRIPT'
REGISTRY="
selene-core|value_clone|0
"
SMOKE="
selene-graph|never_registered|node_fetch
"
SCRIPT
cat > "$TMPDIR/BENCHMARKS.md" <<'DOC'
# benchmarks
## Hardware footprint
Rows: value_clone.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

echo "OK: benchmarks-doc parity lint smoke tests passed."
