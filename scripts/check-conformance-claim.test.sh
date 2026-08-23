#!/usr/bin/env bash
# Process-seam tests for the clean exact-revision claim wrapper.

set -euo pipefail

script="$(pwd)/scripts/check-conformance-claim.sh"
temp="$(mktemp -d "${TMPDIR:-/tmp}/selene-claim-test.XXXXXX")"
trap 'rm -rf "$temp"' EXIT
mkdir "$temp/bin" "$temp/runner"

cat > "$temp/bin/git" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  "rev-parse HEAD") printf '%s\n' "$FAKE_HEAD" ;;
  "status --porcelain=v1 --untracked-files=all") printf '%s' "${FAKE_DIRTY:-}" ;;
  *) exit 97 ;;
esac
EOF
cat > "$temp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_CARGO_LOG"
exit "${FAKE_CARGO_STATUS:-0}"
EOF
chmod +x "$temp/bin/git" "$temp/bin/cargo"

sha="0123456789abcdef0123456789abcdef01234567"
export PATH="$temp/bin:$PATH" RUNNER_TEMP="$temp/runner" FAKE_HEAD="$sha"
export FAKE_CARGO_LOG="$temp/cargo.log"

expect_fail() {
  if "$script" "$@" >/dev/null 2>&1; then
    echo "FAIL: accepted invalid claim invocation: $*" >&2
    exit 1
  fi
}

expect_fail
expect_fail short iso_aligned
expect_fail "$sha" unknown
FAKE_HEAD="f${sha:1}" expect_fail "$sha" iso_aligned
FAKE_DIRTY="?? untracked" expect_fail "$sha" iso_aligned

FAKE_HEAD="$sha" FAKE_DIRTY="" "$script" "$sha" iso_aligned
grep -Fq -- "run --root $(pwd) --revision $sha --claim iso_aligned --output " "$FAKE_CARGO_LOG"
grep -Fq -- "/selene-conformance." "$FAKE_CARGO_LOG"
FAKE_CARGO_STATUS=1 expect_fail "$sha" selected_profile

echo "OK: conformance claim wrapper tests passed."
