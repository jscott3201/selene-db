#!/usr/bin/env bash
# Keep repository ownership metadata pinned to the canonical GitHub home.
set -euo pipefail

root="${1:-$(git rev-parse --show-toplevel)}"
cd "$root"

owner_repo="jscott3201/selene-db"
home_url="https://github.com/${owner_repo}"
legacy_owner_repo="Aionforge-Labs""/selene-db"

require_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    printf '%s is missing required repository-home marker: %s\n' "$file" "$needle" >&2
    return 1
  fi
}

require_nightly_jobs_canonical() {
  local file=".github/workflows/nightly.yml"
  local missing
  missing="$(
    awk -v needle="github.repository == '${owner_repo}'" '
      /^jobs:[[:space:]]*$/ {
        in_jobs = 1
        next
      }

      in_jobs && /^[^[:space:]#][^:]*:/ {
        if (job != "" && !job_ok) {
          print job
        }
        in_jobs = 0
        job = ""
        job_ok = 0
        next
      }

      in_jobs && /^  [[:alnum:]_-]+:[[:space:]]*$/ {
        if (job != "" && !job_ok) {
          print job
        }
        job = $1
        sub(/:$/, "", job)
        job_ok = 0
        next
      }

      in_jobs && job != "" && index($0, needle) > 0 {
        job_ok = 1
      }

      END {
        if (in_jobs && job != "" && !job_ok) {
          print job
        }
      }
    ' "$file"
  )"

  if [ -n "$missing" ]; then
    printf '%s has nightly jobs without canonical repository guards for %s:\n' \
      "$file" "$owner_repo" >&2
    printf '%s\n' "$missing" >&2
    return 1
  fi
}

if matches="$(git grep -n --fixed-strings "$legacy_owner_repo" -- .)"; then
  printf 'Found stale repository owner references; replace with %s:\n' "$owner_repo" >&2
  printf '%s\n' "$matches" >&2
  exit 1
fi

require_contains Cargo.toml "repository = \"${home_url}\""
require_contains Cargo.toml "homepage = \"${home_url}\""
require_contains CHANGELOG.md "\`${owner_repo}\` namespace"
require_contains docs/getting-started.md "git clone ${home_url}.git"
require_contains crates/selene-testing/src/local_omlx/client.rs "\"${home_url}\""
require_contains .github/workflows/release.yml "github.repository == '${owner_repo}'"
require_contains .github/workflows/release.yml "github.com/${owner_repo}"
require_nightly_jobs_canonical
