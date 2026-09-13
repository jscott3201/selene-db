#!/usr/bin/env bash
# Verify THIRDPARTY.md is in sync with Cargo.lock by regenerating with
# cargo-about 0.9.2 and diffing. Drift indicates a dependency was added/changed
# without updating the attribution file, or attribution was hand-edited.

set -euo pipefail

SUPPORTED_VERSION="0.9.2"
INSTALL_COMMAND="cargo install cargo-about --version $SUPPORTED_VERSION --locked --features cli"

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "cargo-about $SUPPORTED_VERSION not found. Install with: $INSTALL_COMMAND" >&2
  exit 2
fi

FOUND_VERSION="$(cargo about --version 2>&1)"
if [ "$FOUND_VERSION" != "cargo-about $SUPPORTED_VERSION" ]; then
  echo "cargo-about $SUPPORTED_VERSION is required; found: $FOUND_VERSION" >&2
  echo "Install the supported version with: $INSTALL_COMMAND" >&2
  exit 2
fi

GENERATED=$(mktemp)
trap 'rm -f "$GENERATED"' EXIT

cargo about generate about.hbs | sed 's/[[:space:]]*$//' > "$GENERATED"

if ! diff -q THIRDPARTY.md "$GENERATED" >/dev/null 2>&1; then
  echo "FAIL: THIRDPARTY.md is out of sync with Cargo.lock."
  echo "Regenerate with: cargo about generate about.hbs | sed 's/[[:space:]]*$//' > THIRDPARTY.md"
  echo
  echo "Diff:"
  diff THIRDPARTY.md "$GENERATED" || true
  exit 1
fi

echo "OK: THIRDPARTY.md is in sync with Cargo.lock using cargo-about $SUPPORTED_VERSION."
