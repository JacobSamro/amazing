//! CSS 2.1 §9.5 exclusion geometry, shared by blocks and line boxes.
use crate::Rect;
use amazing_css::ComputedStyle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Side {
    Left,
    Right,
}

pub(crate) fn side(style: &ComputedStyle) -> Option<Side> {
    match style.get("float")? {
        value if value.eq_ignore_ascii_case("left") => Some(Side::Left),
        value if value.eq_ignore_ascii_case("right") => Some(Side::Right),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FloatContext {
    entries: Vec<(Side, Rect)>,
}

impl FloatContext {
    pub fn add(&mut self, side: Side, rect: Rect) {
        self.entries.push((side, rect));
    }
    pub fn bottom(&self) -> f32 {
        self.entries
            .iter()
            .map(|(_, r)| r.y + r.h)
            .fold(f32::NEG_INFINITY, f32::max)
    }
    pub fn clear(&self, value: &str) -> f32 {
        self.entries
            .iter()
            .filter(|(side, _)| {
                value.eq_ignore_ascii_case("both")
                    || (*side == Side::Left && value.eq_ignore_ascii_case("left"))
                    || (*side == Side::Right && value.eq_ignore_ascii_case("right"))
            })
            .map(|(_, r)| r.y + r.h)
            .fold(f32::NEG_INFINITY, f32::max)
    }
    pub fn band(&self, x: f32, width: f32, y: f32, height: f32) -> (f32, f32) {
        let mut left = x;
        let mut right = x + width;
        for (side, r) in &self.entries {
            if r.y < y + height.max(0.001) && r.y + r.h > y {
                match side {
                    Side::Left => left = left.max(r.x + r.w),
                    Side::Right => right = right.min(r.x),
                }
            }
        }
        (left, right.max(left))
    }
    fn next_bottom(&self, y: f32) -> Option<f32> {
        self.entries
            .iter()
            .map(|(_, r)| r.y + r.h)
            .filter(|bottom| *bottom > y)
            .min_by(f32::total_cmp)
    }
    /// Find the highest usable band. Overwide content overflows only after
    /// all obstructing floats have ended; ties choose the requested edge.
    pub fn fit(&self, x: f32, width: f32, mut y: f32, height: f32, needed: f32) -> (f32, f32, f32) {
        loop {
            let (left, right) = self.band(x, width, y, height);
            if right - left + 0.01 >= needed || (left == x && right == x + width) {
                return (left, right, y);
            }
            match self.next_bottom(y) {
                Some(next) => y = next,
                None => return (left, right, y),
            }
        }
    }
    pub fn place(&mut self, side: Side, x: f32, width: f32, y: f32, w: f32, h: f32) -> Rect {
        let top = self.entries.last().map_or(y, |(_, r)| y.max(r.y));
        let (left, right, y) = self.fit(x, width, top, h, w);
        let rect = Rect {
            x: if side == Side::Left { left } else { right - w },
            y,
            w,
            h,
        };
        self.add(side, rect);
        rect
    }
}
