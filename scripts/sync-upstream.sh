#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# Merge an upstream Sage release (or commit) into the current branch.
#
#   scripts/sync-upstream.sh            merge the newest upstream v* tag
#   scripts/sync-upstream.sh v0.13.1    merge that tag
#   scripts/sync-upstream.sh <commit>   hotfix: merge upstream/main up to that commit
#   scripts/sync-upstream.sh --check    only report what is new upstream (used by CI)
#
# Always a merge, never a rebase: shared history keeps later merges small.
#
# The wallet lives in klimper/ of the monorepo while upstream has it at the root, so the merge
# uses git's subtree option: upstream's paths are shifted into klimper/ and nothing outside
# that folder can be touched.
SUBTREE="klimper"
set -euo pipefail

UPSTREAM_URL="https://github.com/xch-dev/sage.git"
# Paths where an upstream change may be a security fix (reported by --check).
WATCH_PATHS=(crates/sage-wallet crates/sage-keychain crates/sage-apps crates/sage-database
  crates/sage src-tauri tauri-plugin-sage Cargo.toml Cargo.lock pnpm-lock.yaml)

cd "$(git rev-parse --show-toplevel)"

git remote get-url upstream >/dev/null 2>&1 || git remote add upstream "$UPSTREAM_URL"
git config merge.ours.driver true
git fetch --quiet --tags upstream

merged_tag="$(git describe --tags --abbrev=0 --match 'v*' HEAD 2>/dev/null || echo none)"
latest_tag="$(git tag --list 'v*' --sort=-v:refname | head -n 1)"

if [[ "${1:-}" == "--check" ]]; then
  echo "merged upstream tag: $merged_tag"
  echo "latest upstream tag: $latest_tag"
  echo
  echo "upstream/main commits not in HEAD that touch watched paths:"
  git log --no-merges --format='  %h %ad %s' --date=short HEAD..upstream/main -- "${WATCH_PATHS[@]}"
  exit 0
fi

target="${1:-$latest_tag}"

if [[ -n "$(git status --porcelain)" ]]; then
  echo "working tree is not clean; commit or stash first" >&2
  exit 1
fi

if git merge-base --is-ancestor "$target" HEAD; then
  echo "$target is already merged"
  exit 0
fi

echo "merging $target (currently on $merged_tag)"
if ! git merge --no-ff -X "subtree=$SUBTREE" -m "Merge Sage $target into $SUBTREE/" "$target"; then
  cat >&2 <<MSG

Conflicts. All paths are under $SUBTREE/. Ours to keep: README.md, LICENSE,
src-tauri/tauri.ios.conf.json and tauri.android.conf.json (identifier lines), the bundle ids
under src-tauri/gen/.
Everything else: take upstream and move our change behind a flag or into our own file.
Playbook: $SUBTREE/docs/UPSTREAM.md. Finish with: git commit
MSG
  exit 1
fi

cat <<MSG

Merged $target. Still to do (docs/UPSTREAM.md):
  1. pnpm install && cargo check --workspace && pnpm tauri:build
  2. bridge smoke test: install mempoolxch.space and sage-permission-probe
  3. compare src-tauri/tauri.conf.json window settings with our tauri.<platform>.conf.json
  4. record the merged version in docs/UPSTREAM.md
MSG
