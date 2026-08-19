// SPDX-License-Identifier: MIT
//
// Everything the crab does that is not tied to a windowing system.
//
// The backends (layer-shell on Wayland, winit everywhere else) own a `Core` and
// do three things with it: tell it how big the strip is, ask it for a frame,
// and forward pointer input. All the state, timing, drawing and persistence
// lives here, so the two backends cannot drift apart in behaviour.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::brain::Brain;
use crate::config::{CrabConfig, SPRITE_VARIANTS};
use crate::geom::IRect;
use crate::menu::Menu;
use crate::render::{Renderer, Viewport};
use crate::tracker::{Reaction, SessionTracker};

/// How often the inbox is drained.
///
/// ponytail: a poll rather than an inotify watch. The directory is almost
/// always empty, a scan of it is a single getdents, and this removes both a
/// dependency and the "the watcher missed an event" fallback the Qt version
/// needed anyway. Raise it if it ever shows up in a profile.
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// What is driving the crab.
pub enum Mode {
    Live,
    /// Cycle every animation on a timer, so art can be tuned without running
    /// Claude Code.
    Demo { index: usize, last: Option<Instant> },
    /// Replay a recorded JSONL event log, looping.
    Replay { lines: Vec<Vec<u8>>, index: usize, last: Option<Instant> },
}

/// How much of the strip the backend can actually put on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// One surface spanning the whole strip, with an input region confined to
    /// the crab. What layer-shell gives us.
    FullStrip,
    /// A small window that follows the crab, because the platform cannot make
    /// part of a window click-through.
    FollowCrab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Other,
}

/// A frame that needs presenting.
pub struct Painted {
    /// Rectangles that changed, in surface coordinates.
    ///
    /// Only the layer-shell backend uses this; the floating backend re-uploads
    /// its (crab-sized) texture wholesale, so it has nothing to damage-track.
    #[allow(dead_code)]
    pub damage: Vec<IRect>,
    /// Where the surface sits within the strip, and how big it is.
    pub surface: IRect,
}

pub struct Core {
    pub config: CrabConfig,
    config_path: PathBuf,
    pub brain: Brain,
    pub menu: Menu,
    tracker: SessionTracker,
    mode: Mode,
    renderer: Option<Renderer>,
    layout: Layout,

    /// Strip size in device pixels, including the menu headroom.
    strip_width: i32,
    strip_height: i32,
    scale: f32,

    last_frame: Option<Instant>,
    last_poll: Instant,
    last_sweep: Instant,
    last_visual: Option<(i32, usize, i32, i32)>,
    last_surface: IRect,
    force_redraw: bool,

    /// Pointer position in strip-space device pixels.
    pointer_pos: (f32, f32),
    /// While a left-drag is in progress, how far into the crab (x) and below
    /// its top (y, strip space) the press landed, so the sprite does not snap
    /// its corner to the pointer.
    drag_offset: Option<(f32, f32)>,
}

impl Core {
    pub fn new(
        config: CrabConfig,
        config_path: PathBuf,
        tracker: SessionTracker,
        brain: Brain,
        mode: Mode,
        layout: Layout,
    ) -> Self {
        let now = Instant::now();
        let mut brain = brain;
        brain.pinned = config.lock_position;
        let mut menu = Menu::new();
        menu.locked = config.lock_position;
        Self {
            config,
            config_path,
            brain,
            menu,
            tracker,
            mode,
            renderer: None,
            layout,
            strip_width: 0,
            strip_height: 0,
            scale: 1.0,
            last_frame: None,
            last_poll: now,
            last_sweep: now,
            last_visual: None,
            last_surface: IRect::EMPTY,
            force_redraw: true,
            pointer_pos: (0.0, 0.0),
            drag_offset: None,
        }
    }

    /// Logical strip height, including the menu headroom.
    pub fn logical_strip_height(&self) -> i32 {
        self.config.strip_height + self.config.menu_headroom
    }

