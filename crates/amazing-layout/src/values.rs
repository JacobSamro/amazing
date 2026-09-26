//! Resolution of computed-style strings into concrete pixel values
//! (css-values-3 subset + CSS2.1 §10 length rules).
//!
//! Phase 3 subset — implemented:
//! - absolute lengths: px, pt, pc, in, cm, mm (96dpi device model)
//! - font-relative lengths: em, rem, ch;
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
//! - ex units, vmin/vmax, `smaller`/`larger` are not resolved
//! - border-width keywords map thin/medium/thick → 1/3/5px regardless of the
//!   computed font-size scaling browsers apply to them

use amazing_css::{ComputedStyle, values::ComputedValue};

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

/// A computed length before its percentage has a used-value basis. Keeping
/// the two terms separate lets callers handle an indefinite containing block
/// or the cyclic-percentage rule for intrinsic sizing.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LengthPercentage {
    pub px: f32,
    pub percentage: f32,
    pub has_percentage: bool,
}

impl LengthPercentage {
    pub fn resolve(self, basis: Option<f32>) -> Option<f32> {
        if self.has_percentage {
            Some(self.px + self.percentage * basis? / 100.0)
        } else {
            Some(self.px)
        }
    }
}

#[derive(Clone, Copy)]
enum CalcValue {
    Number(f32),
    Length(LengthPercentage),
}

struct CalcParser<'a> {
    input: &'a str,
    pos: usize,
    units: FontRelativeUnits,
    size_basis: Option<LengthPercentage>,
}

/// Used font-relative unit sizes for the element and the document root.
/// The text provider supplies the first available face's metrics; the legacy
/// constructor keeps callers without a provider on deterministic defaults.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FontRelativeUnits {
    pub em: f32,
    pub ch: f32,
    pub ic: f32,
    pub ex: f32,
    pub cap: f32,
    pub root_em: f32,
    pub root_ch: f32,
    pub root_ic: f32,
    pub root_ex: f32,
    pub root_cap: f32,
}

impl FontRelativeUnits {
    pub(crate) fn legacy(em: f32, ch: f32) -> Self {
        Self {
            em,
            ch,
            ic: em,
            ex: em * 0.5,
            cap: em * 0.8,
            root_em: ROOT_FONT_SIZE,
            root_ch: ROOT_FONT_SIZE * 0.5,
            root_ic: ROOT_FONT_SIZE,
            root_ex: ROOT_FONT_SIZE * 0.5,
            root_cap: ROOT_FONT_SIZE * 0.8,
        }
    }

    pub(crate) fn from_text_system(
        text: Option<&dyn amazing_text::TextSystem>,
        style: Option<&amazing_text::TextStyle>,
        root_style: Option<&amazing_text::TextStyle>,
        fallback_em: f32,
    ) -> Self {
        let mut units = Self::legacy(fallback_em, fallback_em * 0.6);
        if let (Some(text), Some(style)) = (text, style) {
            let metrics = text.font_relative_metrics(style);
            units.em = style.size;
            units.ch = metrics.ch_advance;
            units.ic = metrics.ic_advance;
            units.ex = metrics.x_height;
            units.cap = metrics.cap_height;
        }
        if let (Some(text), Some(style)) = (text, root_style) {
            let metrics = text.font_relative_metrics(style);
            units.root_em = style.size;
            units.root_ch = metrics.ch_advance;
            units.root_ic = metrics.ic_advance;
            units.root_ex = metrics.x_height;
            units.root_cap = metrics.cap_height;
        }
        units
    }

    fn divided_by(self, factor: f32) -> Self {
        Self {
            em: self.em / factor, ch: self.ch / factor,
            ic: self.ic / factor, ex: self.ex / factor, cap: self.cap / factor,
            root_em: self.root_em / factor, root_ch: self.root_ch / factor,
            root_ic: self.root_ic / factor, root_ex: self.root_ex / factor,
            root_cap: self.root_cap / factor,
        }
    }
}

