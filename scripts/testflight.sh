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
# with the key itself at ~/.appstoreconnect/private_keys/AuthKey_<key id>.p8, where altool also
# looks for it. The key needs the Admin role: xcodebuild uses it to create the distribution
# certificate and provisioning profile when they are missing.
#
# Push wake-ups come from the committed push-gateway.env (the gateway's https wake URL and its
# base64url X25519 public key, docs/push.md) and from nowhere else: the script prints the
# gateway the build gets, and refuses to build while .push-dev.env is here.
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

# The plugin's build (tauri-plugin-klimper-xchonnect/build.rs) takes the push gateway from the
# environment, then from .push-dev.env, then from push-gateway.env. .push-dev.env is a local
# stack's loopback gateway (scripts/push-dev.sh): a TestFlight build made with it wakes nobody.
if [ -f .push-dev.env ]; then
  echo "klimper/.push-dev.env is here and would replace push-gateway.env in this build: move it away first." >&2
  exit 1
fi

env_file="$HOME/.appstoreconnect/klimper.env"
if [ -z "${APPLE_API_KEY:-}" ] && [ -f "$env_file" ]; then
  # shellcheck disable=SC1090
  . "$env_file"
fi
: "${APPLE_API_KEY:?set APPLE_API_KEY or write it to $env_file}"
: "${APPLE_API_ISSUER:?set APPLE_API_ISSUER or write it to $env_file}"
export APPLE_API_KEY APPLE_API_ISSUER
# Out of the environment too, so that the committed file alone decides.
unset KLIMPER_PUSH_GATEWAY_URL KLIMPER_PUSH_GATEWAY_KEY
push_setting() {
  sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*//p" push-gateway.env 2>/dev/null | tail -1 | tr -d '[:space:]'
}
push_url="$(push_setting KLIMPER_PUSH_GATEWAY_URL)"
if [ -n "$push_url" ] && [ -n "$(push_setting KLIMPER_PUSH_GATEWAY_KEY)" ]; then
  echo "Push gateway in this build: $push_url (push-gateway.env)" >&2
else
  echo "Push is off in this build: push-gateway.env has no KLIMPER_PUSH_GATEWAY_URL and KLIMPER_PUSH_GATEWAY_KEY." >&2
fi
export APPLE_API_KEY_PATH="$HOME/.appstoreconnect/private_keys/AuthKey_${APPLE_API_KEY}.p8"
[ -f "$APPLE_API_KEY_PATH" ] || { echo "No API key at $APPLE_API_KEY_PATH" >&2; exit 1; }

# One version in every file that carries it (docs/UPSTREAM.md, "Klimper version"): TestFlight
# groups the builds of a version together and tells them apart by the build number.
version="$(scripts/set-version.sh --check)"

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
# (0.1.0.N), four parts where Apple allows three, and the export then drops it back to 0.1.0.
pnpm tauri ios build --export-method app-store-connect --ci \
  --config "{\"bundle\":{\"iOS\":{\"bundleVersion\":\"$build_number\"}}}"

ipa="$(ls -t src-tauri/gen/apple/build/arm64/*.ipa | head -1)"
echo "Built $ipa (Klimper $version, build $build_number, $(git rev-parse --short HEAD))"
[ "$upload" = 1 ] || exit 0

xcrun altool --upload-app --type ios --file "$ipa" \
  --apiKey "$APPLE_API_KEY" --apiIssuer "$APPLE_API_ISSUER"
echo "Uploaded. It appears in App Store Connect → TestFlight once Apple has processed it."
