#!/usr/bin/env bash
# Registry↔doc parity lint: every bench bin registered in the run-benches.sh
# REGISTRY block must be named in BENCHMARKS.md, so a newly added bench can
# never ship undocumented (the historical expression_eval orphan). Fast-gate
# mirror of the release-gate nextest pin
# (crates/selene-testing/tests/benchmarks_md_pin.rs).

set -euo pipefail

ROOT="${1:-.}"
script="$ROOT/scripts/run-benches.sh"
doc="$ROOT/BENCHMARKS.md"

[ -f "$script" ] || { echo "FAIL: $script not found" >&2; exit 1; }
[ -f "$doc" ] || { echo "FAIL: $doc not found" >&2; exit 1; }

# Bench bin names from the REGISTRY="..." block only (the SMOKE block is also
# pipe-delimited but is not the authoritative registry).
benches="$(awk '
  /^REGISTRY="$/ { inreg = 1; next }
  inreg && /^"$/ { exit }
  inreg && /\|/ { split($0, p, "|"); if (p[2] != "") print p[2] }
' "$script")"

[ -n "$benches" ] || { echo "FAIL: no benches parsed from REGISTRY block in $script" >&2; exit 1; }

missing=0
while IFS= read -r bench; do
  [ -n "$bench" ] || continue
  if ! grep -qF -- "$bench" "$doc"; then
    echo "FAIL: BENCHMARKS.md is missing registered bench '$bench'" >&2
    missing=$((missing + 1))
  fi
done <<< "$benches"

if [ "$missing" -gt 0 ]; then
  echo "==> $missing registered bench(es) undocumented; add a row to BENCHMARKS.md." >&2
  exit 1
fi

if ! grep -qF "Hardware footprint" "$doc"; then
  echo "FAIL: BENCHMARKS.md must include the Hardware footprint section" >&2
  exit 1
fi

echo "OK: all $(printf '%s\n' "$benches" | grep -c .) registered benches documented in BENCHMARKS.md."
