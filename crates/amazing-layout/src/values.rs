//! Resolution of computed-style strings into concrete pixel values
//! (css-values-3 subset + CSS2.1 §10 length rules).
//!
//! Phase 3 subset — implemented:
//! - absolute lengths: px, pt, pc, in, cm, mm (96dpi device model)
//! - font-relative lengths: em, rem, ch (the current 0-glyph approximation);
//!   percentages against a caller-supplied
//!   base (containing-block width for margins/padding/width, parent font for
//!   font-size)
//! - shorthand expansion (`margin`/`padding` 1–4 value forms) layered under
//!   the per-side longhands, since the Phase 2 cascade deliberately does not
//!   expand shorthands
//! - `auto` keyword for margins/width/height (returned as [`MaybeLen::Auto`])
//! - font-size keywords (medium = 16px ladder), unitless line-height numbers
//!
//! Deferred/deviations:
//! - calc(), ex units, vmin/vmax, `smaller`/`larger` are not resolved
//! - border-width keywords map thin/medium/thick → 1/3/5px regardless of the
//!   computed font-size scaling browsers apply to them

use amazing_css::ComputedStyle;

/// A length that may be `auto` (margins, width, height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MaybeLen {
    Auto,
    Px(f32),
}

/// Four physical sides in top/right/bottom/left order.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Sides {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

const PX_PER_IN: f32 = 96.0;

/// Parses one `<length>` / `<percentage>` string. `em_base` is the element's
/// computed font-size; `percent_base` depends on the property being parsed.
/// A bare `0` is valid without a unit (CSS2.1 §4.3.2); other bare numbers
/// are not lengths.
pub fn parse_len(s: &str, em_base: f32, percent_base: f32) -> Option<f32> {
    match MaybeLen::parse(s, em_base, percent_base)? {
        MaybeLen::Px(v) => Some(v),
        MaybeLen::Auto => None,
    }
}

impl MaybeLen {
    /// Parses one component value into [`MaybeLen`] (or None when it is not
    /// a resolvable length/auto at all).
    pub fn parse(s: &str, em_base: f32, percent_base: f32) -> Option<MaybeLen> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s.eq_ignore_ascii_case("auto") {
            return Some(MaybeLen::Auto);
        }
        // Split numeric prefix from unit suffix; a string that is entirely
        // numeric has an empty unit (only valid for zero).
        let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'));
        let (num, unit) = match split {
            Some(i) => s.split_at(i),
            None => (s, ""),
        };
        let n: f32 = num.parse().ok()?;
        Some(match unit.to_ascii_lowercase().as_str() {
            "px" => MaybeLen::Px(n),
            "pt" => MaybeLen::Px(n * PX_PER_IN / 72.0),
            "pc" => MaybeLen::Px(n * PX_PER_IN / 6.0),
            "in" => MaybeLen::Px(n * PX_PER_IN),
            "cm" => MaybeLen::Px(n * PX_PER_IN / 2.54),
            "mm" => MaybeLen::Px(n * PX_PER_IN / 25.4),
            "em" => MaybeLen::Px(n * em_base),
            "rem" => MaybeLen::Px(n * ROOT_FONT_SIZE),
            "ch" => MaybeLen::Px(n * em_base * 0.6),
            "%" => MaybeLen::Px(n * percent_base / 100.0),
            "" if n == 0.0 => MaybeLen::Px(0.0),
            _ => return None,
        })
    }

    pub fn unwrap_or(self, default: f32) -> f32 {
        match self {
            MaybeLen::Px(v) => v,
            MaybeLen::Auto => default,
        }
    }
}

/// Root font size backing `rem` and the `medium` default (16px device
/// convention).
pub const ROOT_FONT_SIZE: f32 = 16.0;

