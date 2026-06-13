#!/usr/bin/env bash
# CONFORMANCE-00 guard (version-string honesty, CW-3).
#
# Forbids the version-locked error variant name `FeatureNotInV1_1` and stale
# version-locked "not supported in vX.Y" *user-facing message* strings from
# re-appearing. The runtime "construct is ISO-legal but not yet implemented"
# error is `ExecutorError::FeatureNotSupportedYet`, and its `#[error(...)]`
# message is deliberately version-agnostic ("feature not yet supported: ...")
# so it does not go stale the moment a new release ships.
#
# Scope: all workspace Rust source (production + tests). Internal design
# prose elsewhere may legitimately reference a release; this gate targets only
# (a) the forbidden variant identifier and (b) the specific stale message
# substring "not supported in v" / "not yet supported in v" inside a string
# literal or `#[error(...)]` attribute.
set -uo pipefail

# (a) the forbidden variant identifier.
variant=$(git grep -nE 'FeatureNotInV1_1' -- ':(glob)crates/**/*.rs' 2>/dev/null || true)

# (b) stale version-locked phrasing inside an error/message string. The phrase
# "not (yet )?supported in v<digit>" is the stale shape CW-3 removed.
message=$(
  git grep -nE 'not (yet )?supported in v[0-9]' -- ':(glob)crates/**/*.rs' 2>/dev/null \
    | grep -vE ':[0-9]+: *//' \
    || true
)

fail=0
if [ -n "$variant" ]; then
  echo "$variant"
  echo
  echo "FORBIDDEN: the version-locked variant 'FeatureNotInV1_1' (CONFORMANCE-00 / CW-3)."
  echo "Use ExecutorError::FeatureNotSupportedYet instead."
  fail=1
fi
if [ -n "$message" ]; then
  echo "$message"
  echo
  echo "FORBIDDEN: a version-locked 'not supported in vX' user-facing string (CW-3)."
  echo "Degrade the message to version-agnostic phrasing (e.g. 'not yet supported')."
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "OK: no version-locked feature-not-supported variant or message strings."
