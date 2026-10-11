//! Authentic item measurement for both Grid Lanes orientations. Dense
//! remapping preserves used dimensions and rebuilds inherited subgrid tracks.
use super::*;

pub(super) fn measure_column_lane(
    it: &mut LayoutBox,
    spec: &GridSpec,
    sizes: &[f32],
    start: usize,
    span: usize,
    area_w: f32,
    fonts: &FontSource,
) -> f32 {
    let cols = subgrid_axis_for_item(
        it,
        "grid-template-columns",
        sizes,
        spec.col_gap,
        &spec.col_line_names,
        start + 1,
        span,
    );
    it.grid_subgrid = cols.map(|cols| SubgridContext {
        cols: Some(cols),
        rows: None,
    });
    it.margin = values::resolve_margins(&it.style, it.font_size, area_w);
    it.padding = values::resolve_padding(&it.style, it.font_size, area_w);
    it.border = values::resolve_borders(&it.style, it.font_size);
    let available =
        (area_w - it.margin.left.unwrap_or(0.0) - it.margin.right.unwrap_or(0.0) - h_chrome(it))
            .max(0.0);
    // Grid Lanes 3 alignment: the grid axis aligns each item inside its
    // lane area with justify-self / justify-items; only `stretch` fills.
    let mut justify = container_self_align(it, "justify-self").unwrap_or(spec.justify_items);
    let width = match crate::used_size::extent(it, "width", area_w) {
        MaybeLen::Px(width) => {
            if justify == ItemAlign::Stretch {
                justify = ItemAlign::Start;
            }
            width.max(0.0)
        }
        MaybeLen::Auto if justify == ItemAlign::Stretch => available,
        MaybeLen::Auto => (crate::flex::max_content_width(it, fonts) - h_chrome(it))
            .max(0.0)
            .min(available),
    };
    if spec.rtl {
        justify = match justify {
            ItemAlign::Start => ItemAlign::End,
            ItemAlign::End => ItemAlign::Start,
            other => other,
        };
    }
    crate::block::layout_block_with_used_content_size(
        it,
        0.0,
        0.0,
        area_w,
        fonts,
        None,
        crate::UsedContentSize {
            width: Some(width),
            height: None,
        },
    );
    let (ml, mr) = (it.margin.left.unwrap_or(0.0), it.margin.right.unwrap_or(0.0));
    let free = area_w - ml - mr - it.border_box.w;
    if margin_is_auto(&it.style, "left") || margin_is_auto(&it.style, "right") {
        ml
    } else {
        match justify {
            ItemAlign::End => ml + free.max(0.0),
            ItemAlign::Center => ml + (free / 2.0).max(0.0),
            _ => ml,
        }
    }
}

pub(super) fn measure_row_lane(
    it: &mut LayoutBox,
    spec: &GridSpec,
    container: &ComputedStyle,
    sizes: &[f32],
    start: usize,
    span: usize,
    area_h: f32,
    content_w: f32,
    fonts: &FontSource,
) {
    it.containing_block_height = Some(area_h);
    let rows = subgrid_axis_for_item(
        it,
        "grid-template-rows",
        sizes,
        spec.row_gap,
        &spec.row_line_names,
        start + 1,
        span,
    );
    it.grid_subgrid = rows.map(|rows| SubgridContext {
        cols: None,
        rows: Some(rows),
    });
    let width = match crate::used_size::extent(it, "width", content_w) {
        MaybeLen::Px(width) => width.max(0.0),
        MaybeLen::Auto => (crate::flex::max_content_width(it, fonts) - h_chrome(it))
            .max(0.0)
            .min((content_w - h_chrome(it)).max(0.0)),
    };
    let mut used = crate::UsedContentSize {
        width: Some(width),
        height: None,
    };
    let align = container_self_align(it, "align-self").unwrap_or(spec.align_items);
    if height_is_auto(it)
        && row_baseline::preference(it, container).is_none()
        && align == ItemAlign::Stretch
        && !margin_is_auto(&it.style, "top")
        && !margin_is_auto(&it.style, "bottom")
    {
        let margin = values::resolve_margins(&it.style, it.font_size, content_w);
        let padding = values::resolve_padding(&it.style, it.font_size, content_w);
        let border = values::resolve_borders(&it.style, it.font_size);
        let chrome = margin.top.unwrap_or(0.0)
            + margin.bottom.unwrap_or(0.0)
            + padding.top
            + padding.bottom
            + border.top
            + border.bottom;
        used.height = Some((area_h - chrome).max(0.0));
    }
    crate::block::layout_block_with_used_content_size(it, 0.0, 0.0, content_w, fonts, None, used);
}

/// Offset of the item's border box below its row-lane area top (margin, auto
/// margins and `align-self` included).
pub(super) fn row_lane_lead(
    it: &LayoutBox,
    spec: &GridSpec,
    container: &ComputedStyle,
    area_h: f32,
    content_w: f32,
) -> f32 {
    let margin = values::resolve_margins(&it.style, it.font_size, content_w);
    let align = container_self_align(it, "align-self").unwrap_or(spec.align_items);
    let auto_top = margin_is_auto(&it.style, "top");
    let auto_bottom = margin_is_auto(&it.style, "bottom");
    let free = (area_h - it.border_box.h - margin.top.unwrap_or(0.0)
        - margin.bottom.unwrap_or(0.0))
    .max(0.0);
    let auto_lead = if auto_top && auto_bottom {
        free * 0.5
    } else if auto_top {
        free
    } else {
        0.0
    };
    let align_lead = if auto_top || auto_bottom || row_baseline::preference(it, container).is_some() {
        0.0
    } else {
        match align {
            ItemAlign::End => free,
            ItemAlign::Center => free / 2.0,
            _ => 0.0,
        }
    };
    margin.top.unwrap_or(0.0) + auto_lead + align_lead
}
