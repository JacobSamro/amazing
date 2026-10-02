//! CSS Containment size axes and CSS Sizing 4 explicit intrinsic inner sizes.
//! Computed physical longhands own cascade order. The shorthand fallback supports
//! older CSS producers; remembered sizes are optional state supplied by a caller.

use amazing_css::ComputedStyle;
use crate::values::{self, FontRelativeUnits};

/// Layout/paint containment share formatting-context and positioning effects.
/// Unlike size containment, table cells and table wrappers remain eligible.
pub(crate) fn layout_containment(style: &ComputedStyle) -> bool {
    applies_layout(style) && (style.get("contain").unwrap_or("none")
        .split_ascii_whitespace().any(|v| matches!(v, "layout" | "content" | "strict"))
        || matches!(style.get("content-visibility"), Some("auto" | "hidden")))
}

pub(crate) fn independent_context(style: &ComputedStyle) -> bool {
    layout_containment(style) || (applies_layout(style) && style.get("contain")
        .is_some_and(|value| value.split_ascii_whitespace().any(|v| v == "paint")))
}

fn applies_layout(style: &ComputedStyle) -> bool {
    !matches!(style.get("display"), Some("none" | "contents" | "inline"
        | "table-row" | "table-row-group" | "table-column" | "table-column-group"
        | "table-header-group" | "table-footer-group"
        | "ruby" | "ruby-base" | "ruby-text" | "ruby-base-container" | "ruby-text-container"))
}

/// None means the axis is not size-contained; Some(0) means containment sizes
/// that axis as empty. Values are content-box lengths, regardless of box-sizing.
pub(crate) fn intrinsic_inner_sizes(
    style: &ComputedStyle,
    units: FontRelativeUnits,
    remembered: Option<(f32, f32)>,
) -> (Option<f32>, Option<f32>) {
    let contain = style.get("contain").unwrap_or("none");
    let full = contain.split_ascii_whitespace().any(|v| matches!(v, "size" | "strict"))
        || style.get("content-visibility") == Some("hidden");
    let inline = full || contain.split_ascii_whitespace().any(|v| v == "inline-size");
    if !inline { return (None, None); }
    // Size containment does not apply to non-atomic inlines or table boxes.
    if matches!(style.get("display"), Some("inline" | "table" | "inline-table"
        | "table-row" | "table-row-group" | "table-column" | "table-column-group"
        | "table-header-group" | "table-footer-group" | "table-cell"
        | "ruby" | "ruby-base" | "ruby-text" | "ruby-base-container" | "ruby-text-container")) {
        return (None, None);
    }
    let vertical = matches!(style.get("writing-mode"),
        Some("vertical-lr" | "vertical-rl" | "sideways-lr" | "sideways-rl"));
    let fallback = style.get("contain-intrinsic-size").and_then(shorthand_axes);
    let width_source = style.get("contain-intrinsic-width")
        .or_else(|| fallback.as_ref().map(|axes| axes.0.as_str()));
    let height_source = style.get("contain-intrinsic-height")
        .or_else(|| fallback.as_ref().map(|axes| axes.1.as_str()));
    let resolve = |source, remembered| intrinsic_axis(style, source, units, remembered).unwrap_or(0.0);
    (
        (full || !vertical).then(|| resolve(width_source, remembered.map(|s| s.0))),
        (full || vertical).then(|| resolve(height_source, remembered.map(|s| s.1))),
    )
}

/// Intrinsic-query callers without a text provider share the existing fallback
/// font units. Block layout calls the resolver with its provider metrics.
pub(crate) fn intrinsic_width(box_: &crate::LayoutBox) -> Option<f32> {
    let units = FontRelativeUnits::from_text_system(None,
        box_.font_metric_style.as_ref(), box_.root_font_metric_style.as_ref(), box_.font_size)
        .with_line_heights(box_.line_height, box_.root_line_height);
    intrinsic_inner_sizes(&box_.style, units, None).0
}

fn intrinsic_axis(
    style: &ComputedStyle,
    source: Option<&str>,
    units: FontRelativeUnits,
    remembered: Option<f32>,
) -> Option<f32> {
    let source = source?.trim();
    let source = if let Some(rest) = source.strip_prefix("auto ") {
        if let Some(size) = remembered.filter(|s| s.is_finite() && *s >= 0.0) {
            return Some(size);
        }
        rest.trim()
    } else { source };
    if source == "none" { return None; }
    let length = values::used_length_percentage_with_units(style, source, units)?;
    (!length.has_percentage && length.px >= 0.0).then_some(length.px)
}

