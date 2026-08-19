#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Assembles Claude Crab.app from prebuilt per-arch binaries and wraps it in a
# dmg. Expects release builds for both aarch64-apple-darwin and
# x86_64-apple-darwin to exist already; run from the repository root:
#
#   packaging/macos/build-app.sh <version>
set -euo pipefail

version=$1
appdir="dist/Claude Crab.app"
rm -rf dist
mkdir -p "$appdir/Contents/MacOS" "$appdir/Contents/Resources"

lipo -create -output "$appdir/Contents/MacOS/claude-crab" \
  target/aarch64-apple-darwin/release/claude-crab \
  target/x86_64-apple-darwin/release/claude-crab

sed "s/@VERSION@/$version/" packaging/macos/Info.plist > "$appdir/Contents/Info.plist"

# The icns is assembled from the icons the build script renders, so the app
# icon cannot drift from the character. iconutil only accepts the standard
# iconset sizes, which is why 48.png is skipped.
icondir=$(find target/aarch64-apple-darwin/release/build -type d -name icons -print -quit)
iconset=dist/claude-crab.iconset
mkdir -p "$iconset"
cp "$icondir/32.png" "$iconset/icon_32x32.png"
cp "$icondir/64.png" "$iconset/icon_32x32@2x.png"
cp "$icondir/128.png" "$iconset/icon_128x128.png"
cp "$icondir/256.png" "$iconset/icon_128x128@2x.png"
cp "$icondir/256.png" "$iconset/icon_256x256.png"
iconutil -c icns -o "$appdir/Contents/Resources/claude-crab.icns" "$iconset"
rm -rf "$iconset"

# Ad-hoc signature: unsigned arm64 binaries refuse to launch at all. Users
# still have to right-click > Open the first time, since the bundle is not
# notarized.
codesign --force -s - "$appdir"

hdiutil create -volname "Claude Crab" -srcfolder "$appdir" -ov -format UDZO \
  "dist/claude-crab-$version-macos.dmg"
