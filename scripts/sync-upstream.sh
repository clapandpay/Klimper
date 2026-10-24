#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# Merge an upstream Sage release (or commit) into the current branch.
#
#   scripts/sync-upstream.sh            merge the newest upstream v* tag
#   scripts/sync-upstream.sh v0.13.1    merge that tag
#   scripts/sync-upstream.sh <commit>   hotfix: merge upstream/main up to that commit
#   scripts/sync-upstream.sh --check    only report what is new upstream (run by hand; CI never
#                                       runs on a clock, see AGENTS.md "CI")
#
# And the other direction — what we owe upstream, and how to hand it over:
#
#   scripts/sync-upstream.sh --diff     every file that differs from the merged upstream tag,
#                                       split into ours (never conflicts) and shared (the whole
#                                       cost of the next merge)
#   scripts/sync-upstream.sh --export   write the shared-file edits as a patch that applies to a
#                                       clean Sage checkout, for proposing them upstream
#   scripts/sync-upstream.sh --credits  who wrote Sage, merged across the several identities
#                                       people commit under. NOTICE has to name them all.
#
# Always a merge, never a rebase: shared history keeps later merges small.
#
# Klimper has upstream's files at upstream's paths, so the merge is a plain one. The folder is
# still worked out from where this script sits: in a checkout that keeps Klimper in a
# subfolder, SUBTREE names it and the merge uses git's subtree option, so that nothing outside
# that folder can be touched.
set -euo pipefail
SUBTREE="$(cd "$(dirname "$0")/.." && git rev-parse --show-prefix)"
SUBTREE="${SUBTREE%/}"
IN="${SUBTREE:+$SUBTREE/}"

UPSTREAM_URL="https://github.com/xch-dev/sage.git"
# Paths where an upstream change may be a security fix (reported by --check).
WATCH_PATHS=(crates/sage-wallet crates/sage-keychain crates/sage-apps crates/sage-database
  crates/sage src-tauri tauri-plugin-sage Cargo.toml Cargo.lock pnpm-lock.yaml)

# The edits to shared files that are meant to stop being ours: the build-time feature flags and
# the route gate, which Sage can take as one pull request ("more VITE_DISABLE_* flags, and gate
# the routes so a disabled feature is unreachable and not merely hidden"). Paths are Sage's, so
# the patch --export writes applies to a clean Sage checkout as it stands.
UPSTREAMABLE=(src/lib/features.ts src/lib/featureElement.tsx src/vite-env.d.ts src/App.tsx
  src/components/Nav.tsx src/pages/TokenList.tsx src/pages/Settings.tsx)

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

# Upstream keeps these files at the root and we keep them under $SUBTREE/, so the two trees are
# compared directly: `git diff <upstream tag> HEAD:$SUBTREE` lines the paths up, and every path it
# prints is already spelled the way Sage spells it.
if [[ "${1:-}" == "--diff" ]]; then
  echo "fork surface against $merged_tag"
  echo
  echo "ours (new files, never conflict):"
  git diff --name-status "$merged_tag" "HEAD:$SUBTREE" | awk '$1 ~ /^A/ {print "  " $2}'
  echo
  echo "shared with Sage (every one of these is work at the next merge):"
  git diff --name-status "$merged_tag" "HEAD:$SUBTREE" | awk '$1 ~ /^[MR]/ {print "  " $1 "\t" $2}'
  echo
  echo "Keep this list and the table in ${IN}docs/UPSTREAM.md saying the same thing."
  exit 0
fi

# Apache-2.0 asks a fork to keep attribution, and Sage ships no NOTICE of its own, so ours is
# the only place its authors are named. People commit under more than one identity, so the
# addresses are folded together here rather than counted twice.
if [[ "${1:-}" == "--credits" ]]; then
  git shortlog -sne "$merged_tag" | awk -F'\t' '
    {
      split($2, p, " <"); name = p[1]; email = substr(p[2], 1, length(p[2]) - 1)
      if (email ~ /rigidnetwork\.com/ || email ~ /Rigidity@/) name = "Rigidity (Brandon Haggstrom)"
      else if (email ~ /hadamcik/)          name = "Jakub Hadamcik"
      else if (email ~ /ronald\.goedeke/)   name = "Ronald Goedeke (BubbleTrouble14)"
      else if (email ~ /geraldneale/)       name = "Gerald Neale"
      n[name] += $1; total += $1
    }
    END {
      for (k in n) printf "%6d  %s\n", n[k], k
      printf "%6d  commits by %d people, at %s\n", total, length(n), "'"$merged_tag"'"
    }' | sort -rn
  echo
  echo "NOTICE must name every one of them. Regenerate it after each merge."
  exit 0
fi

if [[ "${1:-}" == "--export" ]]; then
  out="${2:-upstream-flags.patch}"
  git diff "$merged_tag" "HEAD:$SUBTREE" -- "${UPSTREAMABLE[@]}" > "$out"
  if [[ ! -s "$out" ]]; then
    echo "nothing to export: none of the upstreamable files differ from $merged_tag" >&2
    rm -f "$out"
    exit 1
  fi
  echo "wrote $out ($(grep -c '^diff --git' "$out") files, against $merged_tag)"
  echo
  echo "To propose it to Sage:"
  echo "  git clone https://github.com/xch-dev/sage.git && cd sage"
  echo "  git checkout -b feature-flags $merged_tag"
  echo "  git apply $(cd "$(dirname "$out")" && pwd)/$(basename "$out")"
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
merge_opts=()
[[ -n "$SUBTREE" ]] && merge_opts=(-X "subtree=$SUBTREE")
if ! git merge --no-ff ${merge_opts[@]+"${merge_opts[@]}"} -m "Merge Sage $target into ${IN:-the root}" "$target"; then
  cat >&2 <<MSG

Conflicts. All paths are under ${IN:-the root}. Ours to keep: README.md, LICENSE,
src-tauri/tauri.ios.conf.json and tauri.android.conf.json (identifier lines), the bundle ids
under src-tauri/gen/.
Everything else: take upstream and move our change behind a flag or into our own file.
Keychain password (crates/sage-keychain, crates/sage): take upstream, then pass
self.keychain.password()? wherever Sage passes b"" (docs/UPSTREAM.md, "Staying current").
Playbook: ${IN}docs/UPSTREAM.md. Finish with: git commit
MSG
  exit 1
fi

cat <<MSG

Merged $target. Still to do (docs/UPSTREAM.md):
  1. pnpm install && cargo check --workspace && pnpm tauri:build
  2. bridge smoke test: install mempoolxch.space and sage-permission-probe
  3. compare src-tauri/tauri.conf.json window settings with our tauri.<platform>.conf.json
  4. record the merged version in docs/UPSTREAM.md. Klimper's own version does not follow it
     (scripts/set-version.sh --check): apps are told the Sage version by context() in
     src-tauri/src/lib.rs
MSG
