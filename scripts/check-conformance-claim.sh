#!/usr/bin/env bash
# Bind one release-claim decision to an unchanged clean checkout.

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <40-lowercase-hex-sha> <iso_aligned|selected_profile>" >&2
  exit 2
fi

expected="$1"
claim="$2"
if [[ ! "$expected" =~ ^[0-9a-f]{40}$ ]]; then
  echo "expected revision must be 40 lowercase hexadecimal characters" >&2
  exit 2
fi
case "$claim" in
  iso_aligned|selected_profile) ;;
  *) echo "unknown claim: $claim" >&2; exit 2 ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
assert_unchanged() {
  [ "$(git rev-parse HEAD)" = "$expected" ] || { echo "checkout revision changed" >&2; return 1; }
  [ -z "$(git status --porcelain=v1 --untracked-files=all)" ] || { echo "checkout is dirty" >&2; return 1; }
}
assert_unchanged

temp_base="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
temp_base="$(cd "$temp_base" && pwd)"
case "$temp_base" in "$root"|"$root"/*) echo "temporary output must be outside the repository" >&2; exit 1;; esac
temp_dir="$(mktemp -d "$temp_base/selene-conformance.XXXXXX")"
trap 'rm -rf "$temp_dir"' EXIT

set +e
cargo run --locked -p selene-db-testing --bin selene-conformance -- \
  run --root "$root" --revision "$expected" --claim "$claim" --output "$temp_dir/result.json"
runner_status=$?
set -e
assert_unchanged
exit "$runner_status"