impl<'a> CalcParser<'a> {
    fn skip_ws(&mut self) {
        while self.input.as_bytes().get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_ws();
        if self.input.as_bytes().get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn sum(&mut self) -> Option<CalcValue> {
        let mut lhs = self.product()?;
        loop {
            self.skip_ws();
            let op = match self.input.as_bytes().get(self.pos) {
                Some(b'+') => 1.0,
                Some(b'-') => -1.0,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.product()?;
            lhs = match (lhs, rhs) {
                (CalcValue::Number(a), CalcValue::Number(b)) => CalcValue::Number(a + op * b),
                (CalcValue::Length(a), CalcValue::Length(b)) => CalcValue::Length(LengthPercentage {
                    px: a.px + op * b.px,
                    percentage: a.percentage + op * b.percentage,
                    has_percentage: a.has_percentage || b.has_percentage,
                }),
                _ => return None,
            };
        }
        Some(lhs)
    }

    fn product(&mut self) -> Option<CalcValue> {
        let mut lhs = self.atom()?;
        loop {
            self.skip_ws();
            let divide = match self.input.as_bytes().get(self.pos) {
                Some(b'*') => false,
                Some(b'/') => true,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.atom()?;
            lhs = match (lhs, rhs, divide) {
                (CalcValue::Number(a), CalcValue::Number(b), false) => CalcValue::Number(a * b),
                (CalcValue::Number(a), CalcValue::Number(b), true) if b != 0.0 => CalcValue::Number(a / b),
                (CalcValue::Length(a), CalcValue::Number(b), false) => CalcValue::Length(scale(a, b)),
                (CalcValue::Number(a), CalcValue::Length(b), false) => CalcValue::Length(scale(b, a)),
                (CalcValue::Length(a), CalcValue::Number(b), true) if b != 0.0 => CalcValue::Length(scale(a, 1.0 / b)),
                _ => return None,
            };
        }
        Some(lhs)
    }

    fn atom(&mut self) -> Option<CalcValue> {
        self.skip_ws();
        if self.input[self.pos..].get(..4).is_some_and(|s| s.eq_ignore_ascii_case("size"))
            && !self.input.as_bytes().get(self.pos + 4).is_some_and(u8::is_ascii_alphabetic)
        {
            self.pos += 4;
            return Some(CalcValue::Length(self.size_basis?));
        }
        if self.eat(b'(') {
            let value = self.sum()?;
            return self.eat(b')').then_some(value);
        }
        if self.input[self.pos..].get(..5).is_some_and(|s| s.eq_ignore_ascii_case("calc(")) {
            self.pos += 5;
            let value = self.sum()?;
            return self.eat(b')').then_some(value);
        }
        if self.input[self.pos..].get(..4).is_some_and(|s| s.eq_ignore_ascii_case("max("))
            || self.input[self.pos..].get(..4).is_some_and(|s| s.eq_ignore_ascii_case("min("))
        {
            let maximum = self.input[self.pos..].get(..4).is_some_and(|s| s.eq_ignore_ascii_case("max("));
            self.pos += 4;
            let mut result = self.sum()?;
            while self.eat(b',') {
                let next = self.sum()?;
                result = compare_calc_values(result, next, maximum)?;
            }
            return self.eat(b')').then_some(result);
        }
        if self.input[self.pos..].get(..10).is_some_and(|s| s.eq_ignore_ascii_case("calc-size(")) {
            self.pos += 10;
            self.skip_ws();
            let start = self.pos;
            let mut depth = 0;
            while let Some(&byte) = self.input.as_bytes().get(self.pos) {
                match byte {
                    b'(' => depth += 1,
                    b')' if depth > 0 => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {},
                }
                self.pos += 1;
            }
            let basis = self.input[start..self.pos].trim();
            if !self.eat(b',') { return None; }
            let previous = self.size_basis;
            self.size_basis = if basis.eq_ignore_ascii_case("any") {
                None
            } else {
                parse_length_percentage_with_units(basis, self.units).or(previous)
            };
            let result = self.sum();
            self.size_basis = previous;
            return self.eat(b')').then_some(result?);
        }
        let start = self.pos;
        if matches!(self.input.as_bytes().get(self.pos), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        while self.input.as_bytes().get(self.pos).is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
            self.pos += 1;
        }
        let number: f32 = self.input[start..self.pos].parse().ok()?;
        let unit_start = self.pos;
        while self.input.as_bytes().get(self.pos).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'%') {
            self.pos += 1;
        }
        let unit = &self.input[unit_start..self.pos];
        let length = match unit.to_ascii_lowercase().as_str() {
            "px" => Some(number),
            "pt" => Some(number * PX_PER_IN / 72.0),
            "pc" => Some(number * PX_PER_IN / 6.0),
            "in" => Some(number * PX_PER_IN),
            "cm" => Some(number * PX_PER_IN / 2.54),
            "mm" => Some(number * PX_PER_IN / 25.4),
            "em" => Some(number * self.units.em),
            "rem" => Some(number * self.units.root_em),
            "ch" => Some(number * self.units.ch),
            "rch" => Some(number * self.units.root_ch),
            "ic" => Some(number * self.units.ic),
            "ric" => Some(number * self.units.root_ic),
            "ex" => Some(number * self.units.ex),
            "rex" => Some(number * self.units.root_ex),
            "cap" => Some(number * self.units.cap),
            "rcap" => Some(number * self.units.root_cap),
            _ => None,
        };
        if unit == "%" {
            Some(CalcValue::Length(LengthPercentage { px: 0.0, percentage: number, has_percentage: true }))
        } else if let Some(px) = length {
            Some(CalcValue::Length(LengthPercentage { px, percentage: 0.0, has_percentage: false }))
        } else if unit.is_empty() {
            Some(CalcValue::Number(number))
        } else {
            None
        }
    }
}

fn compare_calc_values(left: CalcValue, right: CalcValue, maximum: bool) -> Option<CalcValue> {
    let (a, b) = match (left, right) {
        (CalcValue::Number(a), CalcValue::Number(b)) => return Some(CalcValue::Number(if maximum { a.max(b) } else { a.min(b) })),
        (CalcValue::Length(a), CalcValue::Length(b)) => (a, b),
        _ => return None,
    };
    if a.has_percentage != b.has_percentage || a.percentage != b.percentage { return None; }
    Some(CalcValue::Length(if (a.px >= b.px) == maximum { a } else { b }))
}

fn scale(value: LengthPercentage, factor: f32) -> LengthPercentage {
    LengthPercentage { px: value.px * factor, percentage: value.percentage * factor, has_percentage: value.has_percentage }
}

/// Parse a `<length-percentage>` including nested `calc()` arithmetic. `ch`
/// comes from the selected font's zero-glyph advance at the computed size.
pub fn parse_length_percentage(s: &str, em: f32, ch: f32) -> Option<LengthPercentage> {
    parse_length_percentage_with_units(s, FontRelativeUnits::legacy(em, ch))
}

pub(crate) fn parse_length_percentage_with_units(
    s: &str, units: FontRelativeUnits,
) -> Option<LengthPercentage> {
    let mut parser = CalcParser { input: s.trim(), pos: 0, units, size_basis: None };
    let value = parser.sum()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() { return None; }
    let result = match value {
        CalcValue::Length(length) => length,
        CalcValue::Number(0.0) => LengthPercentage { px: 0.0, percentage: 0.0, has_percentage: false },
        _ => return None,
    };
    (result.px.is_finite() && result.percentage.is_finite()).then_some(result)
}

/// Read a validated computed math expression while retaining its percentage
/// term until the caller supplies the used-value basis. CSS maps logical size
/// names onto physical longhands before this point, so callers use `width`,
/// `height`, and their min/max counterparts for the actual writing mode.
pub fn style_length_percentage(
    style: &ComputedStyle, property: &str, em: f32, ch: f32,
) -> Option<LengthPercentage> {
    let source = match style.typed(property) {
        Some(ComputedValue::LengthPercentageExpression(expression)) => expression.as_str(),
        _ => style.get(property)?,
    };
    used_length_percentage(style, source, em, ch)
}

pub(crate) fn style_length_percentage_with_units(
    style: &ComputedStyle, property: &str, units: FontRelativeUnits,
) -> Option<LengthPercentage> {
    let source = match style.typed(property) {
        Some(ComputedValue::LengthPercentageExpression(expression)) => expression.as_str(),
        _ => style.get(property)?,
    };
    used_length_percentage_with_units(style, source, units)
}

/// Scale only the absolute term at CSS zoom's used-value boundary. The
/// percentage term uses its caller-supplied (already zoomed) containing box.
/// `em` and `ch` arrive at their used sizes, so undo zoom before parsing and
/// apply it once to the resulting absolute term.
pub fn used_length_percentage(
    style: &ComputedStyle, source: &str, em: f32, ch: f32,
) -> Option<LengthPercentage> {
    let zoom = style.effective_zoom() as f32;
    let mut length = parse_length_percentage(source, em / zoom, ch / zoom)?;
    length.px = style.scale_used_length(length.px as f64) as f32;
    Some(length)
}

pub(crate) fn used_length_percentage_with_units(
    style: &ComputedStyle, source: &str, units: FontRelativeUnits,
) -> Option<LengthPercentage> {
    let zoom = style.effective_zoom() as f32;
    let mut length = parse_length_percentage_with_units(source, units.divided_by(zoom))?;
    length.px = style.scale_used_length(length.px as f64) as f32;
    Some(length)
}

/// Resolve an intrinsic `calc-size()` after layout has determined the used
/// size of its keyword basis. The expression keeps percentage resolution
/// separate from the intrinsic basis, as required for indefinite sizes.
pub(crate) fn used_calc_size_with_basis(
    style: &ComputedStyle, source: &str, units: FontRelativeUnits,
    basis: f32, percent_basis: Option<f32>,
) -> Option<f32> {
    let zoom = style.effective_zoom() as f32;
    let mut parser = CalcParser {
        input: source.trim(), pos: 0, units: units.divided_by(zoom),
        size_basis: Some(LengthPercentage { px: basis / zoom, percentage: 0.0, has_percentage: false }),
    };
    let value = parser.sum()?;
    parser.skip_ws();
    if parser.pos != parser.input.len() { return None; }
    let CalcValue::Length(mut length) = value else { return None; };
    length.px = style.scale_used_length(length.px as f64) as f32;
    length.resolve(percent_basis).filter(|value| value.is_finite())
}

pub(crate) fn calc_size_basis(source: &str) -> Option<&str> {
    let source = source.trim();
    if !source.get(..10)?.eq_ignore_ascii_case("calc-size(") { return None; }
    let mut depth = 0;
    for (index, byte) in source[10..].bytes().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            b',' if depth == 0 => return Some(source[10..10 + index].trim()),
            _ => {},
        }
    }
    None
}

