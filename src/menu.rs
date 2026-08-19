// SPDX-License-Identifier: MIT
//
// The right-click menu.
//
// Drawn into the (deliberately oversized) layer surface rather than as a real
// popup: an xdg_popup parented to a layer surface is far more fragile than
// simply reserving headroom above the walking band and painting into it. That
// was true of the QML original and is just as true here.

use std::time::{Duration, Instant};

use crate::config::SPRITE_VARIANTS;
use crate::geom::IRect;

pub const WIDTH: f32 = 210.0;
pub const HEADER_HEIGHT: f32 = 20.0;
pub const ROW_HEIGHT: f32 = 28.0;
pub const PADDING: f32 = 12.0;
const EDGE_MARGIN: f32 = 4.0;

/// Nothing outside this window can tell us the user clicked elsewhere -- the
/// input region is ours alone -- so the menu also closes itself once the
/// pointer has been away for a moment.
const CLOSE_AFTER_LEAVE: Duration = Duration::from_millis(1500);

pub struct Menu {
    open: bool,
    /// Where the menu should point, in window coordinates.
    anchor_x: f32,
    anchor_y: f32,
    pub hovered: Option<usize>,
    /// When the pointer left the panel, or None while it is inside.
    left_at: Option<Instant>,
    /// Bumped whenever something that affects the drawing changes.
    dirty: bool,
    /// Output scale. Every coordinate here is in device pixels, so the menu
    /// keeps its physical size on a HiDPI screen instead of shrinking.
    pub scale: f32,
    /// Mirror of the pin state, so the renderer can label the row Pin/Unpin
    /// without reaching back into the Core.
    pub locked: bool,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    pub fn new() -> Self {
        Self {
            open: false,
            anchor_x: 0.0,
            anchor_y: 0.0,
            hovered: None,
            left_at: None,
            dirty: false,
            scale: 1.0,
            locked: false,
        }
    }

