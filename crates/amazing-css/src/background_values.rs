//! CSS-30: the comma-separated multi-layer background sub-property grammars
//! -- `background-clip`, `-origin`, `-attachment`, `-repeat`, `-size` and
//! `-position`/`-position-x`/`-position-y` -- moved into their own module for
//! the same reason as `grid_values.rs`/`shadow_values.rs` (CSS-29): kept
//! `computed.rs` under the 4,000-line policy while giving this cluster of
//! properties, previously either an overly-narrow single-keyword validator
//! (`background-clip`) or unvalidated literal passthrough (everything else),
//! a real grammar and, where the specified form differs from the computed
//! form (keyword resolution, box+text reordering, same-value collapsing),
//! a real computed-value serializer.
//!
//! Every `valid_background_*`/`background_*_computed` function here is
//! called from `values.rs` (validity) and from `computed.rs`'s
//! `expand_declaration`/`valid_new_paint_property` (computed serialization
//! and the var()-substitution validity gate) as `crate::computed::name` or
//! `super::name`.
//!
//! `background-position`'s edge+offset resolution (CSS Backgrounds 3 SS3.9)
//! is the deepest grammar here: 1-token (keyword or bare length-percentage),
//! 2-token (a strict X/Y pair, unless a keyword's own axis identity forces a
//! swap), and 3-4-token (`&&`-combined edge keywords each with an optional
//! offset, chunked and then assigned to X/Y by keyword identity) forms, all
//! derived directly from the pinned WPT `background-position*-computed.html`
//! cases. `em`/`rem`-based offsets and calc() expressions that mix a
//! percentage-free length with a relative unit need the div's cascaded
//! `font-size` to resolve to a px number (e.g. `0.5em` -> `20px` at
//! `font-size: 40px`); `wpt_conformance.rs`'s harness casts a lone
//! `div { property: specified }` with no other declaration, so those cases
//! keep their literal specified text (self-consistent with every other
//! property this crate cannot resolve without extra cascade context) rather
//! than guessing -- a documented measurement-harness boundary, not a bug in
//! this grammar.

use super::{top_level_components, trim_tokens};
use crate::syntax::{Token, serialize, tokenize};
use crate::values::LengthPercentage;