pub(crate) fn preferred_aspect_ratio(style: &ComputedStyle) -> Option<f32> {
    let source = style.get("aspect-ratio")?.trim();
    let ratio = source.strip_prefix("auto ").unwrap_or(source);
    let (numerator, denominator) = ratio.split_once('/')?;
    let numerator: f32 = numerator.trim().parse().ok()?;
    let denominator: f32 = denominator.trim().parse().ok()?;
    (numerator.is_finite() && denominator.is_finite() && numerator > 0.0 && denominator > 0.0)
        .then_some(numerator / denominator)
}

pub(crate) fn parse_used_maybe_len(
    style: &ComputedStyle, source: &str, em: f32, basis: f32,
) -> Option<MaybeLen> {
    if source.trim().eq_ignore_ascii_case("auto") {
        return Some(MaybeLen::Auto);
    }
    used_length_percentage(style, source, em, em * 0.6)?
        .resolve(Some(basis))
        .map(MaybeLen::Px)
}

pub fn resolve_used_font_size(style: &ComputedStyle, parent_used_px: f32) -> f32 {
    if let Some(px) = style.used_length_px("font-size") {
        return px as f32;
    }
    let zoom = style.effective_zoom() as f32;
    style.scale_used_length(resolve_font_size(
        style.get("font-size").unwrap_or("medium"),
        parent_used_px / zoom,
    ) as f64) as f32
}

