// SPDX-License-Identifier: MIT
//
// Reads ~/.config/claude-crab.json. Every key is optional; a missing or broken
// file yields defaults rather than an error, because a config typo should not
// cost you the crab.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Sprite variants this build knows how to render.
///
/// Mirrors VARIANTS in tools/gen_sprites.py; adding one there means adding it
/// here too.
pub const SPRITE_VARIANTS: [&str; 3] = ["default", "fancy", "party"];

#[derive(Debug, Clone, PartialEq)]
pub struct Reactions {
    pub waiting: bool,
    pub finished: bool,
    pub error: bool,
    pub tool_flavour: bool,
}

impl Default for Reactions {
    fn default() -> Self {
        Self { waiting: true, finished: true, error: true, tool_flavour: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CrabConfig {
    pub strip_height: i32,
    /// Transparent space reserved above the strip for the right-click menu.
    pub menu_headroom: i32,
    pub crab_scale: f64,
    /// Connector name, e.g. "DP-1"; empty means primary.
    pub output: String,
    pub sleep_corner: String,
    /// Pixels to lift the strip off the bottom of the screen.
    ///
    /// Only the floating backend uses this. Layer-shell learns the panel's size
    /// from its exclusive zone, but winit exposes no work-area API, so on
    /// Windows and macOS this is how you clear the taskbar or Dock.
    pub bottom_margin: i32,
    /// Sprite variant: "default", "fancy" or "party".
    pub sprite: String,
    pub stale_timeout_minutes: i32,
    /// Inbox budget. Hooks keep writing while the crab is stopped, so the
    /// directory needs an upper bound in both age and size.
    pub inbox_max_age_minutes: i32,
    pub inbox_max_megabytes: i32,
    pub reactions: Reactions,
}

impl Default for CrabConfig {
    fn default() -> Self {
        Self {
            strip_height: 72,
            menu_headroom: 220,
            crab_scale: 1.0,
            output: String::new(),
            sleep_corner: "right".to_string(),
            bottom_margin: 0,
            sprite: "default".to_string(),
            stale_timeout_minutes: 10,
            inbox_max_age_minutes: 60,
            inbox_max_megabytes: 32,
            reactions: Reactions::default(),
        }
    }
}

impl CrabConfig {
    /// True when running inside a Flatpak sandbox.
    pub fn inside_flatpak() -> bool {
        Path::new("/.flatpak-info").exists()
    }

    pub fn default_path() -> PathBuf {
        // Windows has no XDG convention; everywhere else keeps ~/.config so the
        // file is where a Linux or macOS user would look for it.
        #[cfg(windows)]
        {
            if let Some(appdata) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
                return PathBuf::from(appdata).join("claude-crab").join("claude-crab.json");
            }
        }
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| home_dir().join(".config"));
        base.join("claude-crab.json")
    }

    /// Directory the hooks write events into.
    ///
    /// Deliberately not the sandbox state dir. The hooks run on the host --
    /// Claude Code is not sandboxed -- so they always write to the host's state
    /// directory. Inside a Flatpak, XDG_STATE_HOME points at
    /// ~/.var/app/<id>/.local/state, and honouring it would leave the crab
    /// watching an empty directory forever.
    pub fn inbox_dir(flatpak: bool) -> PathBuf {
        // An explicit override always wins: it is the escape hatch for a host
        // with a non-default XDG_STATE_HOME, which a sandbox cannot otherwise
        // discover.
        if let Some(over) = std::env::var_os("CLAUDE_CRAB_STATE_DIR")
            && !over.is_empty()
        {
            return PathBuf::from(over).join("inbox");
        }

        // Windows: the hooks write under LOCALAPPDATA, which is the closest
        // equivalent to a state directory. Both sides must agree; see
        // state_dir() in tools/crab_hooks.py.
        #[cfg(windows)]
        {
            let _ = flatpak;
            if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
                return PathBuf::from(local).join("claude-crab").join("inbox");
            }
            return home_dir().join("AppData/Local/claude-crab/inbox");
        }

        #[cfg(not(windows))]
        {
            let mut base = PathBuf::new();
            if !flatpak
                && let Some(state) = std::env::var_os("XDG_STATE_HOME")
                && !state.is_empty()
            {
                base = PathBuf::from(state);
            }
            if base.as_os_str().is_empty() {
                base = home_dir().join(".local/state");
            }
            base.join("claude-crab/inbox")
        }
    }

