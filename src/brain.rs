// SPDX-License-Identifier: MIT
//
// Maps tracker state to an animation and drives the crab across the strip.
// This is the only place that knows what "Bash means scuttle" implies.

use crate::config::Reactions;
use crate::sprites::Manifest;
use crate::tracker::State;

/// Pixels per second for each gait.
const SPEED_WALK: f32 = 60.0;
const SPEED_SCUTTLE: f32 = 150.0;
const SPEED_CREEP: f32 = 24.0;

/// A rectangle in strip coordinates, y measured from the top of the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub struct Brain {
    pub manifest: Manifest,
    /// Strip width in pixels.
    pub width: f32,
    /// Strip height in pixels.
    pub height: f32,
    pub crab_scale: f32,
    pub sleep_corner: String,
    pub reactions: Reactions,

    pub session_state: State,
    pub tool: String,

    /// A one-shot reaction currently playing, or None when free.
    reaction: Option<String>,

    pub x: f32,
    /// +1 walking right, -1 walking left.
    pub direction: i32,

    /// Pinned by the user: drags are ignored, but it roams as usual.
    pub pinned: bool,
    /// Height above the strip floor the crab was dropped at, in device pixels.
    /// It roams horizontally at this height until dragged again.
    pub lift: f32,
    /// Vertical room above the walking band (the menu headroom), in device
    /// pixels. Bounds how far up a drag can carry the crab.
    pub headroom: f32,
    /// Mid-drag: the pointer owns the position until the button is released.
    pub held: bool,

    /// The animation currently on screen, and where it is in its frame list.
    current: String,
    frame: usize,
    frame_accum: f64,
    /// Set when a non-looping animation has reached its last frame.
    exhausted: bool,
}

impl Brain {
    pub fn new(manifest: Manifest, crab_scale: f32, sleep_corner: String, reactions: Reactions) -> Self {
        Self {
            manifest,
            width: 0.0,
            height: 0.0,
            crab_scale,
            sleep_corner,
            reactions,
            session_state: State::Idle,
            tool: String::new(),
            reaction: None,
            x: 0.0,
            direction: 1,
            pinned: false,
            lift: 0.0,
            headroom: 0.0,
            held: false,
            current: String::new(),
            frame: 0,
            frame_accum: 0.0,
            exhausted: false,
        }
    }

    pub fn frame_width(&self) -> f32 {
        self.manifest.frame_width as f32
    }

    pub fn frame_height(&self) -> f32 {
        self.manifest.frame_height as f32
    }

    /// The rightmost x the crab may occupy.
    ///
    /// Uses the unscaled frame width, matching the original QML where `scale`
    /// was a visual transform applied on top of the item's layout width.
    pub fn max_x(&self) -> f32 {
        (self.width - self.frame_width()).max(0.0)
    }

    /// The highest lift that keeps the crab fully on the surface.
    pub fn max_lift(&self) -> f32 {
        (self.headroom + self.height - self.frame_height() * self.crab_scale).max(0.0)
    }

    pub fn corner_x(&self) -> f32 {
        if self.sleep_corner == "left" { 0.0 } else { self.max_x() }
    }

    pub fn at_corner(&self) -> bool {
        (self.x - self.corner_x()).abs() < 1.0
    }

    pub fn facing_right(&self) -> bool {
        self.direction > 0
    }

    /// Everything that affects what the crab looks like this frame.
    ///
    /// The frame loop compares this against the previous frame and skips the
    /// repaint and buffer upload when it is unchanged -- a sleeping crab
    /// animates at 3fps, and re-uploading a full-width strip 60 times a second
    /// to show the same picture is pure waste.
    pub fn visual_key(&self) -> (i32, usize, i32, i32) {
        let row = self.manifest.get(&self.current).map_or(-1, |a| a.row);
        (row, self.frame, self.x.round() as i32, self.direction)
    }

    /// Start a one-shot reaction. Unknown names are refused rather than
    /// leaving the crab stuck on an animation that will never finish.
    pub fn play_reaction(&mut self, name: &str) {
        if self.manifest.get(name).is_none() {
            log::warn!("refusing to play unknown reaction {name}");
            return;
        }
        self.reaction = Some(name.to_string());
    }