pub fn resolve_used_line_height(style: &ComputedStyle, font_px: f32, normal_px: f32) -> f32 {
    let value = style.get("line-height").unwrap_or("normal");
    if value.eq_ignore_ascii_case("normal") {
        return normal_px;
    }
    if let Ok(factor) = value.parse::<f32>() {
        return factor * font_px;
    }
    style_length_percentage(style, "line-height", font_px, font_px * 0.6)
        .and_then(|length| length.resolve(Some(font_px)))
        .unwrap_or(normal_px)
}

fn style_maybe_length(
    style: &ComputedStyle, property: &str, units: FontRelativeUnits, basis: f32,
) -> Option<MaybeLen> {
    if style.get(property)?.trim().eq_ignore_ascii_case("auto") {
        return Some(MaybeLen::Auto);
    }
    style_length_percentage_with_units(style, property, units)?
        .resolve(Some(basis))
        .map(MaybeLen::Px)
}

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
        Some(MaybeLen::Px(parse_length_percentage(s, em_base, em_base * 0.6)?.resolve(Some(percent_base))?))
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
    resolve_margins_with_units(style, FontRelativeUnits::legacy(em_base, em_base * 0.6), cbw)
}

pub(crate) fn resolve_margins_with_units(
    style: &ComputedStyle, units: FontRelativeUnits, cbw: f32,
) -> SidesM {
    read_box_sides_auto(style, "margin", MARGIN_LONGHANDS, units, cbw)
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
    units: FontRelativeUnits,
    percent_base: f32,
) -> SidesM {
    // Slots keep the full MaybeLen so `auto` survives longhand parsing;
    // shorthand components that fail to parse recover to 0.
    let mut slots: [Option<MaybeLen>; 4] = [None; 4];
    for (i, lh) in longhands.iter().enumerate() {
        slots[i] = style_maybe_length(style, lh, units, percent_base);
    }
    if let Some(sh) = style.get(shorthand) {
        let parts: Vec<&str> = sh.split_ascii_whitespace().collect();
        let slot = |s: &str| {
            if s.eq_ignore_ascii_case("auto") {
                MaybeLen::Auto
            } else {
                used_length_percentage_with_units(style, s, units)
                    .and_then(|length| length.resolve(Some(percent_base)))
                    .map_or(MaybeLen::Px(0.0), MaybeLen::Px)
            }
        };
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
    resolve_padding_with_units(style, FontRelativeUnits::legacy(em_base, em_base * 0.6), cbw)
}

pub(crate) fn resolve_padding_with_units(
    style: &ComputedStyle, units: FontRelativeUnits, cbw: f32,
) -> Sides {
    let m = read_box_sides_auto(style, "padding", PADDING_LONGHANDS, units, cbw);
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
                None => style.scale_used_length(3.0) as f32, // initial `medium`
                Some(w) => match w.to_ascii_lowercase().as_str() {
                    "thin" => style.scale_used_length(1.0) as f32,
                    "medium" => style.scale_used_length(3.0) as f32,
                    "thick" => style.scale_used_length(5.0) as f32,
                    _ => used_length_percentage(style, w, em_base, em_base * 0.6)
                        .and_then(|length| length.resolve(Some(0.0)))
                        .unwrap_or(style.scale_used_length(3.0) as f32),
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
    style_length_percentage(style, prop, em_base, em_base * 0.6)
        .and_then(|value| value.resolve(Some(cbw)))
        .map_or(MaybeLen::Auto, MaybeLen::Px)
}

/// Resolve an extent with the selected font's actual zero-glyph advance.
pub fn resolve_extent_with_ch(
    style: &ComputedStyle, prop: &str, em_base: f32, ch_advance: f32, basis: Option<f32>,
) -> MaybeLen {
    style_length_percentage(style, prop, em_base, ch_advance)
        .and_then(|value| value.resolve(basis))
        .map_or(MaybeLen::Auto, MaybeLen::Px)
}

pub(crate) fn resolve_extent_with_units(
    style: &ComputedStyle, prop: &str, units: FontRelativeUnits, basis: Option<f32>,
) -> MaybeLen {
    style_length_percentage_with_units(style, prop, units)
        .and_then(|length| length.resolve(basis))
        .map(MaybeLen::Px)
        .unwrap_or(MaybeLen::Auto)
}

/// Resolve all four border corner radii in top-left, top-right,
/// bottom-right, bottom-left order. Horizontal percentages use the border-box
/// width and vertical percentages use its height. Paint applies the common
/// overlap normalization after consuming these used values.
pub fn resolve_corner_radii(
    style: &ComputedStyle, em: f32, ch: f32, width: f32, height: f32,
) -> [(f32, f32); 4] {
    let longhands = [
        "border-top-left-radius", "border-top-right-radius",
        "border-bottom-right-radius", "border-bottom-left-radius",
    ];
    if longhands.iter().all(|name| style.get(name).is_some()) {
        return longhands.map(|name| radius_pair(style, style.get(name).unwrap_or(""), em, ch, width, height));
    }
    let Some(shorthand) = style.get("border-radius") else { return [(0.0, 0.0); 4] };
    let slash = shorthand.char_indices().scan(0i32, |depth, (i, c)| {
        match c { '(' => *depth += 1, ')' => *depth -= 1, _ => {} }
        Some((i, c, *depth))
    }).find(|(_, c, depth)| *c == '/' && *depth == 0).map(|(i, _, _)| i);
    let (horizontal, vertical) = slash.map_or((shorthand, None), |i| (&shorthand[..i], Some(&shorthand[i + 1..])));
    let horizontal = expand_radii(style, horizontal, em, ch, width);
    let vertical = vertical.map(|value| expand_radii(style, value, em, ch, height)).unwrap_or_else(|| expand_radii(style, horizontal_source(shorthand, slash), em, ch, height));
    std::array::from_fn(|i| (horizontal[i], vertical[i]))
}

fn horizontal_source<'a>(shorthand: &'a str, slash: Option<usize>) -> &'a str {
    slash.map_or(shorthand, |i| &shorthand[..i])
}

