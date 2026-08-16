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

# No references at all is a pass, not a crash.
cat > "$TMPDIR/docs/guide.md" <<'DOC'
Nothing to see here.
DOC
"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

echo "OK: check-doc-constants.sh smoke tests passed."
