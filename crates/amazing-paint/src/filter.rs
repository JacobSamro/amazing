//! Bounded painter reference for CSS `filter` color functions.
//!
//! `filter` on an element applies to the element's whole rendered subtree, so
//! the painter applies the plan to its effect-group layer before compositing.
//! Blur, drop-shadow, `url()` and `calc()` filter functions are accepted by CSS
//! but are not part of this color-only slice: they are skipped rather than
//! guessed (the previous painter deferred the whole filter property anyway).
//! The plan is `Copy` with a fixed cap so it can live inside the copyable paint
//! command and effect-node records.

use amazing_css::effects::{FilterFunction, FilterList};

pub const MAX_FILTERS: usize = 8;

/// One supported color filter function with its resolved amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterOp {
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
}

/// A bounded, `Copy` filter list. At most `MAX_FILTERS` color functions are
/// retained; unsupported functions are dropped, never approximated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterPlan {
    ops: [Option<FilterOp>; MAX_FILTERS],
    len: u8,
}

impl Default for FilterPlan {
    fn default() -> Self {
        Self {
            ops: [None; MAX_FILTERS],
            len: 0,
        }
    }
}

impl FilterPlan {
    /// Parse a computed `filter` value. Only color functions are kept.
    pub fn from_css(value: &str) -> Self {
        let mut plan = Self::default();
        let Some(list) = amazing_css::effects::parse_filter_list(value) else {
            return plan;
        };
        if let FilterList::Functions(functions) = list {
            for function in functions {
                if let Some(op) = op_from(&function) {
                    if (plan.len as usize) < MAX_FILTERS {
                        plan.ops[plan.len as usize] = Some(op);
                        plan.len += 1;
                    }
                }
            }
        }
        plan
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn ops(&self) -> impl Iterator<Item = FilterOp> + '_ {
        self.ops.iter().take(self.len as usize).filter_map(|op| *op)
    }

    /// Apply every retained function in source order to one straight-alpha pixel.
    pub fn apply(&self, mut pixel: [u8; 4]) -> [u8; 4] {
        for op in self.ops() {
            pixel = apply_op(op, pixel);
        }
        pixel
    }
}

fn op_from(function: &FilterFunction) -> Option<FilterOp> {
    let amount = |value: f64| {
        let value = value as f32;
        value.is_finite().then_some(value)
    };
    Some(match function {
        FilterFunction::Brightness(value) => FilterOp::Brightness(amount(*value)?),
        FilterFunction::Contrast(value) => FilterOp::Contrast(amount(*value)?),
        FilterFunction::Grayscale(value) => FilterOp::Grayscale(amount(*value)?),
        FilterFunction::HueRotate(value) => FilterOp::HueRotate(amount(*value)?),
        FilterFunction::Invert(value) => FilterOp::Invert(amount(*value)?),
        FilterFunction::Opacity(value) => FilterOp::Opacity(amount(*value)?),
        FilterFunction::Saturate(value) => FilterOp::Saturate(amount(*value)?),
        FilterFunction::Sepia(value) => FilterOp::Sepia(amount(*value)?),
        FilterFunction::Blur(_)
        | FilterFunction::DropShadow(_)
        | FilterFunction::Url(_)
        | FilterFunction::Expression { .. } => return None,
    })
}

fn apply_op(op: FilterOp, pixel: [u8; 4]) -> [u8; 4] {
    let [r, g, b, a] = pixel.map(|channel| channel as f32);
    let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
    match op {
        FilterOp::Brightness(amount) => [
            channel(r * amount),
            channel(g * amount),
            channel(b * amount),
            a as u8,
        ],
        FilterOp::Contrast(amount) => [
            channel((r - 127.5) * amount + 127.5),
            channel((g - 127.5) * amount + 127.5),
            channel((b - 127.5) * amount + 127.5),
            a as u8,
        ],
        FilterOp::Grayscale(amount) => {
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            [
                channel(r + (luma - r) * amount),
                channel(g + (luma - g) * amount),
                channel(b + (luma - b) * amount),
                a as u8,
            ]
        }
        FilterOp::Invert(amount) => [
            channel(r + (255.0 - 2.0 * r) * amount),
            channel(g + (255.0 - 2.0 * g) * amount),
            channel(b + (255.0 - 2.0 * b) * amount),
            a as u8,
        ],
        FilterOp::Opacity(amount) => [r as u8, g as u8, b as u8, channel(a * amount)],
        FilterOp::Saturate(amount) => {
            let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            [
                channel(luma + (r - luma) * amount),
                channel(luma + (g - luma) * amount),
                channel(luma + (b - luma) * amount),
                a as u8,
            ]
        }
        FilterOp::Sepia(amount) => {
            let sr = 0.393 * r + 0.769 * g + 0.189 * b;
            let sg = 0.349 * r + 0.686 * g + 0.168 * b;
            let sb = 0.272 * r + 0.534 * g + 0.131 * b;
            [
                channel(r + (sr - r) * amount),
                channel(g + (sg - g) * amount),
                channel(b + (sb - b) * amount),
                a as u8,
            ]
        }
        FilterOp::HueRotate(degrees) => {
            let radians = degrees.to_radians();
            let (sin, cos) = (radians.sin(), radians.cos());
            let matrix = [
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
            ];
            [
                channel(matrix[0] * r + matrix[1] * g + matrix[2] * b),
                channel(matrix[3] * r + matrix[4] * g + matrix[5] * b),
                channel(matrix[6] * r + matrix[7] * g + matrix[8] * b),
                a as u8,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FilterOp, FilterPlan};

    #[test]
    fn parses_only_supported_color_functions_in_order() {
        let plan = FilterPlan::from_css("grayscale(1) blur(2px) brightness(0.5)");
        let ops: Vec<_> = plan.ops().collect();
        assert_eq!(ops, [FilterOp::Grayscale(1.0), FilterOp::Brightness(0.5)]);
        assert!(FilterPlan::from_css("none").is_empty());
        assert!(FilterPlan::from_css("blur(2px) drop-shadow(1px 1px red)").is_empty());
    }

    #[test]
    fn color_functions_match_their_definitions() {
        let red = [255, 0, 0, 255];
        assert_eq!(FilterPlan::from_css("grayscale(1)").apply(red), {
            let luma = (0.2126_f32 * 255.0).round() as u8;
            [luma, luma, luma, 255]
        });
        assert_eq!(FilterPlan::from_css("brightness(0)").apply(red), [0, 0, 0, 255]);
        assert_eq!(
            FilterPlan::from_css("invert(1)").apply(red),
            [0, 255, 255, 255]
        );
        let half = FilterPlan::from_css("opacity(0.5)").apply(red);
        assert_eq!(half, [255, 0, 0, 128]);
    }
}
