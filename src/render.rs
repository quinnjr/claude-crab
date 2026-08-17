// SPDX-License-Identifier: MIT
//
// Draws the crab and its menu with skia-rs.
//
// Everything is positioned in "strip space": a virtual band the width of the
// screen, `strip_height + menu_headroom` tall, with the walking floor at the
// bottom. The surface being drawn into is a `Viewport` onto that band -- the
// whole thing for the Wayland layer-shell backend, or a small rectangle that
// follows the crab for the winit backend, which cannot do per-pixel
// click-through and so keeps its window no bigger than the character.
//
// Only the rectangles that actually changed are redrawn and reported as
// damage: the strip can span a 4K screen, and repainting all of it at 60Hz in
// a software rasteriser would burn CPU to move a 64px character a few pixels.

use skia_rs::canvas::Surface;
use skia_rs::codec::Image;
use skia_rs::core::{AlphaType, Color, ColorType, IRect as SkIRect, ImageInfo, Rect as SkRect};
use skia_rs::paint::{Paint, Style};
use skia_rs::text::Font;

use crate::brain::Brain;
use crate::config::{SPRITE_VARIANTS, label_for};
use crate::geom::IRect;
use crate::menu::Menu;
use crate::sprites;

// Palette, carried over from the QML menu so the look does not shift.
const PANEL_BG: (u8, u8, u8) = (0x1f, 0x1e, 0x1c);
const PANEL_BORDER: (u8, u8, u8) = (0x4a, 0x46, 0x42);
const HEADER_FG: (u8, u8, u8) = (0x8a, 0x85, 0x80);
const ROW_HOVER: (u8, u8, u8) = (0x33, 0x30, 0x2c);
const ROW_FG: (u8, u8, u8) = (0xe8, 0xe6, 0xe0);
const ROW_FG_CURRENT: (u8, u8, u8) = (0xd0, 0x6a, 0x4b);

/// Which part of the virtual strip the surface is showing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Device-pixel offset of the surface within strip space.
    pub origin: (i32, i32),
    /// Full strip width, for clamping the menu to the screen edges.
    pub strip_width: f32,
    /// Where the walking band begins within strip space.
    pub strip_top: f32,
}

pub struct Renderer {
    surface: Surface,
    width: i32,
    height: i32,
    sheet: Image,
    variant: String,
    row_font: Option<Font>,
    header_font: Option<Font>,
    /// Output scale; every coordinate here is in device pixels.
    scale: f32,
    /// Where the crab and menu were last drawn, in surface coordinates.
    prev_crab: IRect,
    prev_menu: IRect,
    /// The viewport origin the previous frame was drawn against.
    prev_origin: (i32, i32),
}

impl Renderer {
    pub fn new(width: i32, height: i32, variant: &str, scale: f32) -> Result<Self, String> {
        let sheet = sprites::load_sheet(variant)?;
        let surface = make_surface(width, height)?;
        // A missing font costs the menu its labels, not the crab its legs, so
        // this is a warning rather than a hard failure.
        let row_font = crate::font::load_menu_font(13.0 * scale);
        let header_font = crate::font::load_menu_font(11.0 * scale);
        if row_font.is_none() {
            log::error!(
                "no usable system font found; the menu will draw without labels. \
                 Set CLAUDE_CRAB_FONT to a font file to fix this."
            );
        }
        Ok(Self {
            surface,
            width,
            height,
            sheet,
            variant: variant.to_string(),
            row_font,
            header_font,
            scale,
            prev_crab: IRect::EMPTY,
            prev_menu: IRect::EMPTY,
            prev_origin: (0, 0),
        })
    }

    /// Reload the fonts at a new output scale.
    pub fn set_scale(&mut self, scale: f32) {
        if (scale - self.scale).abs() < f32::EPSILON {
            return;
        }
        self.scale = scale;
        self.row_font = crate::font::load_menu_font(13.0 * scale);
        self.header_font = crate::font::load_menu_font(11.0 * scale);
        self.invalidate();
    }

    pub fn pixels(&self) -> &[u8] {
        self.surface.pixels()
    }

    pub fn row_bytes(&self) -> usize {
        self.surface.row_bytes()
    }