fn radius_pair(style: &ComputedStyle, value: &str, em: f32, ch: f32, width: f32, height: f32) -> (f32, f32) {
    let values = radius_tokens(value);
    match values.as_slice() {
        [both] => (radius_value(style, both, em, ch, width), radius_value(style, both, em, ch, height)),
        [x, y] => (radius_value(style, x, em, ch, width), radius_value(style, y, em, ch, height)),
        _ => (0.0, 0.0),
    }
}

fn expand_radii(style: &ComputedStyle, value: &str, em: f32, ch: f32, basis: f32) -> [f32; 4] {
    let numbers = radius_tokens(value).into_iter().map(|token| radius_value(style, token, em, ch, basis)).collect::<Vec<_>>();
    match numbers.as_slice() {
        [one] => [*one; 4],
        [top_left, top_right] => [*top_left, *top_right, *top_left, *top_right],
        [top_left, other, bottom_right] => [*top_left, *other, *bottom_right, *other],
        [top_left, top_right, bottom_right, bottom_left] => [*top_left, *top_right, *bottom_right, *bottom_left],
        _ => [0.0; 4],
    }
}

fn radius_value(style: &ComputedStyle, value: &str, em: f32, ch: f32, basis: f32) -> f32 {
    used_length_percentage(style, value, em, ch)
        .and_then(|value| value.resolve(Some(basis)))
        .unwrap_or(0.0).max(0.0)
}