    fn gait_for_tool(&self, name: &str) -> &'static str {
        if !self.reactions.tool_flavour {
            return "walk";
        }
        match name {
            "Bash" | "BashOutput" => "scuttle",
            "Edit" | "Write" | "NotebookEdit" => "creep",
            "Read" | "Grep" | "Glob" => "walk",
            // Between tools: the model itself is working.
            "" => "think",
            _ => "walk",
        }
    }

    fn speed_for(gait: &str) -> f32 {
        match gait {
            "scuttle" => SPEED_SCUTTLE,
            "creep" => SPEED_CREEP,
            "walk" => SPEED_WALK,
            _ => 0.0,
        }
    }

    /// The animation the crab should be showing, ignoring reactions.
    pub fn base_animation(&self) -> &str {
        match self.session_state {
            State::WaitingInput => {
                if self.reactions.waiting {
                    "wave"
                } else {
                    "think"
                }
            }
            State::Working => self.gait_for_tool(&self.tool),
            State::Idle => {
                if self.at_corner() {
                    "sleep"
                } else {
                    "walk"
                }
            }
        }
    }

    /// The animation actually on screen: a reaction if one is playing.
    pub fn active_animation(&self) -> &str {
        match &self.reaction {
            Some(name) => name,
            None => self.base_animation(),
        }
    }

    /// The character's current rectangle, in strip coordinates.
    ///
    /// Deviation from the QML original, which computed the top as
    /// `crab.y + height - crab.height * scale` -- using the strip height where
    /// it meant the frame height, so the input region sat 8px low whenever
    /// stripHeight and frameHeight differed (the default 72 vs 64). The crab is
    /// bottom-anchored, so its visual top is `height - frameHeight * scale`.
    pub fn crab_rect(&self) -> Rect {
        let w = self.frame_width() * self.crab_scale;
        let h = self.frame_height() * self.crab_scale;
        Rect { x: self.x, y: self.height - h - self.lift, width: w, height: h }
    }

    /// Advance position and animation by `dt` seconds.
    pub fn tick(&mut self, dt: f64) {
        self.step_position(dt as f32);
        self.step_animation(dt);
    }

    fn step_position(&mut self, dt: f32) {
        // Grabbed: the user owns the position, animations play in place.
        if self.held {
            return;
        }
        // Reactions play in place.
        if self.reaction.is_some() {
            return;
        }
        // Waiting means stopped, facing the user.
        if self.session_state == State::WaitingInput {
            return;
        }

        if self.session_state == State::Idle {
            // Head for the sleeping corner and settle there.
            let target = self.corner_x();
            if self.at_corner() {
                self.x = target;
                return;
            }
            self.direction = if target > self.x { 1 } else { -1 };
            let step = SPEED_WALK * dt;
            if (target - self.x).abs() <= step {
                self.x = target;
            } else {
                self.x += self.direction as f32 * step;
            }
            return;
        }

        // Working: patrol the strip, bouncing off both ends.
        let speed = Self::speed_for(self.base_animation());
        if speed == 0.0 {
            // Thinking: planted.
            return;
        }
        self.x += self.direction as f32 * speed * dt;
        let max_x = self.max_x();
        if self.x <= 0.0 {
            self.x = 0.0;
            self.direction = 1;
        } else if self.x >= max_x {
            self.x = max_x;
            self.direction = -1;
        }
    }

    fn step_animation(&mut self, dt: f64) {
        let wanted = self.active_animation().to_string();
        if wanted != self.current {
            // Restart from frame 0, otherwise a one-shot re-triggered while
            // already showing would never finish again.
            self.current = wanted;
            self.frame = 0;
            self.frame_accum = 0.0;
            self.exhausted = false;
        }

        let Some(anim) = self.manifest.get(&self.current) else {
            log::warn!("unknown animation {}", self.current);
            return;
        };
        let (frames, fps, looping) = (anim.frames, anim.fps, anim.looping);

        if self.exhausted {
            return;
        }

        self.frame_accum += dt;
        let seconds_per_frame = 1.0 / fps;
        // Guard against a long stall producing a huge catch-up loop.
        let mut budget = frames.max(1) * 2;
        while self.frame_accum >= seconds_per_frame && budget > 0 {
            self.frame_accum -= seconds_per_frame;
            budget -= 1;
            if self.frame + 1 < frames {
                self.frame += 1;
            } else if looping {
                self.frame = 0;
            } else {
                self.exhausted = true;
                // The reaction that just ended releases the crab.
                if self.reaction.as_deref() == Some(self.current.as_str()) {
                    self.reaction = None;
                }
                break;
            }
        }
        if budget == 0 {
            self.frame_accum = 0.0;
        }
    }

    /// Source rectangle on the sheet for the frame currently showing:
    /// (x, y, width, height).
    pub fn source_frame(&self) -> (i32, i32, i32, i32) {
        let fw = self.manifest.frame_width;
        let fh = self.manifest.frame_height;
        let row = self.manifest.get(&self.current).map_or(0, |a| a.row);
        (self.frame as i32 * fw, row * fh, fw, fh)
    }

    #[cfg(test)]
    fn frame_index(&self) -> usize {
        self.frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brain() -> Brain {
        let mut b = Brain::new(
            Manifest::embedded().unwrap(),
            1.0,
            "right".to_string(),
            Reactions::default(),
        );
        b.width = 800.0;
        b.height = 72.0;
        b
    }

    #[test]
    fn a_pinned_brain_still_patrols() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.x = 100.0;
        b.pinned = true;
        for _ in 0..60 {
            b.tick(1.0 / 60.0);
        }
        assert_ne!(b.x, 100.0, "pinning blocks drags, not roaming");
    }

    #[test]
    fn a_pinned_idle_brain_still_walks_to_its_corner() {
        let mut b = brain();
        b.session_state = State::Idle;
        b.x = 100.0; // nowhere near the right-hand sleeping corner
        b.pinned = true;
        b.tick(0.1);
        assert!(b.x > 100.0, "must head for the corner even when pinned");
        assert_eq!(b.base_animation(), "walk", "still walking, not asleep in place");
    }

    #[test]
    fn a_held_brain_stays_put_until_released() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.x = 100.0;
        b.held = true;
        b.tick(0.1);
        assert_eq!(b.x, 100.0, "a grabbed crab must not walk out of the hand");
        b.held = false;
        b.tick(0.1);
        assert_ne!(b.x, 100.0, "release should resume the patrol");
    }

    #[test]
    fn idle_away_from_the_corner_walks_there() {
        let mut b = brain();
        b.x = 100.0;
        b.session_state = State::Idle;
        assert_eq!(b.base_animation(), "walk");
        // The sleeping corner is on the right by default.
        b.tick(0.1);
        assert!(b.x > 100.0, "should head right, got {}", b.x);
    }

    #[test]
    fn idle_at_the_corner_sleeps_and_stops() {
        let mut b = brain();
        b.session_state = State::Idle;
        b.x = b.max_x();
        assert!(b.at_corner());
        assert_eq!(b.base_animation(), "sleep");
        let before = b.x;
        b.tick(0.5);
        assert_eq!(b.x, before);
    }

    #[test]
    fn left_corner_is_honoured() {
        let mut b = brain();
        b.sleep_corner = "left".to_string();
        b.session_state = State::Idle;
        b.x = 300.0;
        b.tick(0.1);
        assert!(b.x < 300.0, "should head left, got {}", b.x);
        assert_eq!(b.corner_x(), 0.0);
    }

    #[test]
    fn idle_walk_does_not_overshoot_the_corner() {
        let mut b = brain();
        b.session_state = State::Idle;
        b.x = b.max_x() - 1.0;
        // A long dt would sail past the corner without the clamp.
        b.tick(10.0);
        assert_eq!(b.x, b.max_x());
    }

    #[test]
    fn tools_pick_their_gait() {
        let mut b = brain();
        b.session_state = State::Working;
        for (tool, gait) in [
            ("Bash", "scuttle"),
            ("BashOutput", "scuttle"),
            ("Edit", "creep"),
            ("Write", "creep"),
            ("NotebookEdit", "creep"),
            ("Read", "walk"),
            ("Grep", "walk"),
            ("Glob", "walk"),
            ("", "think"),
            ("SomethingNew", "walk"),
        ] {
            b.tool = tool.to_string();
            assert_eq!(b.base_animation(), gait, "tool {tool}");
        }
    }

    #[test]
    fn tool_flavour_off_flattens_every_gait_to_walk() {
        let mut b = brain();
        b.reactions.tool_flavour = false;
        b.session_state = State::Working;
        for tool in ["Bash", "Edit", "Read", ""] {
            b.tool = tool.to_string();
            assert_eq!(b.base_animation(), "walk", "tool {tool}");
        }
    }

    #[test]
    fn thinking_is_planted() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = String::new();
        assert_eq!(b.base_animation(), "think");
        b.x = 100.0;
        b.tick(0.5);
        assert_eq!(b.x, 100.0);
    }

    #[test]
    fn working_bounces_off_both_ends() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.x = b.max_x() - 1.0;
        b.direction = 1;
        b.tick(0.5);
        assert_eq!(b.x, b.max_x());
        assert_eq!(b.direction, -1);

        b.x = 1.0;
        b.tick(0.5);
        assert_eq!(b.x, 0.0);
        assert_eq!(b.direction, 1);
    }

    #[test]
    fn waiting_waves_in_place() {
        let mut b = brain();
        b.session_state = State::WaitingInput;
        assert_eq!(b.base_animation(), "wave");
        b.x = 200.0;
        b.tick(0.5);
        assert_eq!(b.x, 200.0);
    }

    #[test]
    fn waiting_reaction_off_falls_back_to_think() {
        let mut b = brain();
        b.reactions.waiting = false;
        b.session_state = State::WaitingInput;
        assert_eq!(b.base_animation(), "think");
    }

    #[test]
    fn a_reaction_overrides_the_base_animation_and_pins_position() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.x = 100.0;
        b.play_reaction("celebrate");
        assert_eq!(b.active_animation(), "celebrate");
        b.tick(0.01);
        assert_eq!(b.x, 100.0, "reactions play in place");
    }

    #[test]
    fn a_one_shot_reaction_clears_itself_when_it_ends() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.play_reaction("celebrate");

        let anim = b.manifest.get("celebrate").unwrap().clone();
        assert!(!anim.looping);
        // Run just past the full length of the animation.
        let total = anim.frames as f64 / anim.fps;
        let mut elapsed = 0.0;
        while elapsed < total + 0.5 && b.reaction.is_some() {
            b.tick(1.0 / 60.0);
            elapsed += 1.0 / 60.0;
        }
        assert!(b.reaction.is_none(), "celebrate should have released the crab");
        assert_eq!(b.active_animation(), "scuttle");
    }

    #[test]
    fn a_looping_animation_wraps_forever() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        let anim = b.manifest.get("scuttle").unwrap().clone();
        assert!(anim.looping);
        for _ in 0..1000 {
            b.tick(1.0 / 60.0);
            assert!(b.frame_index() < anim.frames);
        }
    }

    #[test]
    fn unknown_reactions_are_refused() {
        let mut b = brain();
        b.play_reaction("moonwalk");
        assert!(b.reaction.is_none());
    }

    #[test]
    fn crab_rect_is_bottom_anchored() {
        let mut b = brain();
        b.x = 40.0;
        let r = b.crab_rect();
        assert_eq!(r.x, 40.0);
        assert_eq!(r.width, 64.0);
        assert_eq!(r.height, 64.0);
        // Strip is 72 tall, frame 64 -> the crab's top is 8px down.
        assert_eq!(r.y, 8.0);

        b.crab_scale = 0.5;
        let r = b.crab_rect();
        assert_eq!(r.height, 32.0);
        assert_eq!(r.y, 40.0, "a scaled crab still stands on the floor");
    }

    #[test]
    fn source_frame_follows_the_row_and_frame() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.tick(0.0);
        let scuttle_row = b.manifest.get("scuttle").unwrap().row;
        let (x, y, w, h) = b.source_frame();
        assert_eq!((x, y, w, h), (0, scuttle_row * 64, 64, 64));

        // Advance exactly one frame.
        let fps = b.manifest.get("scuttle").unwrap().fps;
        b.tick(1.0 / fps + 1e-6);
        let (x, _, _, _) = b.source_frame();
        assert_eq!(x, 64);
    }

    #[test]
    fn facing_follows_direction() {
        let mut b = brain();
        b.direction = 1;
        assert!(b.facing_right());
        b.direction = -1;
        assert!(!b.facing_right());
    }

    #[test]
    fn a_narrow_strip_does_not_produce_a_negative_max_x() {
        let mut b = brain();
        b.width = 10.0;
        assert_eq!(b.max_x(), 0.0);
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.tick(0.5);
        assert_eq!(b.x, 0.0);
    }

    #[test]
    fn a_long_stall_does_not_spin_the_frame_clock() {
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        // Simulate the process being stopped for a minute.
        b.tick(60.0);
        assert!(b.frame_index() < b.manifest.get("scuttle").unwrap().frames);
    }
}