/// Resolves the computed font-size string to px. Relative units and the
/// keyword ladder scale off `parent_px`.
pub fn resolve_font_size(spec: &str, parent_px: f32) -> f32 {
    let spec = spec.trim();
    // Keyword ladder approximates Chrome defaults (css-fonts-3 absolute-size table).
    const LADDER: &[(&str, f32)] = &[
        ("xx-small", 9.0),
        ("x-small", 10.0),
        ("small", 13.0),
        ("medium", 16.0),
        ("large", 18.0),
        ("x-large", 24.0),
        ("xx-large", 32.0),
    ];
    if let Some((_, px)) = LADDER.iter().find(|(k, _)| spec.eq_ignore_ascii_case(k)) {
        return *px;
    }
    match parse_len(spec, parent_px, parent_px) {
        Some(px) if px > 0.0 => px,
        _ => parent_px, // unparseable → inherit-ish fallback
    }
}

/// Resolves line-height. `normal_px` is the caller's font-derived default
/// (ascent − descent + gap). Unitless numbers multiply the font size.
pub fn resolve_line_height(v: &str, font_px: f32, normal_px: f32) -> f32 {
    let v = v.trim();
    if v.eq_ignore_ascii_case("normal") {
        return normal_px;
    }
    // Unitless number: factor × font size.
    if let Ok(factor) = v.parse::<f32>() {
        return factor * font_px;
    }
    parse_len(v, font_px, font_px).unwrap_or(normal_px)
}

