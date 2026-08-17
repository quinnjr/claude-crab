// SPDX-License-Identifier: MIT
//
// gen_sprites.py is the source of truth; the PNGs are build artefacts and are
// not checked in, so art and manifest can never drift apart.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let generator = manifest_dir.join("tools/gen_sprites.py");
    let icon_dir = out_dir.join("icons");

    println!("cargo:rerun-if-changed={}", generator.display());

    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
    let status = Command::new(&python)
        .arg(&generator)
        .arg("--out")
        .arg(&out_dir)
        .arg("--icons")
        .arg(&icon_dir)
        .status();

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => panic!("{} {} failed with {status}", python, generator.display()),
        Err(err) => panic!(
            "cannot run {} {}: {err}\n\
             The sprite sheet is generated at build time and requires Python 3 with Pillow.",
            python,
            generator.display()
        ),
    }

    // Fail the build rather than ship a binary with an unrenderable crab.
    for name in ["spritesheet.png", "spritesheet-fancy.png", "spritesheet-party.png", "manifest.json"]
    {
        let path = out_dir.join(name);
        assert!(path.exists(), "{} did not produce {}", generator.display(), path.display());
    }

    println!("cargo:rustc-env=CRAB_ICON_DIR={}", icon_dir.display());
}
