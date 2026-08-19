// SPDX-License-Identifier: MIT
//
// Backend selection.
//
// Two ways to put a crab on a screen:
//
//   * `layer_shell` -- a wlr-layer-shell surface spanning the bottom of the
//     screen, with the input region confined to the character so the rest of
//     the strip stays click-through. Best fidelity, but the protocol only
//     exists on wlroots-family compositors and KWin.
//
//   * `floating` -- a small always-on-top window that follows the crab around.
//     Works on macOS, Windows, X11 and GNOME, at the cost of being a real
//     window rather than a layer above the panel.

use crate::app::{Core, Layout, Mode};
use crate::brain::Brain;
use crate::config::CrabConfig;
use crate::tracker::SessionTracker;
use std::path::PathBuf;

#[cfg(all(unix, not(target_os = "macos")))]
pub mod layer_shell;

pub mod floating;

/// Everything a `Core` needs, kept separate so a backend that turns out to be
/// unavailable can hand them back rather than swallowing them.
pub struct CoreParts {
    pub config: CrabConfig,
    pub config_path: PathBuf,
    pub tracker: SessionTracker,
    pub brain: Brain,
    pub mode: Mode,
}

impl CoreParts {
    pub fn into_core(self, layout: Layout) -> Core {
        Core::new(self.config, self.config_path, self.tracker, self.brain, self.mode, layout)
    }
}

/// Which backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    LayerShell,
    Floating,
}

impl Backend {
    pub fn layout(self) -> Layout {
        match self {
            Self::LayerShell => Layout::FullStrip,
            Self::Floating => Layout::FollowCrab,
        }
    }
}

/// Pick a backend for this session.
///
/// `CLAUDE_CRAB_BACKEND=floating` forces the portable path, which is the
/// escape hatch when a compositor advertises layer-shell but misbehaves.
pub fn pick() -> Backend {
    match std::env::var("CLAUDE_CRAB_BACKEND").as_deref() {
        Ok("floating") => return Backend::Floating,
        Ok("layer-shell") => return Backend::LayerShell,
        Ok(other) if !other.is_empty() => {
            log::warn!("unknown CLAUDE_CRAB_BACKEND={other}; ignoring");
        }
        _ => {}
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    if layer_shell::available() {
        return Backend::LayerShell;
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    log::info!(
        "no wlr-layer-shell on this session; using a floating window. \
         GNOME and X11 take this path."
    );

    Backend::Floating
}

pub fn run(parts: CoreParts) -> Result<(), String> {
    let backend = pick();
    log::info!("backend: {backend:?}");
    let core = parts.into_core(backend.layout());

    match backend {
        #[cfg(all(unix, not(target_os = "macos")))]
        Backend::LayerShell => layer_shell::run(core),
        #[cfg(not(all(unix, not(target_os = "macos"))))]
        Backend::LayerShell => {
            Err("layer-shell only exists on Linux and the BSDs".to_string())
        }

        Backend::Floating => floating::run(core),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_their_backends() {
        assert_eq!(Backend::LayerShell.layout(), Layout::FullStrip);
        assert_eq!(Backend::Floating.layout(), Layout::FollowCrab);
    }

    #[test]
    fn the_env_override_forces_a_backend() {
        // SAFETY: single-threaded test; nothing else reads the environment.
        unsafe { std::env::set_var("CLAUDE_CRAB_BACKEND", "floating") };
        assert_eq!(pick(), Backend::Floating);
        unsafe { std::env::set_var("CLAUDE_CRAB_BACKEND", "layer-shell") };
        assert_eq!(pick(), Backend::LayerShell);
        unsafe { std::env::remove_var("CLAUDE_CRAB_BACKEND") };
    }
}
