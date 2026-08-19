// SPDX-License-Identifier: MIT
//
// Integer rectangles in window coordinates. Damage tracking and input regions
// both speak pixels, so everything is rounded once, here, rather than at each
// call site.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl IRect {
    pub const EMPTY: Self = Self { x: 0, y: 0, width: 0, height: 0 };

    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width: width.max(0), height: height.max(0) }
    }

    /// Round outward, so a sub-pixel move never leaves a stale edge behind.
    pub fn from_f32(x: f32, y: f32, width: f32, height: f32) -> Self {
        let left = x.floor() as i32;
        let top = y.floor() as i32;
        let right = (x + width).ceil() as i32;
        let bottom = (y + height).ceil() as i32;
        Self::new(left, top, right - left, bottom - top)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let (x, y) = (x.floor() as i32, y.floor() as i32);
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    /// Grow by `by` pixels on every side. An empty rect stays empty.
    pub fn inflate(&self, by: i32) -> Self {
        if self.is_empty() {
            return *self;
        }
        Self::new(self.x - by, self.y - by, self.width + by * 2, self.height + by * 2)
    }

    /// Clip to a window of `width` x `height` at the origin.
    pub fn clamp_to(&self, width: i32, height: i32) -> Self {
        let x = self.x.max(0);
        let y = self.y.max(0);
        let right = self.right().min(width);
        let bottom = self.bottom().min(height);
        Self::new(x, y, right - x, bottom - y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_f32_rounds_outward() {
        let r = IRect::from_f32(10.4, 20.6, 30.2, 40.1);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 20);
        // 10.4 + 30.2 = 40.6 -> 41, so width covers 10..41.
        assert_eq!(r.right(), 41);
        assert_eq!(r.bottom(), 61);
    }

    #[test]
    fn union_of_two_rects_covers_both() {
        let a = IRect::new(0, 0, 10, 10);
        let b = IRect::new(20, 5, 10, 10);
        let u = a.union(&b);
        assert_eq!(u, IRect::new(0, 0, 30, 15));
    }

    #[test]
    fn union_with_empty_is_identity() {
        let a = IRect::new(5, 5, 10, 10);
        assert_eq!(a.union(&IRect::EMPTY), a);
        assert_eq!(IRect::EMPTY.union(&a), a);
    }

    #[test]
    fn clamp_keeps_the_rect_inside_the_window() {
        let r = IRect::new(-5, -5, 20, 20).clamp_to(10, 10);
        assert_eq!(r, IRect::new(0, 0, 10, 10));

        let off = IRect::new(50, 50, 10, 10).clamp_to(10, 10);
        assert!(off.is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let r = IRect::new(0, 0, 10, 10);
        assert!(r.contains(0.0, 0.0));
        assert!(r.contains(9.9, 9.9));
        assert!(!r.contains(10.0, 5.0));
        assert!(!r.contains(-0.1, 5.0));
    }

    #[test]
    fn inflate_grows_on_every_side() {
        let r = IRect::new(10, 10, 5, 5).inflate(2);
        assert_eq!(r, IRect::new(8, 8, 9, 9));
        // An empty rect has no edges to grow.
        assert!(IRect::EMPTY.inflate(4).is_empty());
    }

    #[test]
    fn negative_extents_collapse_to_empty() {
        assert!(IRect::new(0, 0, -5, 10).is_empty());
    }
}