    pub fn resize(&mut self, width: i32, height: i32) -> Result<(), String> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        self.surface = make_surface(width, height)?;
        self.width = width;
        self.height = height;
        // The old positions mean nothing on a surface of a different size.
        self.invalidate();
        Ok(())
    }

    pub fn set_variant(&mut self, variant: &str) -> Result<(), String> {
        if variant == self.variant {
            return Ok(());
        }
        self.sheet = sprites::load_sheet(variant)?;
        self.variant = variant.to_string();
        self.invalidate();
        Ok(())
    }

    /// Force the next render to repaint everything it owns.
    pub fn invalidate(&mut self) {
        self.prev_crab = IRect::EMPTY;
        self.prev_menu = IRect::EMPTY;
    }

    /// Repaint, returning the rectangles that changed, in surface coordinates.
    pub fn render(&mut self, brain: &Brain, menu: &Menu, view: Viewport) -> Vec<IRect> {
        // A scrolled viewport means every previously-drawn pixel now belongs
        // somewhere else; start from a clean surface rather than trying to
        // translate the old damage rectangles.
        if view.origin != self.prev_origin {
            self.prev_origin = view.origin;
            self.clear_all();
            self.prev_crab = IRect::EMPTY;
            self.prev_menu = IRect::EMPTY;
        }

        let mut damage = Vec::new();

        let crab = self.draw_crab(brain, view);
        let crab_damage = crab.union(&self.prev_crab).clamp_to(self.width, self.height);
        if !crab_damage.is_empty() {
            damage.push(crab_damage);
        }
        self.prev_crab = crab;

        let menu_damage = self.draw_menu(menu, view);
        if !menu_damage.is_empty() {
            damage.push(menu_damage);
        }

        damage
    }

    fn draw_crab(&mut self, brain: &Brain, view: Viewport) -> IRect {
        let r = brain.crab_rect();
        let dst = IRect::from_f32(
            r.x - view.origin.0 as f32,
            view.strip_top + r.y - view.origin.1 as f32,
            r.width,
            r.height,
        );
        if dst.clamp_to(self.width, self.height).is_empty() {
            self.clear_rect(self.prev_crab);
            return IRect::EMPTY;
        }

        // Erase both the old and the new footprint before drawing: the sprite
        // has an alpha edge, so painting over it without clearing would smear.
        let prev = self.prev_crab;
        self.clear_rect(prev);
        self.clear_rect(dst);

        let (sx, sy, sw, sh) = brain.source_frame();
        let src = SkIRect::new(sx, sy, sx + sw, sy + sh);
        let (dx, dy) = (dst.x as f32, dst.y as f32);
        let (dw, dh) = (dst.width as f32, dst.height as f32);

        let mut canvas = self.surface.canvas();
        if brain.facing_right() {
            canvas.draw_image_rect(&self.sheet, Some(&src), &SkRect::from_xywh(dx, dy, dw, dh), None);
        } else {
            // Mirror about the sprite's own centre line.
            canvas.save();
            canvas.translate(dx + dw, dy);
            canvas.scale(-1.0, 1.0);
            canvas.draw_image_rect(
                &self.sheet,
                Some(&src),
                &SkRect::from_xywh(0.0, 0.0, dw, dh),
                None,
            );
            canvas.restore();
        }
        dst
    }

    fn draw_menu(&mut self, menu: &Menu, view: Viewport) -> IRect {
        if !menu.is_open() {
            if self.prev_menu.is_empty() {
                return IRect::EMPTY;
            }
            // Just closed: erase it and report the hole.
            let gone = self.prev_menu;
            self.clear_rect(gone);
            self.prev_menu = IRect::EMPTY;
            return gone.clamp_to(self.width, self.height);
        }

        let offset =
            |r: IRect| IRect::new(r.x - view.origin.0, r.y - view.origin.1, r.width, r.height);
        let panel = offset(menu.panel_rect(view.strip_width));
        if panel.clamp_to(self.width, self.height).is_empty() {
            return IRect::EMPTY;
        }

        let stale = self.prev_menu;
        self.clear_rect(stale);
        self.clear_rect(panel);

        let hovered = menu.hovered;
        let current = self.variant.clone();
        let rows: Vec<IRect> =
            (0..Menu::item_count()).map(|i| offset(menu.row_rect(view.strip_width, i))).collect();

        {
            let mut canvas = self.surface.canvas();

            let body = SkRect::from_xywh(
                panel.x as f32,
                panel.y as f32,
                panel.width as f32,
                panel.height as f32,
            );
            let mut fill = Paint::new();
            fill.set_anti_alias(true);
            fill.set_style(Style::Fill);
            fill.set_argb(255, PANEL_BG.0, PANEL_BG.1, PANEL_BG.2);
            canvas.draw_round_rect(&body, 6.0, 6.0, &fill);

            let mut border = Paint::new();
            border.set_anti_alias(true);
            border.set_style(Style::Stroke);
            border.set_stroke_width(1.0);
            border.set_argb(255, PANEL_BORDER.0, PANEL_BORDER.1, PANEL_BORDER.2);
            // Inset by half the stroke width: a centred stroke would spill
            // outside the panel rect, and those pixels are outside the damage
            // and erase rectangles, so they would survive the menu closing.
            let outline = SkRect::from_xywh(
                panel.x as f32 + 0.5,
                panel.y as f32 + 0.5,
                panel.width as f32 - 1.0,
                panel.height as f32 - 1.0,
            );
            canvas.draw_round_rect(&outline, 6.0, 6.0, &border);

            if let Some(i) = hovered
                && let Some(row) = rows.get(i)
            {
                let mut hl = Paint::new();
                hl.set_style(Style::Fill);
                hl.set_argb(255, ROW_HOVER.0, ROW_HOVER.1, ROW_HOVER.2);
                canvas.draw_rect(
                    &SkRect::from_xywh(
                        row.x as f32,
                        row.y as f32,
                        row.width as f32,
                        row.height as f32,
                    ),
                    &hl,
                );
            }
        }

        // Text last, so nothing paints over it.
        if let Some(font) = self.header_font.clone() {
            let mut paint = Paint::new();
            paint.set_anti_alias(true);
            paint.set_argb(255, HEADER_FG.0, HEADER_FG.1, HEADER_FG.2);
            let mut canvas = self.surface.canvas();
            canvas.draw_string(
                "Sprite",
                panel.x as f32 + 10.0 * self.scale,
                panel.y as f32 + 15.0 * self.scale,
                &font,
                &paint,
            );
        }
        if let Some(font) = self.row_font.clone() {
            for (i, variant) in SPRITE_VARIANTS.iter().enumerate() {
                let Some(row) = rows.get(i) else { continue };
                let is_current = *variant == current;
                let mut paint = Paint::new();
                paint.set_anti_alias(true);
                if is_current {
                    paint.set_argb(255, ROW_FG_CURRENT.0, ROW_FG_CURRENT.1, ROW_FG_CURRENT.2);
                } else {
                    paint.set_argb(255, ROW_FG.0, ROW_FG.1, ROW_FG.2);
                }
                let label = format!(
                    "{}{}",
                    if is_current { "\u{2713}  " } else { "   " },
                    label_for(variant)
                );
                let baseline = row.y as f32 + row.height as f32 / 2.0 + 4.5 * self.scale;
                let mut canvas = self.surface.canvas();
                canvas.draw_string(
                    &label,
                    row.x as f32 + 10.0 * self.scale,
                    baseline,
                    &font,
                    &paint,
                );
            }
        }

        self.prev_menu = panel;
        panel.union(&stale).clamp_to(self.width, self.height)
    }

    fn clear_all(&mut self) {
        self.surface.pixels_mut().fill(0);
    }

    /// Zero a rectangle. The surface is premultiplied BGRA, so all-zero bytes
    /// are fully transparent -- cheaper and more predictable than a Clear-mode
    /// draw.
    fn clear_rect(&mut self, rect: IRect) {
        let rect = rect.clamp_to(self.width, self.height);
        if rect.is_empty() {
            return;
        }
        let row_bytes = self.surface.row_bytes();
        let start_x = rect.x as usize * 4;
        let span = rect.width as usize * 4;
        let pixels = self.surface.pixels_mut();
        for row in rect.y..rect.bottom() {
            let offset = row as usize * row_bytes + start_x;
            if offset + span <= pixels.len() {
                pixels[offset..offset + span].fill(0);
            }
        }
    }
}