    pub fn load(path: &Path) -> Self {
        let mut config = Self::default();

        if !path.exists() {
            return config;
        }
        let raw = match std::fs::read(path) {
            Ok(raw) => raw,
            Err(err) => {
                log::warn!("cannot read {} ({err}) - using defaults", path.display());
                return config;
            }
        };
        let obj = match serde_json::from_slice::<Value>(&raw) {
            Ok(Value::Object(obj)) => obj,
            Ok(_) => {
                log::warn!("{} is not a JSON object - using defaults", path.display());
                return config;
            }
            Err(err) => {
                log::warn!("{} is not valid JSON - {err} - using defaults", path.display());
                return config;
            }
        };

        if let Some(v) = obj.get("stripHeight").and_then(as_i32) {
            config.strip_height = v;
        }
        if let Some(v) = obj.get("menuHeadroom").and_then(as_i32) {
            config.menu_headroom = v;
        }
        if let Some(v) = obj.get("crabScale").and_then(Value::as_f64) {
            config.crab_scale = v;
        }
        if let Some(v) = obj.get("output").and_then(Value::as_str) {
            config.output = v.to_string();
        }
        if let Some(v) = obj.get("sleepCorner").and_then(Value::as_str) {
            config.sleep_corner = v.to_string();
        }
        if let Some(v) = obj.get("bottomMargin").and_then(as_i32) {
            config.bottom_margin = v;
        }
        if let Some(v) = obj.get("sprite").and_then(Value::as_str) {
            config.sprite = v.to_string();
        }
        if let Some(v) = obj.get("staleTimeoutMinutes").and_then(as_i32) {
            config.stale_timeout_minutes = v;
        }
        if let Some(v) = obj.get("inboxMaxAgeMinutes").and_then(as_i32) {
            config.inbox_max_age_minutes = v;
        }
        if let Some(v) = obj.get("inboxMaxMegabytes").and_then(as_i32) {
            config.inbox_max_megabytes = v;
        }

        if let Some(Value::Object(reactions)) = obj.get("reactions") {
            for (key, value) in reactions {
                // Qt's toBool(true) semantics: a non-bool value reads as true.
                let on = value.as_bool().unwrap_or(true);
                match key.as_str() {
                    "waiting" => config.reactions.waiting = on,
                    "finished" => config.reactions.finished = on,
                    "error" => config.reactions.error = on,
                    "toolFlavour" => config.reactions.tool_flavour = on,
                    other => log::warn!("unknown reaction {other} - ignoring"),
                }
            }
        }

        config.validate();
        config
    }

    /// Clamp out-of-range values, loudly. A bad key costs its own setting, not
    /// the whole file.
    fn validate(&mut self) {
        if self.sleep_corner != "left" && self.sleep_corner != "right" {
            log::warn!("sleepCorner must be 'left' or 'right', got {}", self.sleep_corner);
            self.sleep_corner = "right".to_string();
        }
        if !SPRITE_VARIANTS.contains(&self.sprite.as_str()) {
            log::warn!(
                "unknown sprite variant {} - using default; known variants: {}",
                self.sprite,
                SPRITE_VARIANTS.join(", ")
            );
            self.sprite = "default".to_string();
        }
        if self.bottom_margin < 0 {
            log::warn!("bottomMargin cannot be negative; using 0");
            self.bottom_margin = 0;
        }
        if self.menu_headroom < 0 {
            log::warn!("menuHeadroom cannot be negative; using 0");
            self.menu_headroom = 0;
        }
        // A zero or negative budget would disable pruning entirely, which is
        // the one setting that cannot be allowed: the inbox would grow without
        // bound.
        if self.inbox_max_age_minutes < 1 {
            log::warn!("inboxMaxAgeMinutes must be at least 1; using 60");
            self.inbox_max_age_minutes = 60;
        }
        if self.inbox_max_megabytes < 1 {
            log::warn!("inboxMaxMegabytes must be at least 1; using 32");
            self.inbox_max_megabytes = 32;
        }
        if self.strip_height < 16 {
            log::warn!("stripHeight {} is too small; using 72", self.strip_height);
            self.strip_height = 72;
        }
    }

    /// Rewrite just the "sprite" key at `path`, preserving every other key.
    ///
    /// Read-modify-write rather than serialising the whole struct: the file
    /// belongs to the user, who may have keys this build does not know about.
    pub fn save_sprite(path: &Path, variant: &str) -> bool {
        let mut obj = Map::new();
        if path.exists() {
            match std::fs::read(path) {
                Ok(raw) => match serde_json::from_slice::<Value>(&raw) {
                    Ok(Value::Object(existing)) => obj = existing,
                    _ if !raw.is_empty() => {
                        log::warn!(
                            "{} is not a JSON object; refusing to overwrite it",
                            path.display()
                        );
                        return false;
                    }
                    _ => {}
                },
                Err(err) => {
                    log::warn!("cannot read {} ({err}); refusing to overwrite it", path.display());
                    return false;
                }
            }
        }

        obj.insert("sprite".to_string(), Value::String(variant.to_string()));

        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            log::warn!("cannot create {} - {err}", parent.display());
            return false;
        }

        // Write-then-rename, matching QSaveFile: a crash mid-write must not
        // truncate the user's config.
        let tmp = path.with_extension("json.tmp");
        let mut text = serde_json::to_string_pretty(&Value::Object(obj)).unwrap_or_default();
        text.push('\n');
        if let Err(err) = std::fs::write(&tmp, text) {
            log::warn!("cannot write {} - {err}", tmp.display());
            return false;
        }
        if let Err(err) = std::fs::rename(&tmp, path) {
            log::warn!("cannot replace {} - {err}", path.display());
            let _ = std::fs::remove_file(&tmp);
            return false;
        }
        true
    }
}

