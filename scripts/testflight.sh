#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# Build the signed iOS app and upload it to TestFlight. Runs on the Mac that holds the App Store
# Connect API key, never in CI (see AGENTS.md: CI builds the website and Green Envelope images and
# nothing else, and the key stays on one machine).
#
#   scripts/testflight.sh                  # build number: minutes since the epoch
#   scripts/testflight.sh --build-number 42
#   scripts/testflight.sh --no-upload      # build and sign only
#
# The API key is read from the environment, or from ~/.appstoreconnect/klimper.env:
#   APPLE_API_KEY=<key id>
#   APPLE_API_ISSUER=<issuer id>
# Optional, same file: KLIMPER_PUSH_GATEWAY_URL and KLIMPER_PUSH_GATEWAY_KEY (the gateway's
# https wake URL and its base64url X25519 public key) turn push wake-ups on in the build; without
# them the app only updates while it is open.
# with the key itself at ~/.appstoreconnect/private_keys/AuthKey_<key id>.p8, where altool also
# looks for it. The key needs the Admin role: xcodebuild uses it to create the distribution
# certificate and provisioning profile when they are missing.
set -euo pipefail

cd "$(dirname "$0")/.."

build_number="$(( $(date -u +%s) / 60 ))"
upload=1
while [ $# -gt 0 ]; do
  case "$1" in
    --build-number) build_number="$2"; shift 2 ;;
    --no-upload) upload=0; shift ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

env_file="$HOME/.appstoreconnect/klimper.env"
if [ -z "${APPLE_API_KEY:-}" ] && [ -f "$env_file" ]; then
  # shellcheck disable=SC1090
  . "$env_file"
fi
: "${APPLE_API_KEY:?set APPLE_API_KEY or write it to $env_file}"
: "${APPLE_API_ISSUER:?set APPLE_API_ISSUER or write it to $env_file}"
export APPLE_API_KEY APPLE_API_ISSUER
if [ -n "${KLIMPER_PUSH_GATEWAY_URL:-}" ] && [ -n "${KLIMPER_PUSH_GATEWAY_KEY:-}" ]; then
  export KLIMPER_PUSH_GATEWAY_URL KLIMPER_PUSH_GATEWAY_KEY
else
  unset KLIMPER_PUSH_GATEWAY_URL KLIMPER_PUSH_GATEWAY_KEY
  echo "Push is off in this build: set KLIMPER_PUSH_GATEWAY_URL and KLIMPER_PUSH_GATEWAY_KEY." >&2
fi
export APPLE_API_KEY_PATH="$HOME/.appstoreconnect/private_keys/AuthKey_${APPLE_API_KEY}.p8"
[ -f "$APPLE_API_KEY_PATH" ] || { echo "No API key at $APPLE_API_KEY_PATH" >&2; exit 1; }

if [ -n "$(git status --porcelain -- . ':!builtin-apps/build')" ]; then
  echo "Warning: uncommitted changes in klimper/ go into this build." >&2
fi

command -v bindgen >/dev/null || { echo "bindgen is missing: cargo install bindgen-cli --locked" >&2; exit 1; }
# swift-rs needs llvm-objcopy under Xcode 27 to export the plugins' @_cdecl functions; without it
# the link fails on undefined init_plugin_* symbols (docs/UPSTREAM.md, swift-rs 1.0.8).
compgen -G "$(rustc --print sysroot)/lib/rustlib/*/bin/llvm-objcopy" >/dev/null ||
  { echo "llvm-objcopy is missing: rustup component add llvm-tools" >&2; exit 1; }
pnpm install --frozen-lockfile
# Release builds strip debug info by default, and Xcode 27's strip leaves the build-time proc
# macros (sqlx_macros first) with a LINKEDIT that macOS 27's dyld refuses to load ("mis-aligned
# LINKEDIT string pool"). Leaving those host-only libraries unstripped costs nothing in the app.
export CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=false
# CFBundleVersion is the build number alone: tauri's --build-number appends it to the version
# (0.13.1.N), four parts where Apple allows three, and the export then drops it back to 0.13.1.
pnpm tauri ios build --export-method app-store-connect --ci \
  --config "{\"bundle\":{\"iOS\":{\"bundleVersion\":\"$build_number\"}}}"

ipa="$(ls -t src-tauri/gen/apple/build/arm64/*.ipa | head -1)"
echo "Built $ipa ($(git rev-parse --short HEAD), build $build_number)"
[ "$upload" = 1 ] || exit 0

xcrun altool --upload-app --type ios --file "$ipa" \
  --apiKey "$APPLE_API_KEY" --apiIssuer "$APPLE_API_ISSUER"
echo "Uploaded. It appears in App Store Connect → TestFlight once Apple has processed it."