fn make_surface(width: i32, height: i32) -> Result<Surface, String> {
    // BGRA explicitly rather than N32: wl_shm's ARGB8888 and softbuffer's
    // 0RGB are both little-endian, so the bytes must land in BGRA order.
    // ColorType::n32() is RGBA on Linux and would swap red and blue.
    let info = ImageInfo::new(width.max(1), height.max(1), ColorType::Bgra8888, AlphaType::Premul)
        .map_err(|e| format!("bad surface info {width}x{height}: {e:?}"))?;
    let mut surface = Surface::new_raster(&info, None)
        .ok_or_else(|| format!("cannot allocate {width}x{height}"))?;
    surface.canvas().clear(Color::TRANSPARENT);
    Ok(surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Reactions;
    use crate::sprites::Manifest;
    use crate::tracker::State;

    const FULL: Viewport = Viewport { origin: (0, 0), strip_width: 400.0, strip_top: 220.0 };

    fn brain() -> Brain {
        let mut b =
            Brain::new(Manifest::embedded().unwrap(), 1.0, "right".to_string(), Reactions::default());
        b.width = 400.0;
        b.height = 72.0;
        b
    }

    fn renderer() -> Renderer {
        Renderer::new(400, 292, "default", 1.0).unwrap()
    }

    fn opaque_pixels(r: &Renderer) -> usize {
        r.pixels().chunks(4).filter(|px| px[3] != 0).count()
    }

    #[test]
    fn a_fresh_surface_is_transparent() {
        assert_eq!(opaque_pixels(&renderer()), 0);
    }

    #[test]
    fn rendering_the_crab_puts_pixels_on_the_surface() {
        let mut r = renderer();
        let mut b = brain();
        b.x = 100.0;
        b.tick(0.0);
        assert_eq!(r.render(&b, &Menu::new(), FULL).len(), 1);
        assert!(opaque_pixels(&r) > 100, "the crab should have drawn");
    }

    #[test]
    fn damage_covers_both_the_old_and_new_position() {
        let mut r = renderer();
        let mut b = brain();
        b.x = 0.0;
        b.tick(0.0);
        r.render(&b, &Menu::new(), FULL);

        b.x = 120.0;
        let d = r.render(&b, &Menu::new(), FULL)[0];
        assert!(d.x <= 0, "damage should start at the old position");
        assert!(d.right() >= 184, "damage should reach the new position, got {d:?}");
    }

    #[test]
    fn moving_leaves_no_smear_behind() {
        let mut r = renderer();
        let mut b = brain();
        b.x = 0.0;
        b.tick(0.0);
        r.render(&b, &Menu::new(), FULL);
        let first = opaque_pixels(&r);

        b.x = 200.0;
        r.render(&b, &Menu::new(), FULL);
        let second = opaque_pixels(&r);
        assert!(second <= first + 2, "old position not erased: {first} -> {second}");
    }

    #[test]
    fn flipping_direction_still_draws() {
        let mut r = renderer();
        let mut b = brain();
        b.x = 100.0;
        b.direction = -1;
        b.tick(0.0);
        r.render(&b, &Menu::new(), FULL);
        assert!(opaque_pixels(&r) > 100, "a left-facing crab should still draw");
    }

    #[test]
    fn the_menu_draws_and_then_erases() {
        let mut r = renderer();
        let b = brain();
        let mut m = Menu::new();

        r.render(&b, &m, FULL);
        let without = opaque_pixels(&r);

        m.open_at(200.0, 220.0);
        assert_eq!(r.render(&b, &m, FULL).len(), 2, "crab plus menu");
        let with = opaque_pixels(&r);
        assert!(with > without + 1000, "the menu panel should be a solid block");

        m.close();
        r.render(&b, &m, FULL);
        assert!(opaque_pixels(&r) <= without + 2, "the menu should have been erased");
    }

    #[test]
    fn the_menu_panel_is_opaque_where_it_claims_to_be() {
        let mut r = renderer();
        let b = brain();
        let mut m = Menu::new();
        m.open_at(200.0, 220.0);
        r.render(&b, &m, FULL);

        let panel = m.panel_rect(400.0);
        let (cx, cy) = (panel.x + panel.width / 2, panel.y + panel.height / 2);
        let offset = cy as usize * r.row_bytes() + cx as usize * 4;
        assert_eq!(r.pixels()[offset + 3], 255, "panel centre should be opaque");
    }

    #[test]
    fn switching_variant_reloads_the_sheet() {
        let mut r = renderer();
        assert_eq!(r.variant, "default");
        r.set_variant("party").unwrap();
        assert_eq!(r.variant, "party");
        assert!(r.set_variant("nonsense").is_ok());
    }

    #[test]
    fn resize_reallocates_and_forgets_old_positions() {
        let mut r = renderer();
        let mut b = brain();
        b.tick(0.0);
        r.render(&b, &Menu::new(), FULL);
        r.resize(800, 292).unwrap();
        assert_eq!((r.width, r.height), (800, 292));
        assert_eq!(opaque_pixels(&r), 0, "a resized surface starts clean");
    }

    #[test]
    fn a_crab_off_the_edge_does_not_panic() {
        let mut r = renderer();
        let mut b = brain();
        b.x = 5000.0;
        b.tick(0.0);
        for d in r.render(&b, &Menu::new(), FULL) {
            assert!(d.right() <= 400 && d.bottom() <= 292);
        }
    }

    #[test]
    fn damage_never_escapes_the_surface() {
        let mut r = renderer();
        let mut b = brain();
        let mut m = Menu::new();
        m.open_at(395.0, 220.0);
        b.x = 390.0;
        b.tick(0.0);
        for d in r.render(&b, &m, FULL) {
            assert!(d.x >= 0 && d.y >= 0, "{d:?}");
            assert!(d.right() <= 400 && d.bottom() <= 292, "{d:?}");
        }
    }

    #[test]
    fn state_changes_move_the_crab_and_repaint() {
        let mut r = renderer();
        let mut b = brain();
        b.session_state = State::Working;
        b.tool = "Bash".to_string();
        b.tick(0.0);
        r.render(&b, &Menu::new(), FULL);
        for _ in 0..30 {
            b.tick(1.0 / 60.0);
        }
        assert!(!r.render(&b, &Menu::new(), FULL).is_empty());
    }

    // --- viewport, for the follow-the-crab backend --------------------------

    #[test]
    fn a_viewport_draws_the_crab_at_the_surface_origin() {
        // A 64x64 surface showing exactly the crab, wherever it is in the strip.
        let mut r = Renderer::new(64, 64, "default", 1.0).unwrap();
        let mut b = brain();
        b.x = 300.0;
        b.tick(0.0);
        let view = Viewport { origin: (300, 228), strip_width: 400.0, strip_top: 220.0 };
        r.render(&b, &Menu::new(), view);
        assert!(opaque_pixels(&r) > 100, "the crab should fill the followed window");
    }

    #[test]
    fn scrolling_the_viewport_clears_the_previous_contents() {
        let mut r = Renderer::new(64, 64, "default", 1.0).unwrap();
        let mut b = brain();
        b.x = 300.0;
        b.tick(0.0);
        r.render(&b, &Menu::new(), Viewport { origin: (300, 228), ..FULL });
        let first = opaque_pixels(&r);
        assert!(first > 100);

        // Crab moves and the window follows: the surface must not keep a ghost.
        b.x = 310.0;
        r.render(&b, &Menu::new(), Viewport { origin: (310, 228), ..FULL });
        let second = opaque_pixels(&r);
        assert!(
            second.abs_diff(first) < first / 2,
            "a scrolled viewport should show one crab, not a smear: {first} -> {second}"
        );
    }

    #[test]
    fn a_viewport_offsets_the_menu_too() {
        let mut r = Renderer::new(256, 256, "default", 1.0).unwrap();
        let b = brain();
        let mut m = Menu::new();
        m.open_at(200.0, 220.0);

        let panel = m.panel_rect(400.0);
        let view = Viewport { origin: (panel.x, panel.y), strip_width: 400.0, strip_top: 220.0 };
        r.render(&b, &m, view);
        // With the origin at the panel's own corner the panel starts at (0,0),
        // but the corner itself is rounded, so sample just inside the radius.
        let inside = 20 * r.row_bytes() + 20 * 4;
        assert_eq!(r.pixels()[inside + 3], 255, "panel should be drawn at the surface origin");
        // ...and the far side of the panel must still land inside the surface.
        let far = 20 * r.row_bytes() + (panel.width as usize - 20) * 4;
        assert_eq!(r.pixels()[far + 3], 255, "panel should span its full width");
    }
}
