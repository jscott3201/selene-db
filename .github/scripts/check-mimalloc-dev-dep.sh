#!/usr/bin/env bash
# Verify mimalloc stays a bench-only dev-dependency for library crates.

set -euo pipefail

ROOT="${1:-.}"
violations=0

check_manifest() {
  local manifest="$1"
  local section=""
  local line_no=0

  [ -f "$manifest" ] || return 0

  while IFS= read -r line || [ -n "$line" ]; do
    line_no=$((line_no + 1))
    local stripped="${line%%#*}"

    if [[ "$stripped" =~ ^[[:space:]]*\[([^]]+)\][[:space:]]*$ ]]; then
      section="${BASH_REMATCH[1]}"
      continue
    fi

    if [[ "$stripped" =~ (^|[[:space:]])mimalloc([[:space:]._-]|=) ]]; then
      if [ "$section" != "dev-dependencies" ]; then
        echo "FAIL: mimalloc must remain in [dev-dependencies]: $manifest:$line_no"
        violations=$((violations + 1))
      fi
    fi
  done < "$manifest"
}

check_manifest "$ROOT/crates/selene-graph/Cargo.toml"
check_manifest "$ROOT/crates/selene-persist/Cargo.toml"
check_manifest "$ROOT/crates/selene-gql/Cargo.toml"

if [ "$violations" -gt 0 ]; then
  echo
  echo "Library crates must stay allocator-agnostic; bench binaries own mimalloc."
  exit 1
fi

echo "OK: mimalloc is limited to library crate dev-dependencies."
