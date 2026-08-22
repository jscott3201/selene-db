#!/usr/bin/env bash
# Focused tests for the release-tag validator and publishing workflow wiring.

set -euo pipefail

SCRIPT_UNDER_TEST="$(pwd)/.github/scripts/check-release-tag.sh"
WORKFLOW="$(pwd)/.github/workflows/release.yml"

expect_pass() {
  local tag="$1"
  local version="$2"
  if ! "$SCRIPT_UNDER_TEST" "$tag" "$version" >/dev/null 2>&1; then
    printf 'FAIL: expected tag %s to accept workspace version %s\n' "$tag" "$version" >&2
    exit 1
  fi
}

expect_fail() {
  local description="$1"
  shift
  if "$SCRIPT_UNDER_TEST" "$@" >/dev/null 2>&1; then
    printf 'FAIL: accepted %s\n' "$description" >&2
    exit 1
  fi
}

require_workflow_line() {
  local marker="$1"
  if ! grep -Fq -- "$marker" "$WORKFLOW"; then
    printf 'FAIL: release workflow is missing: %s\n' "$marker" >&2
    exit 1
  fi
}

expect_pass "v2.0.0" "2.0.0"
expect_pass "v2.0.0-alpha.1" "2.0.0-alpha.1"
expect_pass "v2.7.3-rc.2+build.5" "2.7.3-rc.2+build.5"

expect_fail "missing arguments"
expect_fail "one argument" "v2.0.0-alpha.1"
expect_fail "the 1.x archive tag" "archive-v1-eol-2026-08-21" "2.0.0-alpha.1"
expect_fail "a v1 release tag" "v1.5.0" "1.5.0"
expect_fail "a malformed v2 tag" "v2.0" "2.0.0"
expect_fail "a leading-zero core version" "v2.00.0" "2.0.0"
expect_fail "a leading-zero prerelease identifier" "v2.0.0-alpha.01" "2.0.0-alpha.01"
expect_fail "a tag/workspace mismatch" "v2.0.0-alpha.1" "2.0.0-alpha.2"

require_workflow_line "- 'v2.*.*'"
require_workflow_line "release-policy:"
require_workflow_line "run: bash .github/scripts/check-release-tag.test.sh"
require_workflow_line "needs: release-policy"
require_workflow_line "bash .github/scripts/check-release-tag.sh \"\$GITHUB_REF_NAME\" \"\$workspace_version\""
require_workflow_line "needs: publish-crates"

policy_block="$(awk '
  /^  release-policy:/ { in_policy = 1; next }
  in_policy && /^  [[:alnum:]_-]+:/ { exit }
  in_policy { print }
' "$WORKFLOW")"
if grep -Eq '^    if:' <<<"$policy_block"; then
  echo "FAIL: release-policy must run on every triggered event, including tag pushes" >&2
  exit 1
fi

if [ "$(grep -Fc "startsWith(github.ref, 'refs/tags/v2.')" "$WORKFLOW")" -ne 2 ]; then
  echo "FAIL: publishing jobs must both require refs/tags/v2." >&2
  exit 1
fi

validator_line="$(grep -nF 'bash .github/scripts/check-release-tag.sh' "$WORKFLOW" | cut -d: -f1)"
publish_line="$(grep -nF 'cargo publish -p' "$WORKFLOW" | cut -d: -f1)"
if [ -z "$validator_line" ] || [ -z "$publish_line" ] || [ "$validator_line" -ge "$publish_line" ]; then
  echo "FAIL: actual tag validation must precede cargo publish" >&2
  exit 1
fi

echo "OK: release tag policy tests passed."
