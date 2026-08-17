// SPDX-License-Identifier: MIT

mod app;
mod brain;
mod config;
mod font;
mod geom;
mod menu;
mod platform;
mod render;
mod sprites;
mod tracker;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::app::Mode;
use crate::brain::Brain;
use crate::config::{CrabConfig, SPRITE_VARIANTS};
use crate::platform::CoreParts;
use crate::sprites::Manifest;
use crate::tracker::SessionTracker;

#[derive(Parser, Debug)]
#[command(
    name = "claude-crab",
    version,
    about = "A crab that walks above your panel while Claude Code works."
)]
struct Args {
    /// Cycle every animation on a timer.
    #[arg(long)]
    demo: bool,

    /// Replay a recorded JSONL event log.
    #[arg(long, value_name = "FILE")]
    replay: Option<PathBuf>,

    /// Path to claude-crab.json.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Sprite variant, overriding the config file.
    #[arg(long, value_name = "VARIANT")]
    sprite: Option<String>,
}

fn main() -> ExitCode {
    // Default to info so the startup diagnostics are visible without the user
    // having to know about RUST_LOG.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();
    let config_path = args.config.unwrap_or_else(CrabConfig::default_path);

    let mut config = CrabConfig::load(&config_path);
    if let Some(requested) = args.sprite {
        if !SPRITE_VARIANTS.contains(&requested.as_str()) {
            eprintln!(
                "unknown sprite variant {requested} - expected one of {}",
                SPRITE_VARIANTS.join(", ")
            );
            return ExitCode::from(2);
        }
        config.sprite = requested;
    }
    log::info!("sprite variant: {}", config.sprite);

    let manifest = match Manifest::embedded() {
        Ok(manifest) => {
            log::info!(
                "sprite manifest: {} animations, {}px frames",
                manifest.animations.len(),
                manifest.frame_width
            );
            manifest
        }
        Err(err) => {
            // A packaging bug, not a user error: fail loudly rather than
            // showing an invisible crab.
            eprintln!("sprite manifest is unusable ({err}); the build is broken");
            return ExitCode::FAILURE;
        }
    };

    let inbox = CrabConfig::inbox_dir(CrabConfig::inside_flatpak());
    log::info!("watching {}", inbox.display());
    let mut tracker = SessionTracker::new(inbox);
    tracker.set_stale_timeout_ms(i64::from(config.stale_timeout_minutes) * 60 * 1000);
    tracker.set_inbox_budget(
        i64::from(config.inbox_max_megabytes) * 1024 * 1024,
        i64::from(config.inbox_max_age_minutes) * 60 * 1000,
    );

    let brain = Brain::new(
        manifest,
        config.crab_scale as f32,
        config.sleep_corner.clone(),
        config.reactions.clone(),
    );

    let mode = match (args.demo, args.replay) {
        (true, Some(_)) => {
            eprintln!("--demo and --replay are mutually exclusive");
            return ExitCode::from(2);
        }
        (true, None) => Mode::Demo { index: 0, last: None },
        (false, Some(path)) => match app::read_replay(&path) {
            Ok(lines) => Mode::Replay { lines, index: 0, last: None },
            Err(err) => {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
        },
        (false, None) => {
            // Only the live mode touches the inbox.
            tracker.start();
            Mode::Live
        }
    };

    let parts = CoreParts { config, config_path, tracker, brain, mode };
    if let Err(err) = platform::run(parts) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
