#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright 2026 the Klimper authors
#
# Regenerate every app icon (macOS, Windows, Linux, iOS, Android) from one square logo.
#
#   scripts/update-logo.sh                 # from src/icon-light.png
#   scripts/update-logo.sh path/to/logo.png
#
# The logo should be square, at least 1024px, with its own opaque background (like
# src/icon-light.png). Two sources are made from it and kept in src-tauri/icons/:
#   source-mobile.png   full bleed, no transparency: iOS and Android round the corners themselves
#   source-desktop.png  the macOS shape: a rounded square with a transparent margin
# The in-app logo (src/icon-dark.png, src/icon-light.png) is not touched: replace those files by
# hand. Needs ImageMagick (brew install imagemagick) and the project's pnpm install.
set -euo pipefail

cd "$(dirname "$0")/.."
src="${1:-src/icon-light.png}"
icons=src-tauri/icons
res=src-tauri/gen/android/app/src/main/res

command -v magick >/dev/null || { echo "ImageMagick is missing: brew install imagemagick" >&2; exit 1; }
[ -f "$src" ] || { echo "No such file: $src" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# Full bleed for the phones: square, 1024px, no alpha.
magick "$src" -resize '1024x1024^' -gravity center -extent 1024x1024 -alpha off "$icons/source-mobile.png"

# The macOS template: an 824px rounded square (radius 185) centred on a transparent 1024px canvas.
magick "$src" -resize '824x824^' -gravity center -extent 824x824 -alpha off \
  \( -size 824x824 xc:black -fill white -draw 'roundrectangle 0,0 823,823 185,185' -alpha off \) \
  -compose CopyOpacity -composite \
  -compose Over -background none -gravity center -extent 1024x1024 "$icons/source-desktop.png"

# Phones first: this writes src-tauri/icons, the iOS asset catalogue and the Android mipmaps.
pnpm exec tauri icon "$icons/source-mobile.png" >/dev/null 2>&1
# tauri icon writes the iOS icons with an alpha channel, opaque or not, and App Store Connect
# refuses an upload whose 1024px icon has one (ITMS-90717). Drop the channel.
for f in src-tauri/gen/apple/Assets.xcassets/AppIcon.appiconset/*.png; do
  magick "$f" -background white -alpha remove -alpha off "$f"
done

# Then the desktop files, from the macOS-shaped source, over the ones just written.
pnpm exec tauri icon "$icons/source-desktop.png" -o "$tmp" >/dev/null 2>&1
for f in "$tmp"/*; do
  [ -f "$f" ] && cp "$f" "$icons/"
done

# tauri icon rewrites two Android XML files in its own formatting and sets a white launcher
# background. Keep ours and take the background colour from the logo's corner instead.
git checkout -- "$res/mipmap-anydpi-v26/ic_launcher.xml" "$res/values/ic_launcher_background.xml"
bg="$(magick "$src" -format '#%[hex:p{2,2}]' info: | cut -c1-7)"
sed -i.bak -E "s/#[0-9A-Fa-f]{3,8}</${bg}</" "$res/values/ic_launcher_background.xml"
rm -f "$res/values/ic_launcher_background.xml.bak"

echo "App icons regenerated from $src (Android launcher background $bg)."
git status --short -- "$icons" src-tauri/gen | sed 's/^/  /' | head -20
echo "Rebuild the app to see them; macOS may cache the old Dock icon until the app is restarted."
