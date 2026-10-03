//! Convert definite author dimensions to content sizes without altering style.
use crate::{
    Flow, LayoutBox,
    values::{self, MaybeLen},
};
/// Sizing belongs to the original table grid; parent item algorithms must
/// not infer auto from its anonymous positioning wrapper's initial values.
pub(crate) fn style(b: &LayoutBox) -> &amazing_css::ComputedStyle {
    if b.table_wrapper
        && let Flow::Blocks(children) = &b.flow
        && let Some(grid) = children.first()
    {
        return &grid.style;
    }
    &b.style
}
pub(crate) fn content_value(b: &LayoutBox, value: f32, horizontal: bool, basis: f32) -> f32 {
    if b.table_wrapper
        && let Flow::Blocks(children) = &b.flow
        && let Some(grid) = children.first()
    {
        let padding = values::resolve_padding(&grid.style, grid.font_size, basis);
        let border = values::resolve_borders(&grid.style, grid.font_size);
        if horizontal {
            return crate::table::outer_table_width(
                grid,
                value,
                padding.left + padding.right + border.left + border.right,
            )
            .max(0.0);
        }
        let captions = (b.border_box.h - grid.border_box.h).max(0.0);
        let edges = padding.top + padding.bottom + border.top + border.bottom;
        return value.max(0.0)
            + captions
            + if grid.style.get("box-sizing") == Some("border-box") {
                0.0
            } else {
                edges
            };
    }
    if b.style.get("box-sizing") != Some("border-box") {
        return value.max(0.0);
    }
    let padding = values::resolve_padding(&b.style, b.font_size, basis);
    let border = values::resolve_borders(&b.style, b.font_size);
    let edges = if horizontal {
        padding.left + padding.right + border.left + border.right
    } else {
        padding.top + padding.bottom + border.top + border.bottom
    };
    (value - edges).max(0.0)
}
pub(crate) fn extent(b: &LayoutBox, property: &str, basis: f32) -> MaybeLen {
    match values::resolve_extent(style(b), property, b.font_size, basis) {
        MaybeLen::Px(value) => {
            MaybeLen::Px(content_value(b, value, property.ends_with("width"), basis))
        }
        MaybeLen::Auto => MaybeLen::Auto,
    }
}