    /// Tell the core how big the strip is, in device pixels.
    pub fn set_geometry(&mut self, width: i32, height: i32, scale: f32) {
        let changed =
            width != self.strip_width || height != self.strip_height || scale != self.scale;
        if !changed {
            return;
        }
        self.strip_width = width;
        self.strip_height = height;
        self.scale = scale;
        self.menu.scale = scale;
        self.brain.width = width as f32;
        self.brain.height = (self.config.strip_height as f32 * scale).max(1.0);
        self.brain.crab_scale = self.config.crab_scale as f32 * scale;
        self.brain.headroom = self.strip_top();
        // A smaller screen must keep a lifted crab on it.
        self.brain.lift = self.brain.lift.clamp(0.0, self.brain.max_lift());
        self.force_redraw = true;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_scale(scale);
            renderer.invalidate();
        }
    }

    /// Where the walking band begins within strip space, in device pixels.
    pub fn strip_top(&self) -> f32 {
        (self.strip_height as f32 - self.config.strip_height as f32 * self.scale).max(0.0)
    }

    /// The character's rectangle in strip space.
    pub fn crab_rect(&self) -> IRect {
        let r = self.brain.crab_rect();
        IRect::from_f32(r.x, self.strip_top() + r.y, r.width, r.height)
            .clamp_to(self.strip_width, self.strip_height)
    }

    /// The rectangle that should accept pointer input, in strip space.
    ///
    /// Layer-shell only: the floating backend's window *is* the crab, so
    /// everything outside it is click-through by construction.
    #[allow(dead_code)]
    pub fn input_rect(&self) -> IRect {
        if self.menu.is_open() {
            // While open, everything takes input so the menu can be hovered and
            // dismissed.
            IRect::new(0, 0, self.strip_width, self.strip_height)
        } else {
            self.crab_rect()
        }
    }

    /// The part of the strip the surface has to cover this frame.
    fn surface_rect(&self) -> IRect {
        match self.layout {
            Layout::FullStrip => IRect::new(0, 0, self.strip_width, self.strip_height),
            Layout::FollowCrab => {
                let mut rect = self.crab_rect();
                if self.menu.is_open() {
                    rect = rect.union(&self.menu.panel_rect(self.strip_width as f32));
                }
                // A pixel of slack around the sprite's antialiased edge.
                rect.inflate(2).clamp_to(self.strip_width, self.strip_height)
            }
        }
    }

    pub fn pixels(&self) -> &[u8] {
        self.renderer.as_ref().map_or(&[], Renderer::pixels)
    }

    pub fn row_bytes(&self) -> usize {
        self.renderer.as_ref().map_or(0, Renderer::row_bytes)
    }

    /// Advance one frame. Returns None when nothing changed and the previous
    /// frame is still correct on screen.
    pub fn frame(&mut self) -> Option<Painted> {
        if self.strip_width <= 0 || self.strip_height <= 0 {
            return None;
        }

        let now = Instant::now();
        let dt = self.last_frame.map_or(1.0 / 60.0, |last| {
            // Cap the step so a stall does not teleport the crab.
            now.duration_since(last).as_secs_f64().min(0.1)
        });
        self.last_frame = Some(now);

        self.pump_mode(now);
        self.menu.tick(now);
        self.brain.tick(dt);

        let surface = self.surface_rect();
        if surface.is_empty() {
            return None;
        }

        // Drained unconditionally: `||` short-circuits, and a dirty flag left
        // set would force a redundant repaint on the following frame.
        let menu_dirty = self.menu.take_dirty();
        let visual = self.brain.visual_key();
        let moved = surface != self.last_surface;
        let changed =
            self.force_redraw || menu_dirty || moved || self.last_visual != Some(visual);
        if !changed {
            return None;
        }

        self.force_redraw = false;
        self.last_visual = Some(visual);
        self.last_surface = surface;

        // Repaint only when the picture actually differs. A sleeping crab
        // animates at 3fps; without this the frame callback would re-render and
        // re-upload sixty times a second to show the very same pixels.
        let scale = self.scale;
        let variant = self.config.sprite.clone();
        let renderer = match self.renderer.as_mut() {
            Some(renderer) => renderer,
            None => match Renderer::new(surface.width, surface.height, &variant, scale) {
                Ok(renderer) => self.renderer.insert(renderer),
                Err(err) => {
                    log::error!("cannot create renderer: {err}");
                    return None;
                }
            },
        };
        if let Err(err) = renderer.resize(surface.width, surface.height) {
            log::error!("cannot resize renderer: {err}");
            return None;
        }

        let view = Viewport {
            origin: (surface.x, surface.y),
            strip_width: self.strip_width as f32,
            strip_top: (self.strip_height as f32
                - self.config.strip_height as f32 * self.scale)
                .max(0.0),
        };
        let damage = renderer.render(&self.brain, &self.menu, view);
        Some(Painted { damage, surface })
    }

    fn pump_mode(&mut self, now: Instant) {
        match &mut self.mode {
            Mode::Live => {
                if now.duration_since(self.last_poll) >= POLL_INTERVAL {
                    self.last_poll = now;
                    let reactions = self.tracker.poll();
                    self.apply_reactions(&reactions);
                }
                if now.duration_since(self.last_sweep) >= SWEEP_INTERVAL {
                    self.last_sweep = now;
                    let stamp = crate::tracker::now_ms();
                    self.tracker.sweep_stale(stamp);
                    self.tracker.prune_inbox(stamp);
                }
                self.sync_tracker();
            }
            Mode::Demo { index, last } => {
                let due = last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(2));
                if due {
                    *last = Some(now);
                    let names: Vec<String> =
                        self.brain.manifest.names().iter().map(|s| s.to_string()).collect();
                    if !names.is_empty() {
                        let name = names[*index % names.len()].clone();
                        *index += 1;
                        log::info!("demo: {name}");
                        self.brain.play_reaction(&name);
                    }
                }
            }
            Mode::Replay { lines, index, last } => {
                let due = last.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(900));
                if due && !lines.is_empty() {
                    *last = Some(now);
                    let line = lines[*index % lines.len()].clone();
                    *index += 1;
                    let stamp = crate::tracker::now_ms();
                    let reactions = match serde_json::from_slice::<serde_json::Value>(&line) {
                        Ok(serde_json::Value::Object(obj)) => self.tracker.handle_event(&obj, stamp),
                        _ => Vec::new(),
                    };
                    self.apply_reactions(&reactions);
                    self.sync_tracker();
                }
            }
        }
    }

    fn sync_tracker(&mut self) {
        if self.tracker.take_dirty() {
            self.brain.session_state = self.tracker.aggregate_state();
            self.brain.tool = self.tracker.current_tool().to_string();
        }
    }

    fn apply_reactions(&mut self, reactions: &[Reaction]) {
        for reaction in reactions {
            match reaction {
                Reaction::Errored if self.config.reactions.error => {
                    self.brain.play_reaction("tumble");
                }
                Reaction::Finished if self.config.reactions.finished => {
                    self.brain.play_reaction("celebrate");
                }
                _ => {}
            }
        }
    }

    // --- input, in strip-space device pixels --------------------------------

    pub fn pointer_moved(&mut self, x: f32, y: f32) {
        self.pointer_pos = (x, y);
        if let Some((dx, dy)) = self.drag_offset {
            self.brain.x = (x - dx).clamp(0.0, self.brain.max_x());
            // The grabbed point follows the pointer vertically too: the crab's
            // floor-level top minus its new top is the lift.
            let h = self.brain.frame_height() * self.brain.crab_scale;
            let floor_top = self.strip_top() + self.brain.height - h;
            self.brain.lift = (floor_top - (y - dy)).clamp(0.0, self.brain.max_lift());
            return;
        }
        self.menu.pointer_moved(self.strip_width as f32, x, y, Instant::now());
    }

    pub fn pointer_left(&mut self) {
        // A grab normally outlives a Leave, but the floating backend can lose
        // the pointer mid-drag; dropping the crab is better than a stuck grab.
        self.end_drag();
        self.menu.pointer_left(Instant::now());
    }

    pub fn pointer_released(&mut self, button: Button) {
        if button == Button::Left {
            self.end_drag();
        }
    }

    fn end_drag(&mut self) {
        if self.drag_offset.take().is_some() {
            self.brain.held = false;
        }
    }

    pub fn pointer_pressed(&mut self, button: Button) {
        let (x, y) = self.pointer_pos;
        let width = self.strip_width as f32;

        if self.menu.is_open() {
            if matches!(button, Button::Left | Button::Right)
                && let Some(index) = self.menu.hit_test(width, x, y)
            {
                if index == Menu::lock_index() {
                    self.toggle_lock();
                } else {
                    let variant = SPRITE_VARIANTS[index];
                    self.set_variant(variant);
                }
                self.menu.close();
                return;
            }
            // A click anywhere else dismisses it, matching a normal menu.
            if !self.menu.panel_rect(width).contains(x, y) {
                self.menu.close();
            }
            return;
        }

        if !self.crab_rect().contains(x, y) {
            return;
        }
        match button {
            Button::Right => {
                // Anchored to the crab's centre, opening upward into the headroom.
                let r = self.brain.crab_rect();
                self.menu.open_at(r.x + r.width / 2.0, self.strip_top());
            }
            Button::Left if !self.brain.pinned => {
                let top = self.strip_top() + self.brain.crab_rect().y;
                self.drag_offset = Some((x - self.brain.x, y - top));
                self.brain.held = true;
            }
            _ => {}
        }
    }

    /// Pin the crab against drags, or make it draggable again.
    fn toggle_lock(&mut self) {
        let locked = !self.brain.pinned;
        self.brain.pinned = locked;
        self.menu.locked = locked;
        self.config.lock_position = locked;
        self.end_drag();

        // Persisted immediately, like the sprite choice: this runs as a
        // background service, so a lock that vanished on the next restart
        // would read as the toggle not working.
        if !CrabConfig::save_lock(&self.config_path, locked) {
            log::warn!(
                "lock toggled but could not be saved to {} - it will revert on restart",
                self.config_path.display()
            );
        }
    }

    pub fn set_variant(&mut self, variant: &str) {
        if variant == self.config.sprite {
            return;
        }
        if !SPRITE_VARIANTS.contains(&variant) {
            log::warn!("refusing to switch to unknown sprite variant {variant}");
            return;
        }
        self.config.sprite = variant.to_string();
        if let Some(renderer) = self.renderer.as_mut()
            && let Err(err) = renderer.set_variant(variant)
        {
            log::error!("cannot load {variant} sheet: {err}");
            return;
        }
        // The sheet changed under an unchanged visual key, so the frame loop
        // has to be told explicitly that this frame differs.
        self.force_redraw = true;

        // Persisted immediately: this runs as a background service, so a choice
        // that vanished on the next restart would read as the switch not
        // working.
        if !CrabConfig::save_sprite(&self.config_path, variant) {
            log::warn!(
                "sprite switched to {variant} but could not be saved to {} \
                 - it will revert on restart",
                self.config_path.display()
            );
        }
    }
}

