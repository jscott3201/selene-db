#!/usr/bin/env bash
# Documentation drift check for exposed format/cap constants (#1086).
#
# Docs reference these constants BY NAME rather than copying their values,
# because the values have drifted before: the principal cap was documented as
# 254 bytes while the source said 4096, and the snapshot minor version was
# documented as 0 while the source said 5. A name survives a value change; a
# copied literal silently rots.
#
# This checks the half a name-reference cannot guarantee on its own: that every
# `selene_persist::NAME` a doc cites still EXISTS in the crate. That catches a
# rename or removal, which is the realistic failure mode once the values
# themselves are no longer duplicated.
set -euo pipefail

# Optional repo root argument so the companion .test.sh can point this at a
# fixture directory, matching the other check scripts.
repo_root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$repo_root"

docs_glob=(docs/*.md)
src_dir="crates/selene-persist/src"
status=0

# `[A-Z0-9_]+\b` matches SCREAMING_SNAKE only: against `selene_persist::WalWriter`
# the class stops at `W` and the word boundary then fails, so type names are not
# mistaken for constants.
referenced=$(grep -Eho 'selene_persist::[A-Z0-9_]+\b' "${docs_glob[@]}" 2>/dev/null \
  | sed 's/^selene_persist:://' | sort -u || true)

# No early `exit 0` when nothing is referenced: the denylist below must still
# run. A doc that copies a stale literal is precisely a doc that names no
# constant, so an early return here would disable the check on its main case.
if [[ -n "$referenced" ]]; then
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    if ! grep -rq "pub const ${name}\b" "$src_dir"; then
      echo "ERROR: docs reference selene_persist::${name}, but no 'pub const ${name}' exists in ${src_dir}." >&2
      status=1
    fi
  done <<< "$referenced"
fi

# Retired literals: values that were wrong in the docs and have been corrected.
# Re-introducing one means someone copied a stale value back in. Each entry is
# `pattern<TAB>explanation`; add a line when you correct a doc claim that a
# reader would have no way to spot as stale on its own.
#
# These are deliberately specific strings, not general number matching. A check
# that asserted "every number in performance.md appears in BENCHMARKS.md" was
# tried and rejected: it passes by coincidence (an unrelated `4.53 ns` elsewhere
# in the baseline satisfied a stale read-path row) and so provides false
# assurance rather than a guarantee.
retired_literals=(
  $'capped at 254 bytes\tthe 254-byte principal cap is stale (source uses MAX_PRINCIPAL_BYTES); reference the constant by name'
  $'WAL v2 replay\tthe WAL format is 3.0 as of #1108; a doc claiming v2 replay predates it'
  $'54× faster than per-entry\tthe batched-append ratio is ~59× against the re-measured 604.7 ms per-entry row'
)

for entry in "${retired_literals[@]}"; do
  pattern="${entry%%$'\t'*}"
  explanation="${entry#*$'\t'}"
  if grep -rnF -- "$pattern" "${docs_glob[@]}" >/dev/null 2>&1; then
    echo "ERROR: retired literal '${pattern}' is back in docs/ — ${explanation}." >&2
    status=1
  fi
done

if [[ $status -eq 0 ]]; then
  count=$(echo "$referenced" | grep -c . || true)
  echo "OK: all ${count} doc-referenced selene_persist constants exist in source."
fi

exit $status
