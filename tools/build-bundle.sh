#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Build a single-file .flatpak bundle, the thing people can actually install
# without Flathub. Flathub does not accept AI-assisted applications, so this is
# how the Flatpak build reaches anyone at all -- see packaging/flathub/README.md.
#
#   tools/build-bundle.sh                 # from the working tree (default)
#   tools/build-bundle.sh --pinned        # from the tag the Flathub manifest pins
#
# The default builds whatever is checked out, which is what a release artifact
# wants: at the moment a tag is cut, the Flathub manifest still points at the
# PREVIOUS release, because its tag and commit can only be filled in once the
# tag exists. Building that here would quietly ship the last version.
#
# The result installs with:
#   flatpak install --user ./claude-crab-<version>.flatpak

set -euo pipefail

APP_ID=dev.quinnjr.claude-crab
RUNTIME_REPO=https://dl.flathub.org/repo/flathub.flatpakrepo

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

manifest=packaging/${APP_ID}.yml
if [[ ${1:-} == --pinned ]]; then
    manifest=packaging/flathub/${APP_ID}.yml
fi

# Version comes from CMakeLists so the bundle can never disagree with the
# binary inside it.
version=$(sed -n 's/^project(claude-crab VERSION \([0-9.]*\).*/\1/p' CMakeLists.txt)
if [[ -z $version ]]; then
    echo "error: could not read the version out of CMakeLists.txt" >&2
    exit 1
fi

# Kept inside the project rather than /tmp: flatpak-builder refuses to run when
# its state dir (.flatpak-builder, here) is on a different filesystem from the
# build dir, and keeping them together also reuses the downloaded sources.
repo=$root/.bundle-repo
builddir=$root/.bundle-build
rm -rf "$repo" "$builddir"
trap 'rm -rf "$repo" "$builddir"' EXIT

echo "==> building $manifest"
flatpak-builder --user --force-clean --repo="$repo" "$builddir" "$manifest"

# The branch is whatever the manifest produced; asking the repo avoids
# hardcoding a default that a manifest change could silently move.
branch=$(flatpak build-bundle --help >/dev/null 2>&1 && \
    ostree refs --repo="$repo" | sed -n "s|^app/${APP_ID}/[^/]*/||p" | head -1)
branch=${branch:-master}

bundle="claude-crab-${version}.flatpak"
echo "==> bundling $APP_ID (branch $branch) -> $bundle"

# --runtime-repo means a recipient who lacks org.kde.Platform is offered it
# automatically instead of hitting an unresolved-dependency error.
flatpak build-bundle --runtime-repo="$RUNTIME_REPO" \
    "$repo" "$bundle" "$APP_ID" "$branch"

echo "==> $bundle ($(du -h "$bundle" | cut -f1))"
echo "    install with: flatpak install --user ./$bundle"
