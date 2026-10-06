#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# Klimper's own release number, in every file that carries it. It is not Sage's version: the
# engine's stays in the Cargo.toml files and moves with each upstream merge (docs/UPSTREAM.md,
# "Klimper version").
#
#   scripts/set-version.sh 0.1.1     write it
#   scripts/set-version.sh --check   print it, or fail when the files disagree
#
# Three numbers, because that is all Apple takes in CFBundleShortVersionString. A TestFlight
# build of the same version is told apart by its build number (scripts/testflight.sh).
set -euo pipefail

cd "$(dirname "$0")/.."

configs=(src-tauri/tauri.{macos,linux,windows,ios,android}.conf.json)
project=src-tauri/gen/apple/project.yml
plist=src-tauri/gen/apple/sage-tauri_iOS/Info.plist
frontend=src/klimper/version.json

json_version() {
  sed -n 's/.*"version":[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -1
}

if [ "${1:-}" = "--check" ]; then
  version="$(json_version "$frontend")"
  [ -n "$version" ] || { echo "$frontend has no version" >&2; exit 1; }
  bad=0
  for file in "${configs[@]}"; do
    found="$(json_version "$file")"
    [ "$found" = "$version" ] || { echo "$file: \"$found\", not $version" >&2; bad=1; }
  done
  # CFBundleVersion is left out: a TestFlight build writes its build number there.
  grep -q "CFBundleShortVersionString: $version\$" "$project" ||
    { echo "$project: CFBundleShortVersionString is not $version" >&2; bad=1; }
  grep -A1 '<key>CFBundleShortVersionString</key>' "$plist" | grep -q "<string>$version</string>" ||
    { echo "$plist: CFBundleShortVersionString is not $version" >&2; bad=1; }
  [ "$bad" = 0 ] || { echo "Run scripts/set-version.sh $version to make them agree." >&2; exit 1; }
  echo "$version"
  exit 0
fi

version="${1:-}"
if ! [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "Usage: scripts/set-version.sh <major.minor.patch> | --check" >&2
  exit 1
fi

for file in "${configs[@]}"; do
  grep -q '^[[:space:]]*"version":' "$file" || { echo "$file has no version line" >&2; exit 1; }
  V="$version" perl -pi -e 's/^(\s*"version":\s*")[^"]*/$1$ENV{V}/ and $done++ unless $done' "$file"
done
V="$version" perl -pi -e '
  s/^(\s*CFBundleShortVersionString:\s*).*/$1$ENV{V}/;
  s/^(\s*CFBundleVersion:\s*).*/$1"$ENV{V}"/' "$project"
V="$version" perl -0pi -e '
  s|(<key>CFBundleShortVersionString</key>\s*<string>)[^<]*|$1$ENV{V}|;
  s|(<key>CFBundleVersion</key>\s*<string>)[^<]*|$1$ENV{V}|' "$plist"
printf '{ "version": "%s" }\n' "$version" > "$frontend"

"$0" --check >/dev/null
echo "Klimper is $version. A release is tagged klimper-v$version (README.md)."
