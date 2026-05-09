#!/usr/bin/env bash
# Lint runnable repo entry points for forbidden parallel benchmark invocation.

set -euo pipefail

ROOT="${1:-.}"
violations=0

check_file() {
  local file="$1"
  [ -f "$file" ] || return 0

  if grep -nE 'cargo[[:space:]]+bench([^[:alnum:]_-]|$).*--workspace|cargo[[:space:]]+bench([^[:alnum:]_-]|$).*&' "$file" >/tmp/selene-bench-lint.$$ 2>/dev/null; then
    echo "FAIL: forbidden cargo bench invocation in $file"
    cat /tmp/selene-bench-lint.$$
    violations=$((violations + 1))
  fi

  if grep -nE '(xargs[[:space:]].*-P|parallel[[:space:]])' "$file" >/tmp/selene-bench-lint.$$ 2>/dev/null \
    && grep -nE 'cargo[[:space:]]+bench' "$file" >/dev/null 2>&1; then
    echo "FAIL: possible parallel cargo bench wrapper in $file"
    cat /tmp/selene-bench-lint.$$
    violations=$((violations + 1))
  fi

  if [[ "$file" == "$ROOT"/.github/workflows/*.yml || "$file" == "$ROOT"/.github/workflows/*.yaml ]]; then
    if grep -nE 'cargo[[:space:]]+bench' "$file" >/tmp/selene-bench-lint.$$ 2>/dev/null; then
      echo "FAIL: workflow must invoke scripts/run-benches.sh instead of raw cargo bench in $file"
      cat /tmp/selene-bench-lint.$$
      violations=$((violations + 1))
    fi
  fi
}

for file in "$ROOT"/.github/workflows/*.yml "$ROOT"/.github/workflows/*.yaml "$ROOT"/Makefile "$ROOT"/scripts/*.sh; do
  case "$file" in
    "$ROOT"/scripts/run-benches.sh) continue ;;
    "$ROOT"/scripts/*.test.sh) continue ;;
  esac
  check_file "$file"
done

rm -f /tmp/selene-bench-lint.$$

if [ "$violations" -gt 0 ]; then
  echo
  echo "Use scripts/run-benches.sh so benchmark binaries run strictly sequentially."
  exit 1
fi

echo "OK: benchmark invocations use the sequential runner."