fn radius_tokens(value: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut depth = 0i32;
    let mut start = None;
    for (i, c) in value.char_indices() {
        if c.is_ascii_whitespace() && depth == 0 {
            if let Some(begin) = start.take() { tokens.push(&value[begin..i]); }
            continue;
        }
        start.get_or_insert(i);
        match c { '(' => depth += 1, ')' => depth -= 1, _ => {} }
    }
    if let Some(begin) = start { tokens.push(&value[begin..]); }
    tokens
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
    fn used_size_math_handles_resolved_attr_and_definite_calc_size() {
        assert_eq!(parse_len("max(200px)", 16.0, 300.0), Some(200.0));
        assert_eq!(parse_len("min(220px, 200px)", 16.0, 300.0), Some(200.0));
        assert_eq!(parse_len("calc-size(40px, size + 20px)", 16.0, 300.0), Some(60.0));
        assert_eq!(parse_len("calc-size(any, 10px + 50%)", 16.0, 300.0), Some(160.0));
        assert_eq!(parse_len("calc-size(any, size + 10px)", 16.0, 300.0), None);
    }

    #[test]
    fn computed_calc_and_logical_size_use_the_physical_axis() {
        let dom = amazing_html::parse("<body><div id='target'></div></body>");
        let sheet = amazing_css::parse_stylesheet(
            "#target { writing-mode: vertical-lr; inline-size: calc(50% - 2px); block-size: 40px }",
        );
        let styled = amazing_css::compute(&dom, &[sheet]);
        let target = (0..dom.node_count())
            .map(|index| amazing_dom::NodeIdx(index as u32))
            .find(|&node| dom.name(node) == Some("div"))
            .unwrap();
        let style = styled.get(target).unwrap();
        assert!(matches!(
            style.typed("height"),
            Some(ComputedValue::LengthPercentageExpression(_))
        ));
        assert_eq!(resolve_extent(style, "height", 16.0, 200.0), MaybeLen::Px(98.0));
        assert_eq!(resolve_extent(style, "width", 16.0, 200.0), MaybeLen::Px(40.0));
    }

    #[test]
    fn font_relative_units_keep_element_and_root_metrics_separate() {
        let units = FontRelativeUnits {
            em: 20.0, ch: 9.0, ic: 20.0, ex: 8.0, cap: 15.0,
            root_em: 16.0, root_ch: 7.0, root_ic: 16.0, root_ex: 6.0, root_cap: 12.0,
        };
        let length = parse_length_percentage_with_units(
            "calc(1ch + 1ic + 1ex + 1cap + 1rch + 1ric + 1rex + 1rcap)", units,
        ).unwrap();
        assert_eq!(length.resolve(None), Some(93.0));
    }

    #[test]
    fn calc_keeps_length_and_percentage_until_the_used_basis_is_known() {
        let value = parse_length_percentage("calc(150% / 2 - 30px)", 16.0, 8.0).unwrap();
        assert_eq!(value.resolve(Some(100.0)), Some(45.0));
        assert_eq!(value.resolve(None), None);
        assert_eq!(parse_length_percentage("calc(5em - 0%)", 10.0, 5.0).unwrap().resolve(Some(0.0)), Some(50.0));
        let rounded = parse_length_percentage("calc(50% - 2 * 4.009px)", 16.0, 8.0).unwrap().resolve(Some(100.0)).unwrap();
        assert!((rounded - 41.982).abs() < 0.0001);
        assert_eq!(parse_length_percentage("5ch", 80.0, 43.0).unwrap().resolve(None), Some(215.0));
        assert!(parse_length_percentage("calc(1px * 2px)", 16.0, 8.0).is_none());
        assert!(parse_length_percentage("calc(1px / 0)", 16.0, 8.0).is_none());
    }

    #[test]
    fn border_radius_calc_uses_its_own_axis_and_keeps_function_whitespace() {
        let style = style_of(&[("border-radius", "calc((1/32 * 100%) + 5px) calc((1/64 * 100%) - 2px) calc(10px + (1/256 * 100%)) calc((1/16 * 100%) - 3px) / calc((1/32 * 100%) - (1px + (1/128 * 100%))) calc(1/16 * 100%) calc(10px) 3px")]);
        assert_eq!(resolve_corner_radii(&style, 16.0, 8.0, 512.0, 256.0), [(21.0, 5.0), (6.0, 16.0), (12.0, 10.0), (29.0, 3.0)]);
    }

    #[test]
    fn zoom_scales_absolute_used_lengths_once_but_not_percentage_terms() {
        let mut style = ComputedStyle::default();
        style.set("zoom", "2");
        style.set("font-size", "16px");
        style.set("width", "calc(50% + 10px)");
        style.set("border-top-style", "solid");
        style.set("border-top-width", "2px");
        assert_eq!(resolve_used_font_size(&style, 16.0), 32.0);
        assert_eq!(resolve_extent(&style, "width", 32.0, 200.0), MaybeLen::Px(120.0));
        assert_eq!(resolve_borders(&style, 32.0).top, 4.0);

        style.set("width", "1em");
        assert_eq!(resolve_extent(&style, "width", 32.0, 200.0), MaybeLen::Px(32.0));
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