/// Human-readable label for a variant, for menu entries.
pub fn label_for(variant: &str) -> &str {
    match variant {
        "fancy" => "Top Hat and Monocle",
        "party" => "Party Hat",
        "default" => "Plain",
        other => other,
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Qt's `QJsonValue::toInt` accepts a JSON number and truncates it; a string
/// or bool falls through to the default.
fn as_i32(v: &Value) -> Option<i32> {
    v.as_f64().map(|f| f as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("claude-crab.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crabcfg-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_yields_defaults() {
        let cfg = CrabConfig::load(Path::new("/nonexistent/claude-crab.json"));
        assert_eq!(cfg, CrabConfig::default());
    }

    #[test]
    fn broken_json_yields_defaults() {
        let dir = tmpdir("broken");
        let path = write(&dir, "{ this is not json");
        assert_eq!(CrabConfig::load(&path), CrabConfig::default());
    }

    #[test]
    fn non_object_yields_defaults() {
        let dir = tmpdir("nonobj");
        let path = write(&dir, "[1, 2, 3]");
        assert_eq!(CrabConfig::load(&path), CrabConfig::default());
    }

    #[test]
    fn reads_every_key() {
        let dir = tmpdir("full");
        let path = write(
            &dir,
            r#"{
                "stripHeight": 96, "menuHeadroom": 100, "crabScale": 1.5,
                "output": "DP-1", "sleepCorner": "left", "sprite": "fancy", "bottomMargin": 48,
                "staleTimeoutMinutes": 5, "inboxMaxAgeMinutes": 30,
                "inboxMaxMegabytes": 8,
                "reactions": { "waiting": false, "toolFlavour": false }
            }"#,
        );
        let cfg = CrabConfig::load(&path);
        assert_eq!(cfg.strip_height, 96);
        assert_eq!(cfg.menu_headroom, 100);
        assert!((cfg.crab_scale - 1.5).abs() < f64::EPSILON);
        assert_eq!(cfg.output, "DP-1");
        assert_eq!(cfg.sleep_corner, "left");
        assert_eq!(cfg.bottom_margin, 48);
        assert_eq!(cfg.sprite, "fancy");
        assert_eq!(cfg.stale_timeout_minutes, 5);
        assert_eq!(cfg.inbox_max_age_minutes, 30);
        assert_eq!(cfg.inbox_max_megabytes, 8);
        assert!(!cfg.reactions.waiting);
        assert!(!cfg.reactions.tool_flavour);
        // Unmentioned reactions keep their default.
        assert!(cfg.reactions.finished);
        assert!(cfg.reactions.error);
    }

    #[test]
    fn clamps_out_of_range_values() {
        let dir = tmpdir("clamp");
        let path = write(
            &dir,
            r#"{
                "sleepCorner": "middle", "sprite": "nope", "menuHeadroom": -5, "bottomMargin": -9,
                "inboxMaxAgeMinutes": 0, "inboxMaxMegabytes": 0, "stripHeight": 4
            }"#,
        );
        let cfg = CrabConfig::load(&path);
        assert_eq!(cfg.sleep_corner, "right");
        assert_eq!(cfg.sprite, "default");
        assert_eq!(cfg.menu_headroom, 0);
        assert_eq!(cfg.bottom_margin, 0);
        assert_eq!(cfg.inbox_max_age_minutes, 60);
        assert_eq!(cfg.inbox_max_megabytes, 32);
        assert_eq!(cfg.strip_height, 72);
    }

    #[test]
    fn save_sprite_preserves_unknown_keys() {
        let dir = tmpdir("save");
        let path = write(&dir, r#"{"stripHeight": 80, "somethingElse": {"a": 1}}"#);
        assert!(CrabConfig::save_sprite(&path, "party"));

        let raw = std::fs::read(&path).unwrap();
        let obj: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(obj["sprite"], "party");
        assert_eq!(obj["stripHeight"], 80);
        assert_eq!(obj["somethingElse"]["a"], 1);
    }

    #[test]
    fn save_sprite_creates_a_missing_file() {
        let dir = tmpdir("create");
        let path = dir.join("nested/claude-crab.json");
        assert!(CrabConfig::save_sprite(&path, "fancy"));
        assert_eq!(CrabConfig::load(&path).sprite, "fancy");
    }

    #[test]
    fn save_sprite_refuses_to_clobber_a_non_object() {
        let dir = tmpdir("clobber");
        let path = write(&dir, "[1,2,3]");
        assert!(!CrabConfig::save_sprite(&path, "fancy"));
        // The user's file is untouched.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[1,2,3]");
    }

    #[test]
    fn labels_cover_every_variant() {
        for v in SPRITE_VARIANTS {
            assert_ne!(label_for(v), v, "variant {v} has no human-readable label");
        }
    }
}