/// Whitespace separates axes only outside a math function; auto belongs to the
/// following none/length component, rather than being an axis of its own.
fn shorthand_axes(source: &str) -> Option<(String, String)> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (i, ch) in source.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            _ if ch.is_ascii_whitespace() && depth == 0 => {
                if start < i { tokens.push(&source[start..i]); }
                start = i + ch.len_utf8();
            }
            _ => {},
        }
    }
    if depth != 0 { return None; }
    if start < source.len() { tokens.push(&source[start..]); }
    let mut axes = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "auto" {
            i += 1;
            axes.push(format!("auto {}", tokens.get(i)?));
        } else { axes.push(tokens[i].to_owned()); }
        i += 1;
    }
    match axes.as_slice() {
        [one] => Some((one.clone(), one.clone())),
        [width, height] => Some((width.clone(), height.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn style(entries: &[(&str, &str)]) -> ComputedStyle {
        entries.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect()
    }
    #[test]
    fn layout_effects_respect_internal_box_exclusions_and_visibility() {
        for display in ["block", "inline-block", "table", "inline-table", "table-cell", "table-caption", "flex", "grid"] {
            for contain in ["layout", "paint", "content", "strict"] {
                assert!(independent_context(&style(&[("display",display),("contain",contain)])), "{display}/{contain}");
            }
        }
        for display in ["none", "contents", "inline", "table-row", "table-column", "table-row-group", "ruby", "ruby-base", "ruby-text"] {
            assert!(!independent_context(&style(&[("display",display),("contain","strict")])), "{display}");
        }
        for visibility in ["auto", "hidden"] {
            assert!(layout_containment(&style(&[("content-visibility",visibility)])));
        }
        assert!(!layout_containment(&style(&[("contain","paint")])));
        assert!(!independent_context(&style(&[("contain","size style")])));
    }

    #[test]
    fn independent_axes_and_math_keep_none_distinct_from_omission() {
        for (value, expected) in [
            ("10px none", (Some(10.0), Some(0.0))),
            ("none 20px", (Some(0.0), Some(20.0))),
            ("calc(10px + 5px) 20px", (Some(15.0), Some(20.0))),
            ("auto 10px auto none", (Some(10.0), Some(0.0))),
            ("none", (Some(0.0), Some(0.0))),
        ] {
            let s = style(&[("contain", "size"), ("contain-intrinsic-size", value)]);
            assert_eq!(intrinsic_inner_sizes(&s, FontRelativeUnits::legacy(16.0, 8.0), None), expected);
        }
    }
    #[test]
    fn negative_and_percentage_fallbacks_cannot_size_contained_content() {
        for value in ["-10px", "10%", "calc(10px + 2%)"] {
            let s = style(&[("contain", "size"), ("contain-intrinsic-size", value)]);
            assert_eq!(intrinsic_inner_sizes(&s, FontRelativeUnits::legacy(16.0, 8.0), None),
                (Some(0.0), Some(0.0)));
        }
        let table = style(&[("display", "table-cell"), ("contain", "size"),
            ("contain-intrinsic-size", "10px")]);
        assert_eq!(intrinsic_inner_sizes(&table, FontRelativeUnits::legacy(16.0, 8.0), None), (None, None));
        let caption = style(&[("display", "table-caption"), ("contain", "size"),
            ("contain-intrinsic-size", "10px")]);
        assert_eq!(intrinsic_inner_sizes(&caption, FontRelativeUnits::legacy(16.0, 8.0), None), (Some(10.0), Some(10.0)));
    }

    #[test]
    fn resolved_longhands_and_remembered_sizes_override_only_their_axes() {
        let s = style(&[("contain", "size"), ("contain-intrinsic-size", "40px"),
            ("contain-intrinsic-width", "auto 10px"), ("contain-intrinsic-height", "none")]);
        let units = FontRelativeUnits::legacy(16.0, 8.0);
        assert_eq!(intrinsic_inner_sizes(&s, units, None), (Some(10.0), Some(0.0)));
        assert_eq!(intrinsic_inner_sizes(&s, units, Some((23.0, 34.0))), (Some(23.0), Some(0.0)));
    }
    #[test]
    fn inline_size_containment_maps_to_the_writing_mode_axis() {
        for (mode, expected) in [("horizontal-tb", (Some(12.0), None)),
            ("vertical-rl", (None, Some(24.0))), ("sideways-lr", (None, Some(24.0)))] {
            let s = style(&[("contain", "inline-size layout"), ("writing-mode", mode),
                ("contain-intrinsic-size", "12px 24px")]);
            assert_eq!(intrinsic_inner_sizes(&s, FontRelativeUnits::legacy(16.0, 8.0), None), expected);
        }
    }
}
