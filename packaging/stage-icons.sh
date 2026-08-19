#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# The build script renders the icons into a hashed OUT_DIR, so the fixed asset
# paths in Cargo.toml's deb/rpm metadata cannot point at them directly. This
# copies them to target/pkg-icons after `cargo build --release`.
set -euo pipefail

target_dir=${CARGO_TARGET_DIR:-target}
icondir=$(find "$target_dir"/release/build -type d -name icons -print -quit)
if [ -z "$icondir" ]; then
  echo "no icons directory under $target_dir/release/build; run cargo build --release first" >&2
  exit 1
fi

mkdir -p "$target_dir/pkg-icons"
cp "$icondir"/*.png "$target_dir/pkg-icons/"
