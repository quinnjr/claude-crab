// SPDX-License-Identifier: MIT
//
// Finds a UI font for the right-click menu.
//
// skia-rs' default typeface carries no font data, and glyphs without outlines
// are silently skipped -- a menu drawn with it would be blank rather than
// broken-looking. So a real font file has to be found on disk.
//
// ponytail: a path probe rather than linking fontconfig / DirectWrite /
// CoreText. Every desktop that can run this ships one of these faces, and the
// alternative is three platform font APIs for a lookup that is a few `stat`
// calls.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use skia_rs::text::{Font, Typeface};

/// Faces to try, in order. Deliberately boring, always-present families.
#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/gnu-free/FreeSans.ttf",
    "/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf",
    "/usr/share/fonts/cantarell/Cantarell-Regular.otf",
    "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
    // Flatpak exposes the host's fonts here.
    "/run/host/fonts/NotoSans-Regular.ttf",
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/SFNS.ttf",
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/Geneva.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
];

#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &[
    r"C:\Windows\Fonts\segoeui.ttf",
    r"C:\Windows\Fonts\tahoma.ttf",
    r"C:\Windows\Fonts\arial.ttf",
    r"C:\Windows\Fonts\verdana.ttf",
];

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const CANDIDATES: &[&str] = &["/usr/share/fonts/TTF/DejaVuSans.ttf"];

/// Directories to sweep if no known path matched.
#[cfg(target_os = "linux")]
const SWEEP_DIRS: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts", "/run/host/fonts"];

#[cfg(target_os = "macos")]
const SWEEP_DIRS: &[&str] = &["/System/Library/Fonts", "/Library/Fonts"];

#[cfg(target_os = "windows")]
const SWEEP_DIRS: &[&str] = &[r"C:\Windows\Fonts"];

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const SWEEP_DIRS: &[&str] = &["/usr/share/fonts"];

/// Locate a usable font file.
///
/// `CLAUDE_CRAB_FONT` overrides everything, for a system whose fonts live
/// somewhere this does not think to look.
pub fn find_font_file() -> Option<PathBuf> {
    if let Some(over) = std::env::var_os("CLAUDE_CRAB_FONT") {
        let path = PathBuf::from(over);
        if path.is_file() {
            return Some(path);
        }
        log::warn!("CLAUDE_CRAB_FONT={} is not a file; falling back", path.display());
    }

    for candidate in CANDIDATES {
        let path = Path::new(candidate);
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }

    // Last resort: the first regular-looking face anywhere under the font
    // directories. Bounded depth so a pathological tree cannot stall startup.
    for dir in SWEEP_DIRS {
        if let Some(found) = sweep(Path::new(dir), 3) {
            return Some(found);
        }
    }
    None
}

fn sweep(dir: &Path, depth: usize) -> Option<PathBuf> {
    if depth == 0 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        if !matches!(ext.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc") {
            continue;
        }
        // Skip the obvious non-UI faces so the menu does not come out italic.
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
        if name.contains("italic") || name.contains("bold") || name.contains("mono") {
            continue;
        }
        return Some(path);
    }
    subdirs.into_iter().find_map(|d| sweep(&d, depth - 1))
}

/// Load the menu font, or None if no usable face exists on this system.
pub fn load_menu_font(size: f32) -> Option<Font> {
    let path = find_font_file()?;
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(err) => {
            log::warn!("cannot read font {} - {err}", path.display());
            return None;
        }
    };
    let typeface = Typeface::from_data(data).or_else(|| {
        log::warn!("{} is not a font this build can parse", path.display());
        None
    })?;
    log::info!("menu font: {} ({})", typeface.family_name(), path.display());
    Some(Font::new(Arc::new(typeface), size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_font_on_this_system() {
        // Every machine that can run the crab has a font; if this fails the
        // menu would render blank, which is worth a loud test.
        let path = find_font_file().expect("no usable font found");
        assert!(path.is_file());
    }

    #[test]
    fn the_font_actually_parses_and_has_glyphs() {
        let font = load_menu_font(13.0).expect("font should load");
        let typeface = font.typeface().expect("font should carry a typeface");
        assert!(typeface.glyph_count() > 0, "a dataless typeface renders nothing");
    }

    #[test]
    fn env_override_wins_when_valid() {
        let real = find_font_file().unwrap();
        // SAFETY: single-threaded test; no other thread reads the environment.
        unsafe { std::env::set_var("CLAUDE_CRAB_FONT", &real) };
        assert_eq!(find_font_file().unwrap(), real);
        unsafe { std::env::remove_var("CLAUDE_CRAB_FONT") };
    }

    #[test]
    fn a_bogus_override_falls_back_rather_than_failing() {
        unsafe { std::env::set_var("CLAUDE_CRAB_FONT", "/nonexistent/font.ttf") };
        let found = find_font_file();
        unsafe { std::env::remove_var("CLAUDE_CRAB_FONT") };
        assert!(found.is_some(), "should fall back to a system font");
    }

    #[test]
    fn every_platform_has_candidates_configured() {
        assert!(!CANDIDATES.is_empty());
        assert!(!SWEEP_DIRS.is_empty());
    }
}