    pub fn row_height(&self) -> f32 {
        ROW_HEIGHT * self.scale
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Sprite variants plus the pin toggle.
    pub fn item_count() -> usize {
        SPRITE_VARIANTS.len() + 1
    }

    /// The row index of the pin toggle, the last row.
    pub fn lock_index() -> usize {
        SPRITE_VARIANTS.len()
    }

    pub fn height(&self) -> f32 {
        (HEADER_HEIGHT + Self::item_count() as f32 * ROW_HEIGHT + PADDING) * self.scale
    }

    pub fn width(&self) -> f32 {
        WIDTH * self.scale
    }

    pub fn open_at(&mut self, x: f32, y: f32) {
        self.anchor_x = x;
        self.anchor_y = y;
        self.open = true;
        self.hovered = None;
        self.left_at = None;
        self.dirty = true;
    }

    pub fn close(&mut self) {
        if !self.open {
            return;
        }
        self.open = false;
        self.hovered = None;
        self.left_at = None;
        self.dirty = true;
    }

    /// Panel rectangle in window coordinates, clamped so the menu never hangs
    /// off either end of the screen.
    pub fn panel_rect(&self, window_width: f32) -> IRect {
        let (width, height) = (self.width(), self.height());
        let margin = EDGE_MARGIN * self.scale;
        let max_x = (window_width - width - margin).max(margin);
        let x = (self.anchor_x - width / 2.0).clamp(margin, max_x);
        let y = (self.anchor_y - height).max(margin);
        IRect::from_f32(x, y, width, height)
    }

    /// Row rectangle for item `index`, in window coordinates.
    pub fn row_rect(&self, window_width: f32, index: usize) -> IRect {
        let panel = self.panel_rect(window_width);
        let y = panel.y as f32 + (HEADER_HEIGHT + index as f32 * ROW_HEIGHT) * self.scale;
        IRect::from_f32(panel.x as f32, y, self.width(), self.row_height())
    }

    /// Which row is under `(x, y)`, if any.
    pub fn hit_test(&self, window_width: f32, x: f32, y: f32) -> Option<usize> {
        if !self.open {
            return None;
        }
        (0..Self::item_count()).find(|&i| self.row_rect(window_width, i).contains(x, y))
    }

    /// Feed pointer motion. Returns true if a redraw is needed.
    pub fn pointer_moved(&mut self, window_width: f32, x: f32, y: f32, now: Instant) {
        if !self.open {
            return;
        }
        let inside = self.panel_rect(window_width).contains(x, y);
        if inside {
            self.left_at = None;
        } else if self.left_at.is_none() {
            self.left_at = Some(now);
        }

        let hovered = if inside { self.hit_test(window_width, x, y) } else { None };
        if hovered != self.hovered {
            self.hovered = hovered;
            self.dirty = true;
        }
    }

    /// The pointer left the surface entirely.
    pub fn pointer_left(&mut self, now: Instant) {
        if !self.open {
            return;
        }
        if self.left_at.is_none() {
            self.left_at = Some(now);
        }
        if self.hovered.is_some() {
            self.hovered = None;
            self.dirty = true;
        }
    }

    /// Close if the pointer has been away long enough. Returns true if closed.
    pub fn tick(&mut self, now: Instant) -> bool {
        if !self.open {
            return false;
        }
        let Some(left_at) = self.left_at else { return false };
        if now.duration_since(left_at) >= CLOSE_AFTER_LEAVE {
            self.close();
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_at(x: f32, y: f32) -> Menu {
        let mut m = Menu::new();
        m.open_at(x, y);
        m
    }

    #[test]
    fn the_lock_row_sits_below_the_variants() {
        assert_eq!(Menu::item_count(), SPRITE_VARIANTS.len() + 1);
        let m = menu_at(500.0, 300.0);
        let row = m.row_rect(1920.0, Menu::lock_index());
        let centre = (row.x as f32 + row.width as f32 / 2.0, row.y as f32 + row.height as f32 / 2.0);
        assert_eq!(m.hit_test(1920.0, centre.0, centre.1), Some(Menu::lock_index()));
    }

    #[test]
    fn starts_closed() {
        let m = Menu::new();
        assert!(!m.is_open());
        assert_eq!(m.hit_test(1920.0, 100.0, 100.0), None);
    }

    #[test]
    fn opens_centred_on_the_anchor_and_above_it() {
        let m = menu_at(500.0, 300.0);
        let panel = m.panel_rect(1920.0);
        assert_eq!(panel.x, (500.0 - WIDTH / 2.0) as i32);
        // The menu sits above the anchor, not over it.
        assert_eq!(panel.y + panel.height, 300);
    }

    #[test]
    fn clamps_to_the_left_edge() {
        let m = menu_at(10.0, 300.0);
        assert_eq!(m.panel_rect(1920.0).x, 4);
    }

    #[test]
    fn clamps_to_the_right_edge() {
        let m = menu_at(1915.0, 300.0);
        let panel = m.panel_rect(1920.0);
        assert_eq!(panel.x, (1920.0 - WIDTH - 4.0) as i32);
        assert!(panel.x + panel.width <= 1920);
    }

    #[test]
    fn clamps_downward_so_it_never_leaves_the_top() {
        // An anchor near the top would put the panel off-screen.
        let m = menu_at(500.0, 10.0);
        assert_eq!(m.panel_rect(1920.0).y, 4);
    }

    #[test]
    fn a_narrow_screen_does_not_invert_the_clamp() {
        let m = menu_at(50.0, 300.0);
        // Window narrower than the menu itself.
        let panel = m.panel_rect(100.0);
        assert_eq!(panel.x, 4, "max must not fall below the min");
    }

    #[test]
    fn hit_test_finds_each_row() {
        let m = menu_at(500.0, 300.0);
        for i in 0..Menu::item_count() {
            let row = m.row_rect(1920.0, i);
            let (cx, cy) = (row.x as f32 + 5.0, row.y as f32 + ROW_HEIGHT / 2.0);
            assert_eq!(m.hit_test(1920.0, cx, cy), Some(i), "row {i}");
        }
    }

    #[test]
    fn hit_test_misses_the_header_and_outside() {
        let m = menu_at(500.0, 300.0);
        let panel = m.panel_rect(1920.0);
        assert_eq!(m.hit_test(1920.0, panel.x as f32 + 5.0, panel.y as f32 + 2.0), None);
        assert_eq!(m.hit_test(1920.0, 0.0, 0.0), None);
    }

    #[test]
    fn hover_tracks_the_pointer_and_marks_dirty() {
        let mut m = menu_at(500.0, 300.0);
        m.take_dirty();
        let row = m.row_rect(1920.0, 1);
        m.pointer_moved(1920.0, row.x as f32 + 5.0, row.y as f32 + 5.0, Instant::now());
        assert_eq!(m.hovered, Some(1));
        assert!(m.take_dirty());
        // Same position again is not a change.
        m.pointer_moved(1920.0, row.x as f32 + 6.0, row.y as f32 + 6.0, Instant::now());
        assert!(!m.take_dirty());
    }

    #[test]
    fn closes_after_the_pointer_has_been_away() {
        let start = Instant::now();
        let mut m = menu_at(500.0, 300.0);
        m.pointer_moved(1920.0, 0.0, 0.0, start);
        assert!(!m.tick(start));
        assert!(m.is_open());
        assert!(m.tick(start + CLOSE_AFTER_LEAVE));
        assert!(!m.is_open());
    }

    #[test]
    fn returning_to_the_panel_cancels_the_close() {
        let start = Instant::now();
        let mut m = menu_at(500.0, 300.0);
        m.pointer_moved(1920.0, 0.0, 0.0, start);
        let panel = m.panel_rect(1920.0);
        m.pointer_moved(1920.0, panel.x as f32 + 5.0, panel.y as f32 + 25.0, start);
        assert!(!m.tick(start + CLOSE_AFTER_LEAVE * 2));
        assert!(m.is_open());
    }

    #[test]
    fn leaving_the_surface_starts_the_close_timer() {
        let start = Instant::now();
        let mut m = menu_at(500.0, 300.0);
        m.pointer_left(start);
        assert!(m.tick(start + CLOSE_AFTER_LEAVE));
        assert!(!m.is_open());
    }

    #[test]
    fn a_closed_menu_never_ticks_open() {
        let mut m = Menu::new();
        assert!(!m.tick(Instant::now()));
    }
}
