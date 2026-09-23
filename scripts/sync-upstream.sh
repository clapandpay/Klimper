#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Merge a Sage release into the current branch (see docs/UPSTREAM.md).
# Usage: scripts/sync-upstream.sh v0.13.2
set -euo pipefail

tag="${1:?usage: $0 <sage-tag>}"
cd "$(git rev-parse --show-toplevel)"

git diff --quiet && git diff --cached --quiet || { echo "working tree not clean" >&2; exit 1; }
git remote get-url upstream >/dev/null 2>&1 || git remote add upstream https://github.com/xch-dev/sage.git
git fetch upstream --tags

git switch -c "sync/sage-$tag"
if ! git merge --no-ff --no-edit -m "Merge Sage $tag" "$tag"; then
  echo "Conflicts: keep our side for the files listed in docs/UPSTREAM.md, take upstream elsewhere." >&2
  git diff --name-only --diff-filter=U >&2
  exit 1
fi

# Our identifiers must survive the merge.
grep -q '"identifier": "com.clapandpay.klimper"' src-tauri/tauri.conf.json
grep -q 'com.clapandpay.klimper' src-tauri/gen/apple/project.yml

pnpm install --frozen-lockfile
pnpm build
cargo check --workspace

echo "Merged $tag. Next: run the app-bridge smoke test (mempoolxch.space installed), add a row to docs/UPSTREAM.md, open a PR."
