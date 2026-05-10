#!/usr/bin/env bash
# Smoke tests for check-mimalloc-dev-dep.sh.

set -euo pipefail

SCRIPT_UNDER_TEST="$(pwd)/.github/scripts/check-mimalloc-dev-dep.sh"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

mkdir -p "$TMPDIR/crates/selene-graph" "$TMPDIR/crates/selene-persist" "$TMPDIR/crates/selene-gql"

for crate in selene-graph selene-persist selene-gql; do
  cat > "$TMPDIR/crates/$crate/Cargo.toml" <<'MANIFEST'
[package]
name = "dummy"

[dependencies]
thiserror = "2"

[dev-dependencies]
mimalloc.workspace = true
MANIFEST
done

"$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null

cat > "$TMPDIR/crates/selene-gql/Cargo.toml" <<'MANIFEST'
[package]
name = "dummy"

[dependencies]
mimalloc.workspace = true

[dev-dependencies]
criterion = "0.8"
MANIFEST

if "$SCRIPT_UNDER_TEST" "$TMPDIR" >/dev/null 2>&1; then
  echo "FAIL: did not reject normal mimalloc dependency" >&2
  exit 1
fi

echo "OK: mimalloc dev-dependency lint smoke tests passed."