/// Read a JSONL replay log into one payload per line.
pub fn read_replay(path: &Path) -> Result<Vec<Vec<u8>>, String> {
    let raw =
        std::fs::read(path).map_err(|e| format!("cannot open replay file {}: {e}", path.display()))?;
    let lines: Vec<Vec<u8>> = raw
        .split(|b| *b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line).to_vec())
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(format!("replay file {} is empty", path.display()));
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sprites::Manifest;
    use crate::tracker::State;

    fn core(layout: Layout) -> Core {
        let config = CrabConfig::default();
        let brain = Brain::new(
            Manifest::embedded().unwrap(),
            1.0,
            config.sleep_corner.clone(),
            config.reactions.clone(),
        );
        let tracker = SessionTracker::new(PathBuf::from("/nonexistent-inbox"));
        let mut core = Core::new(
            config,
            std::env::temp_dir().join("crab-core-test.json"),
            tracker,
            brain,
            Mode::Live,
            layout,
        );
        // 800x292 strip: 72 walking band under 220 of headroom.
        core.set_geometry(800, 292, 1.0);
        core
    }

    #[test]
    fn strip_top_sits_above_the_walking_band() {
        let c = core(Layout::FullStrip);
        assert_eq!(c.strip_top(), 220.0);
    }

    #[test]
    fn the_full_strip_layout_covers_everything() {
        let mut c = core(Layout::FullStrip);
        let painted = c.frame().expect("first frame always paints");
        assert_eq!(painted.surface, IRect::new(0, 0, 800, 292));
    }

    #[test]
    fn the_follow_layout_tracks_the_crab_and_stays_small() {
        let mut c = core(Layout::FollowCrab);
        c.brain.x = 300.0;
        let painted = c.frame().expect("first frame always paints");
        // Just the crab plus a little slack, not the whole strip.
        assert!(painted.surface.width < 100, "{:?}", painted.surface);
        assert!(painted.surface.height < 100, "{:?}", painted.surface);
        assert!(painted.surface.x >= 296 && painted.surface.x <= 302, "{:?}", painted.surface);
    }

    #[test]
    fn the_follow_layout_grows_to_fit_an_open_menu() {
        let mut c = core(Layout::FollowCrab);
        c.brain.x = 300.0;
        let small = c.frame().unwrap().surface;

        c.menu.open_at(332.0, c.strip_top());
        let big = c.frame().unwrap().surface;
        assert!(big.height > small.height, "menu should enlarge the window");
        assert!(big.width >= crate::menu::WIDTH as i32, "menu should fit horizontally");
    }

    #[test]
    fn a_static_crab_stops_producing_frames() {
        let mut c = core(Layout::FullStrip);
        // Idle at the corner: sleeping, not moving.
        c.brain.session_state = State::Idle;
        c.brain.x = c.brain.max_x();
        assert!(c.frame().is_some(), "the first frame always paints");

        // Burn through the sleep animation, then confirm it goes quiet.
        let mut painted = 0;
        for _ in 0..200 {
            if c.frame().is_some() {
                painted += 1;
            }
        }
        assert!(painted < 200, "a sleeping crab must not repaint every frame");
    }

    #[test]
    fn input_rect_is_the_crab_until_the_menu_opens() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        let crab = c.input_rect();
        assert!(crab.width < 100, "input should be confined to the character");

        c.menu.open_at(132.0, c.strip_top());
        assert_eq!(c.input_rect(), IRect::new(0, 0, 800, 292), "an open menu takes it all");
    }

    #[test]
    fn right_clicking_the_crab_opens_the_menu() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        let crab = c.crab_rect();
        c.pointer_moved(crab.x as f32 + 10.0, crab.y as f32 + 10.0);
        c.pointer_pressed(Button::Right);
        assert!(c.menu.is_open());
    }

    #[test]
    fn left_clicking_the_crab_does_nothing() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        let crab = c.crab_rect();
        c.pointer_moved(crab.x as f32 + 10.0, crab.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        assert!(!c.menu.is_open());
    }

    #[test]
    fn right_clicking_empty_strip_does_nothing() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        c.pointer_moved(700.0, 250.0);
        c.pointer_pressed(Button::Right);
        assert!(!c.menu.is_open());
    }

    #[test]
    fn clicking_outside_an_open_menu_dismisses_it() {
        let mut c = core(Layout::FullStrip);
        c.menu.open_at(400.0, c.strip_top());
        c.pointer_moved(10.0, 10.0);
        c.pointer_pressed(Button::Left);
        assert!(!c.menu.is_open());
    }

    #[test]
    fn choosing_a_menu_row_switches_and_closes() {
        let mut c = core(Layout::FullStrip);
        let path = std::env::temp_dir().join(format!("crab-core-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        c.config_path = path.clone();

        c.menu.open_at(400.0, c.strip_top());
        // Row 1 is "fancy" in SPRITE_VARIANTS order.
        let row = c.menu.row_rect(800.0, 1);
        c.pointer_moved(row.x as f32 + 5.0, row.y as f32 + 5.0);
        c.pointer_pressed(Button::Left);

        assert_eq!(c.config.sprite, "fancy");
        assert!(!c.menu.is_open());
        assert_eq!(CrabConfig::load(&path).sprite, "fancy", "the choice must persist");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dragging_moves_the_crab_and_release_resumes_the_patrol() {
        let mut c = core(Layout::FullStrip);
        c.brain.session_state = State::Working;
        c.brain.tool = "Bash".to_string();
        c.brain.x = 100.0;

        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(300.0, r.y as f32 + 10.0);
        // The grab offset holds: the crab's left edge trails the pointer by
        // the 10px the press landed inside it.
        assert!((c.brain.x - 290.0).abs() < 0.5, "got {}", c.brain.x);

        // Grabbed: the patrol must not fight the hand.
        c.brain.tick(0.1);
        assert!((c.brain.x - 290.0).abs() < 0.5, "moved while held: {}", c.brain.x);

        c.pointer_released(Button::Left);
        c.brain.tick(0.1);
        assert!((c.brain.x - 290.0).abs() > 0.5, "release should resume the patrol");
    }

    #[test]
    fn a_drag_never_leaves_the_strip() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(-500.0, r.y as f32 + 10.0);
        assert_eq!(c.brain.x, 0.0);
        c.pointer_moved(5000.0, r.y as f32 + 10.0);
        assert_eq!(c.brain.x, c.brain.max_x());
    }

    #[test]
    fn a_pinned_crab_cannot_be_dragged() {
        let mut c = core(Layout::FullStrip);
        c.brain.x = 100.0;
        c.brain.pinned = true;
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(300.0, r.y as f32 + 10.0);
        assert_eq!(c.brain.x, 100.0, "a pinned crab must ignore the drag");
    }

    #[test]
    fn a_vertical_drag_lifts_the_crab_and_it_stays_where_dropped() {
        let mut c = core(Layout::FullStrip);
        c.brain.session_state = State::Working;
        c.brain.tool = "Bash".to_string();
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0 - 50.0);
        assert_eq!(c.brain.lift, 50.0, "the crab follows the pointer up");
        c.pointer_released(Button::Left);
        for _ in 0..10 {
            c.brain.tick(0.1);
        }
        assert_eq!(c.brain.lift, 50.0, "it roams at the height it was dropped");
        assert_ne!(c.brain.x, r.x as f32, "still patrolling horizontally");
    }

    #[test]
    fn a_lifted_crab_cannot_leave_the_surface() {
        let mut c = core(Layout::FullStrip);
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(r.x as f32 + 10.0, -500.0);
        assert_eq!(c.brain.lift, c.brain.max_lift(), "clamped at the surface top");
        assert!(c.crab_rect().y >= 0);
        c.pointer_moved(r.x as f32 + 10.0, 5000.0);
        assert_eq!(c.brain.lift, 0.0, "clamped at the floor");
    }

    #[test]
    fn the_crab_can_be_dropped_anywhere_on_screen() {
        let mut c = core(Layout::FullStrip);
        // A full-screen surface, as the layer-shell backend now creates.
        c.set_geometry(800, 600, 1.0);
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(400.0, 10.0);
        c.pointer_released(Button::Left);
        let dropped = c.crab_rect();
        assert_eq!(dropped.y, 0, "the drag reaches the very top of the screen");
        assert!(dropped.x > 300, "and lands at the pointer's x");
    }

    #[test]
    fn a_pinned_crab_ignores_vertical_drags() {
        let mut c = core(Layout::FullStrip);
        c.brain.pinned = true;
        let r = c.crab_rect();
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 + 10.0);
        c.pointer_pressed(Button::Left);
        c.pointer_moved(r.x as f32 + 10.0, r.y as f32 - 100.0);
        assert_eq!(c.brain.lift, 0.0, "pinned means no vertical drag either");
    }

    #[test]
    fn the_lock_row_toggles_and_persists() {
        let mut c = core(Layout::FullStrip);
        let path = std::env::temp_dir().join(format!("crab-lock-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        c.config_path = path.clone();
        c.brain.x = 250.0;

        c.menu.open_at(400.0, c.strip_top());
        let row = c.menu.row_rect(800.0, Menu::lock_index());
        c.pointer_moved(row.x as f32 + 5.0, row.y as f32 + 5.0);
        c.pointer_pressed(Button::Left);

        assert!(c.brain.pinned, "the lock row should pin the crab");
        assert!(c.menu.locked);
        assert!(!c.menu.is_open());
        let saved = CrabConfig::load(&path);
        assert!(saved.lock_position, "the lock must persist");

        c.menu.open_at(400.0, c.strip_top());
        c.pointer_moved(row.x as f32 + 5.0, row.y as f32 + 5.0);
        c.pointer_pressed(Button::Left);
        assert!(!c.brain.pinned, "the same row unlocks");
        assert!(!CrabConfig::load(&path).lock_position);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_locked_config_starts_the_crab_pinned() {
        let mut config = CrabConfig::default();
        config.lock_position = true;
        let brain = Brain::new(
            Manifest::embedded().unwrap(),
            1.0,
            config.sleep_corner.clone(),
            config.reactions.clone(),
        );
        let mut c = Core::new(
            config,
            std::env::temp_dir().join("crab-lock-start.json"),
            SessionTracker::new(PathBuf::from("/nonexistent-inbox")),
            brain,
            Mode::Live,
            Layout::FullStrip,
        );
        c.set_geometry(800, 292, 1.0);
        assert!(c.brain.pinned);
        assert!(c.menu.locked);
    }

    #[test]
    fn demo_mode_cycles_animations() {
        let config = CrabConfig::default();
        let brain = Brain::new(
            Manifest::embedded().unwrap(),
            1.0,
            config.sleep_corner.clone(),
            config.reactions.clone(),
        );
        let mut c = Core::new(
            config,
            std::env::temp_dir().join("crab-demo.json"),
            SessionTracker::new(PathBuf::from("/nonexistent")),
            brain,
            Mode::Demo { index: 0, last: None },
            Layout::FullStrip,
        );
        c.set_geometry(800, 292, 1.0);
        c.frame();
        // The first demo tick fires immediately and picks the first animation.
        assert_eq!(c.brain.active_animation(), "sleep");
    }

    #[test]
    fn replay_reader_splits_and_trims() {
        let dir = std::env::temp_dir().join(format!("crabreplay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("log.jsonl");
        std::fs::write(&path, "{\"a\":1}\r\n\n{\"b\":2}\n").unwrap();

        let lines = read_replay(&path).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], b"{\"a\":1}");
        assert_eq!(lines[1], b"{\"b\":2}");
    }

    #[test]
    fn an_empty_replay_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!("crabreplay-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.jsonl");
        std::fs::write(&path, "\n\n").unwrap();
        assert!(read_replay(&path).is_err());
    }

    #[test]
    fn a_missing_replay_file_is_an_error() {
        assert!(read_replay(Path::new("/nonexistent/log.jsonl")).is_err());
    }

    #[test]
    fn hidpi_scales_the_crab_and_the_band() {
        let mut c = core(Layout::FullStrip);
        c.set_geometry(1600, 584, 2.0);
        // The walking band doubles, and so does the sprite.
        assert_eq!(c.strip_top(), 584.0 - 144.0);
        assert_eq!(c.brain.crab_scale, 2.0);
        assert_eq!(c.crab_rect().width, 128);
    }
}
