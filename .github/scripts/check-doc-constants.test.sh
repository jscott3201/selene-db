#!/usr/bin/env bash
# Smoke tests for check-doc-constants.sh.

set -euo pipefail

SCRIPT_UNDER_TEST="$(pwd)/.github/scripts/check-doc-constants.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

mkdir -p "$TMPDIR/docs" "$TMPDIR/crates/selene-persist/src"

cat > "$TMPDIR/crates/selene-persist/src/entry_header.rs" <<'SRC'
pub const MAX_PRINCIPAL_BYTES: usize = 4096;
SRC

# Passes when every referenced constant exists.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Capped at `selene_persist::MAX_PRINCIPAL_BYTES`.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

# Fails when a referenced constant does not exist (rename / removal).
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Capped at `selene_persist::MAX_PRINCIPAL_OCTETS`.
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject a doc citing a non-existent constant" >&2
  exit 1
fi

# Type names are not constants: `WalWriter` must not be read as `W`.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Open one with `selene_persist::WalWriter` and cap at
`selene_persist::MAX_PRINCIPAL_BYTES`.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

# The stale 254-byte principal literal must not come back.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
An optional principal (audit-trail actor; capped at 254 bytes).
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject the stale 254-byte principal literal" >&2
  exit 1
fi

# The retired WAL-version claim must not come back. The format is 3.0 as of
# #1108, so a doc describing "WAL v2 replay" is describing a format the engine
# no longer opens.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
| Full recovery | 24.75 ms | Snapshot reconciliation + WAL v2 replay. |
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject the retired 'WAL v2 replay' claim" >&2
  exit 1
fi

# The retired batched-append ratio must not come back.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Group-commit dominates; 54× faster than per-entry.
DOC
if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject the retired 54x batched-append ratio" >&2
  exit 1
fi

# A doc carrying the CORRECTED forms of both retired claims must pass. Without
# this, a denylist that matched too broadly (e.g. on "WAL v" or on "faster than
# per-entry") would look healthy while rejecting the fix as well as the defect.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
| Full recovery | 16.31 ms | Snapshot reconciliation + WAL 3.0 replay. |
Group-commit dominates; ~59× faster than per-entry.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

# No references at all is a pass, not a crash.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Nothing to see here.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

echo "OK: check-doc-constants.sh smoke tests passed."
