#!/usr/bin/env bash
# Usage: scripts/version-bump.sh <major.minor.patch>
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

version="${1:-}"
semver_pattern='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'

if [[ ! "$version" =~ $semver_pattern ]]; then
  echo "Usage: scripts/version-bump.sh <major.minor.patch>" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "The working tree must be clean before bumping the version." >&2
  exit 1
fi

current_version="$(awk -F '"' '/^\[package\]/{in_package=1; next} in_package && /^version = /{print $2; exit}' Cargo.toml)"
if [[ -z "$current_version" ]]; then
  echo "Unable to read the current version from Cargo.toml." >&2
  exit 1
fi

if [[ "$current_version" == "$version" ]]; then
  echo "serialX is already at version ${version}." >&2
  exit 1
fi

SERIALX_VERSION="$version" perl -0pi -e '
  BEGIN { $version = $ENV{"SERIALX_VERSION"}; }
  s{(\[package\][\s\S]*?^version\s*=\s*")[^"]+(".*$)}{$1 . $version . $2}em
    or die "Unable to update Cargo.toml\n";
' Cargo.toml

SERIALX_VERSION="$version" perl -0pi -e '
  BEGIN { $version = $ENV{"SERIALX_VERSION"}; }
  s{(\[\[package\]\]\Rname = "serialx"\Rversion = ")[^"]+("\R)}{$1 . $version . $2}e
    or die "Unable to update serialx in Cargo.lock\n";
' Cargo.lock

cargo metadata --locked --no-deps --format-version 1 >/dev/null
git diff --check -- Cargo.toml Cargo.lock

git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to ${version}"
git push