/// Margins keep `auto` distinct (§10.3.3 centers/expands with it).
pub fn resolve_margins(style: &ComputedStyle, em_base: f32, cbw: f32) -> SidesM {
    read_box_sides_auto(style, "margin", MARGIN_LONGHANDS, em_base, cbw)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SidesM {
    pub top: Option<f32>,
    pub right: Option<f32>,
    pub bottom: Option<f32>,
    pub left: Option<f32>,
}

impl SidesM {
    pub fn unwrap_or(&self, d: f32) -> Sides {
        Sides {
            top: self.top.unwrap_or(d),
            right: self.right.unwrap_or(d),
            bottom: self.bottom.unwrap_or(d),
            left: self.left.unwrap_or(d),
        }
    }
}

const MARGIN_LONGHANDS: [&str; 4] = ["margin-top", "margin-right", "margin-bottom", "margin-left"];
const PADDING_LONGHANDS: [&str; 4] = [
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
];

fn read_box_sides_auto(
    style: &ComputedStyle,
    shorthand: &str,
    longhands: [&str; 4],
    em_base: f32,
    percent_base: f32,
) -> SidesM {
    // Slots keep the full MaybeLen so `auto` survives longhand parsing;
    // shorthand components that fail to parse recover to 0.
    let mut slots: [Option<MaybeLen>; 4] = [None; 4];
    for (i, lh) in longhands.iter().enumerate() {
        if let Some(v) = style.get(lh) {
            slots[i] = MaybeLen::parse(v, em_base, percent_base);
        }
    }
    if let Some(sh) = style.get(shorthand) {
        let parts: Vec<&str> = sh.split_ascii_whitespace().collect();
        let slot = |s: &str| MaybeLen::parse(s, em_base, percent_base).unwrap_or(MaybeLen::Px(0.0));
        let expanded: [MaybeLen; 4] = match parts.len() {
            1 => [slot(parts[0]); 4],
            2 => [
                slot(parts[0]),
                slot(parts[1]),
                slot(parts[0]),
                slot(parts[1]),
            ],
            3 => [
                slot(parts[0]),
                slot(parts[1]),
                slot(parts[2]),
                slot(parts[1]),
            ],
            _ if parts.len() >= 4 => [
                slot(parts[0]),
                slot(parts[1]),
                slot(parts[2]),
                slot(parts[3]),
            ],
            _ => [MaybeLen::Px(0.0); 4],
        };
        for i in 0..4 {
            if slots[i].is_none() {
                slots[i] = Some(expanded[i]);
            }
        }
    }
    // Px → a concrete side value; Auto or absent → None (auto margin).
    let conv = |m: Option<MaybeLen>| match m {
        Some(MaybeLen::Px(v)) => Some(v),
        _ => None,
    };
    SidesM {
        top: conv(slots[0]),
        right: conv(slots[1]),
        bottom: conv(slots[2]),
        left: conv(slots[3]),
    }
}

/// Padding has no `auto`; unresolved sides are 0.
pub fn resolve_padding(style: &ComputedStyle, em_base: f32, cbw: f32) -> Sides {
    let m = read_box_sides_auto(style, "padding", PADDING_LONGHANDS, em_base, cbw);
    m.unwrap_or(0.0)
}

/// Border widths: thin/medium/thick → 1/3/5px; a side whose style is
/// none/hidden collapses to 0 (CSS2.1 §8.5.2). Sources layer longhand over
/// `border-width`/`border-style` over the full `border` shorthand.
pub fn resolve_borders(style: &ComputedStyle, em_base: f32) -> Sides {
    /// Picks side `i`'s component (t r b l replication) from the most
    /// specific source that declares one.
    fn side_component(
        style: &ComputedStyle,
        i: usize,
        want_width: bool,
        em_base: f32,
    ) -> Option<String> {
        const WIDTH_LONGHANDS: [&str; 4] = [
            "border-top-width",
            "border-right-width",
            "border-bottom-width",
            "border-left-width",
        ];
        const STYLE_LONGHANDS: [&str; 4] = [
            "border-top-style",
            "border-right-style",
            "border-bottom-style",
            "border-left-style",
        ];

        if let Some(v) = style.get(if want_width {
            WIDTH_LONGHANDS[i]
        } else {
            STYLE_LONGHANDS[i]
        }) {
            return Some(v.to_owned());
        }
        let shorthand = if want_width {
            "border-width"
        } else {
            "border-style"
        };
        if let Some(v) = style.get(shorthand) {
            let parts: Vec<&str> = v.split_ascii_whitespace().collect();
            return replicate_side(&parts, i).map(str::to_owned);
        }
        // `border: <width> || <style> || <color>` — components in any order;
        // classify each word by which grammar slot it can fill.
        if let Some(v) = style.get("border") {
            for p in v.split_ascii_whitespace() {
                let fits = if want_width {
                    is_width_component(p, em_base)
                } else {
                    is_style_component(p)
                };
                if fits {
                    return Some(p.to_owned());
                }
            }
        }
        None
    }

    fn is_style_component(s: &str) -> bool {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "none"
                | "hidden"
                | "dotted"
                | "dashed"
                | "solid"
                | "double"
                | "groove"
                | "ridge"
                | "inset"
                | "outset"
        )
    }

    fn is_width_component(s: &str, em_base: f32) -> bool {
        match s.to_ascii_lowercase().as_str() {
            "thin" | "medium" | "thick" => true,
            _ => MaybeLen::parse(s, em_base, 0.0).is_some(),
        }
    }

    let mut out = Sides::default();
    for (i, side) in [&mut out.top, &mut out.right, &mut out.bottom, &mut out.left]
        .into_iter()
        .enumerate()
    {
        let st = side_component(style, i, false, em_base).unwrap_or_else(|| "none".into());
        let visible = !st.eq_ignore_ascii_case("none") && !st.eq_ignore_ascii_case("hidden");
        *side = if !visible {
            0.0
        } else {
            match side_component(style, i, true, em_base).as_deref() {
                None => 3.0, // initial `medium`
                Some(w) => match w.to_ascii_lowercase().as_str() {
                    "thin" => 1.0,
                    "medium" => 3.0,
                    "thick" => 5.0,
                    _ => parse_len(w, em_base, 0.0).unwrap_or(3.0),
                },
            }
        };
    }
    out
}

/// Replicates a 1–4 component list onto side `i` (CSS2.1 §8.3: t r b l).
pub(crate) fn replicate_side<'a>(parts: &[&'a str], i: usize) -> Option<&'a str> {
    match parts.len() {
        1 => parts.first().copied(),
        2 => parts.get(i % 2).copied(),
        3 => match i {
            0 => parts.first().copied(),
            1 | 3 => parts.get(1).copied(),
            _ => parts.get(2).copied(),
        },
        _ => parts.get(i).copied(),
    }
}