/// Splits on top-level (paren/bracket/brace-depth-zero) commas -- the same
/// shape as `shadow_values::top_level_comma_groups`, duplicated rather than
/// shared so each module stays self-contained.
fn top_level_comma_groups(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut groups = Vec::new();
    let mut stack = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Comma if stack.is_empty() => {
                groups.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    groups.push(&tokens[start..]);
    Some(groups)
}

/// Runs `layer` over every top-level comma group of `value`'s own top-level
/// (whitespace-separated) components, joining successful results with
/// `", "`. `None` from either splitter or `layer` invalidates the whole
/// value -- CSS comma lists do not allow an empty item.
fn list_layers(value: &str, layer: impl Fn(&[&[Token]]) -> Option<String>) -> Option<String> {
    let tokens = tokenize(value);
    let groups = top_level_comma_groups(&tokens)?;
    let resolved: Option<Vec<String>> = groups
        .iter()
        .map(|group| {
            let components = top_level_components(group)?;
            layer(&components)
        })
        .collect();
    resolved.map(|layers| layers.join(", "))
}

fn ident_of(component: &[Token]) -> Option<String> {
    match trim_tokens(component) {
        [Token::Ident(name)] => Some(name.clone()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// background-clip: `<bg-clip-layer># = [ <visual-box> | border-area | text |
// ( <visual-box> || text ) ]#` -- CSS-29 mistakenly used a single-keyword-only
// validator (`valid_background_clip`) here that never accepted a comma list
// or the box+text combination, which `expand_declaration`'s guard then read
// as "invalid", silently dropping the declaration back to the property's
// initial value regardless of what was specified.

fn background_clip_layer(components: &[&[Token]]) -> Option<String> {
    if components.is_empty() || components.len() > 2 {
        return None;
    }
    let mut visual_box: Option<&'static str> = None;
    let mut text = false;
    for component in components {
        let ident = ident_of(component)?;
        match ident.to_ascii_lowercase().as_str() {
            "border-box" if visual_box.is_none() => visual_box = Some("border-box"),
            "padding-box" if visual_box.is_none() => visual_box = Some("padding-box"),
            "content-box" if visual_box.is_none() => visual_box = Some("content-box"),
            "border-area" if visual_box.is_none() => visual_box = Some("border-area"),
            "text" if !text => text = true,
            _ => return None,
        }
    }
    Some(match (visual_box, text) {
        (Some(b), true) => format!("{b} text"),
        (Some(b), false) => b.to_string(),
        (None, true) => "text".to_string(),
        (None, false) => unreachable!("loop requires each component to set box or text"),
    })
}

pub(crate) fn valid_background_clip(value: &str) -> bool {
    background_clip_computed(value).is_some()
}

pub(crate) fn background_clip_computed(value: &str) -> Option<String> {
    list_layers(value, background_clip_layer)
}

// ---------------------------------------------------------------------------
// background-origin: `<visual-box>#`, no reordering, no combination.

fn background_origin_layer(components: &[&[Token]]) -> Option<String> {
    match components {
        [one] => match ident_of(one)?.to_ascii_lowercase().as_str() {
            "border-box" => Some("border-box".to_string()),
            "padding-box" => Some("padding-box".to_string()),
            "content-box" => Some("content-box".to_string()),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn valid_background_origin(value: &str) -> bool {
    background_origin_computed(value).is_some()
}

pub(crate) fn background_origin_computed(value: &str) -> Option<String> {
    list_layers(value, background_origin_layer)
}

// ---------------------------------------------------------------------------
// background-attachment: `<attachment>#` = scroll | fixed | local.

fn background_attachment_layer(components: &[&[Token]]) -> Option<String> {
    match components {
        [one] => match ident_of(one)?.to_ascii_lowercase().as_str() {
            "scroll" => Some("scroll".to_string()),
            "fixed" => Some("fixed".to_string()),
            "local" => Some("local".to_string()),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn valid_background_attachment(value: &str) -> bool {
    background_attachment_computed(value).is_some()
}

pub(crate) fn background_attachment_computed(value: &str) -> Option<String> {
    list_layers(value, background_attachment_layer)
}

// ---------------------------------------------------------------------------
// background-repeat: `<repeat-style>#` = repeat-x | repeat-y |
// [repeat|space|round|no-repeat]{1,2}. A single component's second axis
// defaults to the same keyword, and two identical keywords collapse back to
// one at the computed-value stage (confirmed against WPT: `"repeat repeat"`
// computes to `"repeat"`, and `"repeat"` alone round-trips to `"repeat"`).

fn repeat_word(component: &[Token]) -> Option<&'static str> {
    match ident_of(component)?.to_ascii_lowercase().as_str() {
        "repeat" => Some("repeat"),
        "space" => Some("space"),
        "round" => Some("round"),
        "no-repeat" => Some("no-repeat"),
        _ => None,
    }
}

fn background_repeat_layer(components: &[&[Token]]) -> Option<String> {
    match components {
        [one] if ident_of(one).is_some_and(|id| id.eq_ignore_ascii_case("repeat-x")) => {
            Some("repeat-x".to_string())
        }
        [one] if ident_of(one).is_some_and(|id| id.eq_ignore_ascii_case("repeat-y")) => {
            Some("repeat-y".to_string())
        }
        [one] => repeat_word(one).map(str::to_string),
        [a, b] => {
            let wa = repeat_word(a)?;
            let wb = repeat_word(b)?;
            Some(if wa == wb {
                wa.to_string()
            } else {
                format!("{wa} {wb}")
            })
        }
        _ => None,
    }
}

pub(crate) fn valid_background_repeat(value: &str) -> bool {
    background_repeat_computed(value).is_some()
}

pub(crate) fn background_repeat_computed(value: &str) -> Option<String> {
    list_layers(value, background_repeat_layer)
}

// ---------------------------------------------------------------------------
// background-size: `<bg-size>#` = [ [<length-percentage>|auto]{1,2} | cover |
// contain ]#. Per WPT, the computed value always has two components unless
// both are literally `auto` (which collapses to the single keyword `auto`);
// a single specified component's missing second defaults to `auto`. Negative
// lengths/percentages are rejected; a calc()-derived value is not sign
// checked at parse-validity time (CSS Values 4 clamps out-of-range calc()
// results at computed-value time instead), matching `box-shadow`'s policy.

enum SizeComponent {
    Auto,
    LengthPercentage(String),
}

fn size_component(component: &[Token]) -> Option<SizeComponent> {
    if ident_of(component).is_some_and(|id| id.eq_ignore_ascii_case("auto")) {
        return Some(SizeComponent::Auto);
    }
    let text = serialize(component);
    if let Some(length) = LengthPercentage::parse(&text) {
        return length
            .is_nonnegative()
            .then(|| SizeComponent::LengthPercentage(text));
    }
    crate::math::length_percentage(&text, true).then(|| SizeComponent::LengthPercentage(text))
}

fn join_size(x: SizeComponent, y: SizeComponent) -> String {
    match (x, y) {
        (SizeComponent::Auto, SizeComponent::Auto) => "auto".to_string(),
        (SizeComponent::Auto, SizeComponent::LengthPercentage(y)) => format!("auto {y}"),
        (SizeComponent::LengthPercentage(x), SizeComponent::Auto) => format!("{x} auto"),
        (SizeComponent::LengthPercentage(x), SizeComponent::LengthPercentage(y)) => {
            format!("{x} {y}")
        }
    }
}

fn background_size_layer(components: &[&[Token]]) -> Option<String> {
    match components {
        [one] if ident_of(one).is_some_and(|id| id.eq_ignore_ascii_case("cover")) => {
            Some("cover".to_string())
        }
        [one] if ident_of(one).is_some_and(|id| id.eq_ignore_ascii_case("contain")) => {
            Some("contain".to_string())
        }
        [a] => Some(join_size(size_component(a)?, SizeComponent::Auto)),
        [a, b] => Some(join_size(size_component(a)?, size_component(b)?)),
        _ => None,
    }
}

pub(crate) fn valid_background_size(value: &str) -> bool {
    background_size_computed(value).is_some()
}

pub(crate) fn background_size_computed(value: &str) -> Option<String> {
    list_layers(value, background_size_layer)
}

// ---------------------------------------------------------------------------
// background-position / background-position-x / background-position-y:
// CSS Backgrounds 3 SS3.9's `<bg-position>` resolution algorithm.

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

fn is_center(component: &[Token]) -> bool {
    ident_of(component).is_some_and(|id| id.eq_ignore_ascii_case("center"))
}

fn edge_of(component: &[Token]) -> Option<Edge> {
    match ident_of(component)?.to_ascii_lowercase().as_str() {
        "left" => Some(Edge::Left),
        "right" => Some(Edge::Right),
        "top" => Some(Edge::Top),
        "bottom" => Some(Edge::Bottom),
        _ => None,
    }
}

/// The writing-mode-relative `x-start`/`x-end`/`y-start`/`y-end` keywords
/// (CSS Backgrounds 4). This crate has no writing-mode/direction context to
/// resolve them to a physical percentage, so -- like an unresolved `em`
/// offset -- they stay opaque, passed through as their own literal text.
fn logical_edge_of(component: &[Token]) -> Option<&'static str> {
    match ident_of(component)?.to_ascii_lowercase().as_str() {
        "x-start" => Some("x-start"),
        "x-end" => Some("x-end"),
        "y-start" => Some("y-start"),
        "y-end" => Some("y-end"),
        _ => None,
    }
}

fn is_length_percentage_component(component: &[Token]) -> bool {
    let text = serialize(component);
    LengthPercentage::parse(&text).is_some() || crate::math::length_percentage(&text, true)
}

fn lp_text(component: &[Token]) -> String {
    serialize(component)
}

fn fmt_num(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// The value an offset from the *end* edge (`right`/`bottom`) resolves to:
/// `100% - offset`, done as plain percentage arithmetic when the offset is a
/// literal percentage, or as a `calc(100% - <length>)` expression (sign
/// flipped to `+` for a negative offset) when it is a literal absolute
/// length. A `rem`/`em`-relative or calc()-derived offset cannot be resolved
/// without cascade context this crate does not have here; it is embedded
/// best-effort into the same `calc(100% - ...)` shape without sign
/// normalization -- already a documented boundary case, and never a
/// regression since no grammar existed for this property before CSS-30.
fn complement_text(component: &[Token]) -> String {
    let text = serialize(component);
    match LengthPercentage::parse(&text) {
        Some(LengthPercentage::Percentage(v)) => format!("{}%", fmt_num(100.0 - v)),
        Some(LengthPercentage::Px(v)) if v >= 0.0 => format!("calc(100% - {}px)", fmt_num(v)),
        Some(LengthPercentage::Px(v)) => format!("calc(100% + {}px)", fmt_num(-v)),
        _ => format!("calc(100% - {text})"),
    }
}

/// Resolves one component of a single-axis longhand (`background-position-x`
/// or `background-position-y`), with no offset chunking (1-token form only).
fn resolve_axis_component(component: &[Token], is_x: bool) -> Option<String> {
    if is_center(component) {
        return Some("50%".to_string());
    }
    if let Some(logical) = logical_edge_of(component) {
        let matches_axis = if is_x {
            logical.starts_with('x')
        } else {
            logical.starts_with('y')
        };
        return matches_axis.then(|| logical.to_string());
    }
    if let Some(edge) = edge_of(component) {
        let matches_axis = if is_x {
            matches!(edge, Edge::Left | Edge::Right)
        } else {
            matches!(edge, Edge::Top | Edge::Bottom)
        };
        if !matches_axis {
            return None;
        }
        return Some(match edge {
            Edge::Left | Edge::Top => "0%".to_string(),
            Edge::Right | Edge::Bottom => "100%".to_string(),
        });
    }
    is_length_percentage_component(component).then(|| lp_text(component))
}

fn position_axis_layer(components: &[&[Token]], is_x: bool) -> Option<String> {
    match components {
        [one] => resolve_axis_component(one, is_x),
        [a, b] => {
            let edge = edge_of(a)?;
            let matches_axis = if is_x {
                matches!(edge, Edge::Left | Edge::Right)
            } else {
                matches!(edge, Edge::Top | Edge::Bottom)
            };
            if !matches_axis || !is_length_percentage_component(b) {
                return None;
            }
            Some(match edge {
                Edge::Left | Edge::Top => lp_text(b),
                Edge::Right | Edge::Bottom => complement_text(b),
            })
        }
        _ => None,
    }
}

pub(crate) fn valid_background_position_axis(value: &str, is_x: bool) -> bool {
    background_position_axis_computed(value, is_x).is_some()
}

pub(crate) fn background_position_axis_computed(value: &str, is_x: bool) -> Option<String> {
    list_layers(value, |components| position_axis_layer(components, is_x))
}

/// One `&&`-combined chunk of the 3-4-token edge+offset shorthand grammar:
/// a lone `center`, or an edge keyword with its resolved text already
/// computed (the `left`/`top` vs `right`/`bottom` resolution rule is
/// independent of which axis the chunk ends up assigned to).
enum Chunk {
    Center,
    Edge { is_x: bool, text: String },
}

fn chunk_layer(components: &[&[Token]]) -> Option<Vec<Chunk>> {
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < components.len() {
        let component = components[i];
        if is_center(component) {
            chunks.push(Chunk::Center);
            i += 1;
            continue;
        }
        let Some(edge) = edge_of(component) else {
            return None;
        };
        let is_x = matches!(edge, Edge::Left | Edge::Right);
        let is_start = matches!(edge, Edge::Left | Edge::Top);
        let offset = if i + 1 < components.len() && is_length_percentage_component(components[i + 1]) {
            Some(components[i + 1])
        } else {
            None
        };
        let text = match offset {
            None if is_start => "0%".to_string(),
            None => "100%".to_string(),
            Some(tok) if is_start => lp_text(tok),
            Some(tok) => complement_text(tok),
        };
        chunks.push(Chunk::Edge { is_x, text });
        i += if offset.is_some() { 2 } else { 1 };
    }
    (chunks.len() == 2).then_some(chunks)
}

/// Assigns two chunks to (x, y) by keyword identity: a `Center` chunk takes
/// whichever axis the other chunk did not claim; two `Edge` chunks must
/// claim different axes.
fn assign_axes(mut chunks: Vec<Chunk>) -> Option<(String, String)> {
    let c1 = chunks.pop().unwrap();
    let c0 = chunks.pop().unwrap();
    let axis0 = match &c0 {
        Chunk::Center => None,
        Chunk::Edge { is_x, .. } => Some(*is_x),
    };
    let axis1 = match &c1 {
        Chunk::Center => None,
        Chunk::Edge { is_x, .. } => Some(*is_x),
    };
    let text = |c: Chunk| match c {
        Chunk::Center => "50%".to_string(),
        Chunk::Edge { text, .. } => text,
    };
    match (axis0, axis1) {
        (Some(true), Some(false)) => Some((text(c0), text(c1))),
        (Some(false), Some(true)) => Some((text(c1), text(c0))),
        (Some(true), None) => Some((text(c0), text(c1))),
        (None, Some(false)) => Some((text(c0), text(c1))),
        (None, Some(true)) => Some((text(c1), text(c0))),
        (Some(false), None) => Some((text(c1), text(c0))),
        (None, None) => Some((text(c0), text(c1))),
        _ => None,
    }
}

/// Classification for the strict 2-token form (no offset chunking): a
/// keyword's own axis identity can force a swap of which slot binds to X vs
/// Y, but a bare length-percentage has no axis affinity of its own.
#[derive(Clone, Copy)]
enum TwoToken {
    Center,
    X(Edge),
    Y(Edge),
    LengthPercentage,
}

fn classify_two_token(component: &[Token]) -> Option<TwoToken> {
    if is_center(component) {
        return Some(TwoToken::Center);
    }
    if let Some(edge) = edge_of(component) {
        return Some(if matches!(edge, Edge::Left | Edge::Right) {
            TwoToken::X(edge)
        } else {
            TwoToken::Y(edge)
        });
    }
    is_length_percentage_component(component).then_some(TwoToken::LengthPercentage)
}

fn position_shorthand_layer(components: &[&[Token]]) -> Option<String> {
    match components.len() {
        1 => {
            let component = components[0];
            if is_center(component) {
                return Some("50% 50%".to_string());
            }
            if let Some(edge) = edge_of(component) {
                return Some(
                    match edge {
                        Edge::Left => "0% 50%",
                        Edge::Right => "100% 50%",
                        Edge::Top => "50% 0%",
                        Edge::Bottom => "50% 100%",
                    }
                    .to_string(),
                );
            }
            is_length_percentage_component(component)
                .then(|| format!("{} 50%", lp_text(component)))
        }
        2 => {
            let a = classify_two_token(components[0])?;
            let b = classify_two_token(components[1])?;
            let swap = matches!(a, TwoToken::Y(_)) || matches!(b, TwoToken::X(_));
            let (xc, yc, xk, yk) = if swap {
                (components[1], components[0], b, a)
            } else {
                (components[0], components[1], a, b)
            };
            if matches!(xk, TwoToken::Y(_)) || matches!(yk, TwoToken::X(_)) {
                return None;
            }
            let x_text = match xk {
                TwoToken::Center => "50%".to_string(),
                TwoToken::X(Edge::Left) => "0%".to_string(),
                TwoToken::X(Edge::Right) => "100%".to_string(),
                TwoToken::LengthPercentage => lp_text(xc),
                _ => return None,
            };
            let y_text = match yk {
                TwoToken::Center => "50%".to_string(),
                TwoToken::Y(Edge::Top) => "0%".to_string(),
                TwoToken::Y(Edge::Bottom) => "100%".to_string(),
                TwoToken::LengthPercentage => lp_text(yc),
                _ => return None,
            };
            Some(format!("{x_text} {y_text}"))
        }
        3 | 4 => {
            let chunks = chunk_layer(components)?;
            let (x_text, y_text) = assign_axes(chunks)?;
            Some(format!("{x_text} {y_text}"))
        }
        _ => None,
    }
}

pub(crate) fn valid_background_position(value: &str) -> bool {
    background_position_computed(value).is_some()
}

pub(crate) fn background_position_computed(value: &str) -> Option<String> {
    list_layers(value, position_shorthand_layer)
}
