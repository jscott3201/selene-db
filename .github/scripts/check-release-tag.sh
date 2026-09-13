#!/usr/bin/env bash
# Authorize a release tag against the version declared by the workspace.

set -euo pipefail

usage() {
  printf 'usage: %s <tag> <workspace-version>\n' "${0##*/}" >&2
}

if [ "$#" -ne 2 ]; then
  usage
  exit 2
fi

tag="$1"
workspace_version="$2"

numeric='(0|[1-9][0-9]*)'
non_numeric='[0-9]*[A-Za-z-][0-9A-Za-z-]*'
prerelease="(${numeric}|${non_numeric})(\.(${numeric}|${non_numeric}))*"
build='[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*'
semver="${numeric}\.${numeric}\.${numeric}(-${prerelease})?(\+${build})?"

if [[ "$tag" != v* ]]; then
  printf 'release tag must start with v: %s\n' "$tag" >&2
  exit 1
fi

tag_version="${tag#v}"
if ! [[ "$tag_version" =~ ^${semver}$ ]]; then
  printf 'release tag is not valid SemVer: %s\n' "$tag" >&2
  exit 1
fi

if [[ "$tag_version" != 2.* ]]; then
  printf 'release tag must select the 2.x line: %s\n' "$tag" >&2
  exit 1
fi

if ! [[ "$workspace_version" =~ ^${semver}$ ]]; then
  printf 'workspace version is not valid SemVer: %s\n' "$workspace_version" >&2
  exit 1
fi

if [[ "$tag_version" != "$workspace_version" ]]; then
  printf 'release tag %s does not match workspace version %s\n' \
    "$tag" "$workspace_version" >&2
  exit 1
fi