/// width/height of the content box (`auto` preserved for §10.3.3).
pub fn resolve_extent(style: &ComputedStyle, prop: &str, em_base: f32, cbw: f32) -> MaybeLen {
    match style.get(prop) {
        Some(v) => MaybeLen::parse(v, em_base, cbw).unwrap_or(MaybeLen::Auto),
        None => MaybeLen::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_cover_absolute_font_relative_and_percent() {
        assert_eq!(parse_len("12px", 16.0, 100.0), Some(12.0));
        assert_eq!(parse_len("12pt", 16.0, 100.0), Some(16.0));
        assert_eq!(parse_len("1em", 20.0, 100.0), Some(20.0));
        assert_eq!(parse_len("50%", 16.0, 400.0), Some(200.0));
        assert_eq!(parse_len("0", 16.0, 100.0), Some(0.0));
        assert_eq!(parse_len("1rem", 16.0, 100.0), Some(16.0));
        assert_eq!(parse_len("16ch", 32.0, 100.0), Some(307.2));
        assert_eq!(parse_len("bogus", 16.0, 100.0), None);
        assert_eq!(parse_len("14", 16.0, 100.0), None); // bare nonzero number
    }

    #[test]
    fn padding_defaults_to_zero_without_declarations() {
        let p = resolve_padding(&ComputedStyle::default(), 16.0, 800.0);
        assert_eq!(p, Sides::default());
    }

    #[test]
    fn border_style_none_zeroes_width() {
        // Width without a visible style collapses.
        let b = resolve_borders(&style_of(&[("border-top-width", "10px")]), 16.0);
        assert_eq!(b.top, 0.0);
        let cs = style_of(&[("border-top-width", "10px"), ("border-top-style", "solid")]);
        assert_eq!(resolve_borders(&cs, 16.0).top, 10.0);
        // Side-keyword shorthands…
        let cs = style_of(&[("border-width", "2px"), ("border-style", "dashed")]);
        assert_eq!(resolve_borders(&cs, 16.0).top, 2.0);
        // …and the full `border` shorthand (width || style || color).
        let cs = style_of(&[("border", "4px solid red")]);
        assert_eq!(resolve_borders(&cs, 16.0).top, 4.0);
        assert_eq!(resolve_borders(&cs, 16.0).left, 4.0);
        // `border: solid red` keeps initial medium width.
        let cs = style_of(&[("border", "solid red")]);
        assert_eq!(resolve_borders(&cs, 16.0).top, 3.0);
    }

    #[test]
    fn font_size_ladder_and_relative_units() {
        assert_eq!(resolve_font_size("medium", 99.0), 16.0);
        assert_eq!(resolve_font_size("XX-LARGE", 99.0), 32.0);
        assert_eq!(resolve_font_size("1.5em", 20.0), 30.0);
        assert_eq!(resolve_font_size("150%", 20.0), 30.0);
        assert_eq!(resolve_font_size("nonsense", 20.0), 20.0);
    }

    #[test]
    fn line_height_forms() {
        assert_eq!(resolve_line_height("normal", 16.0, 19.2), 19.2);
        assert_eq!(resolve_line_height("1.5", 16.0, 19.2), 24.0);
        assert_eq!(resolve_line_height("24px", 16.0, 19.2), 24.0);
        assert_eq!(resolve_line_height("120%", 16.0, 19.2), 19.2);
    }

    /// Builds a style from (property, value) pairs via the public
    /// FromIterator impl.
    fn style_of(pairs: &[(&str, &str)]) -> ComputedStyle {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn margin_shorthand_replicates_and_auto_survives() {
        let cs = style_of(&[("margin", "1px 2px")]);
        let m = resolve_margins(&cs, 16.0, 800.0);
        assert_eq!(m.top, Some(1.0));
        assert_eq!(m.right, Some(2.0));
        assert_eq!(m.bottom, Some(1.0));
        assert_eq!(m.left, Some(2.0));

        let m = resolve_margins(&style_of(&[("margin", "0 auto")]), 16.0, 800.0);
        assert_eq!(m.top, Some(0.0));
        assert_eq!(m.left, None); // auto

        // Longhand beats replicated shorthand slot.
        let cs = style_of(&[("margin", "0 auto"), ("margin-left", "40px")]);
        let m = resolve_margins(&cs, 16.0, 800.0);
        assert_eq!(m.left, Some(40.0));
    }
}
