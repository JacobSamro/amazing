//! Driver adapter from retained layout damage to paint-owned item deltas.
//!
//! LAYOUT-17 can populate this small input after relayout. The adapter resolves
//! the live DOM arena generation at the update boundary; removed items retain
//! their old key because their DOM node may already be gone.

use amazing_dom::Dom;
use amazing_layout::LayoutBox;
use amazing_paint::controls::ControlProvider;
use amazing_paint::display_list_view::{
    DisplayListView, PaintDamage, PaintFallbackReason, PaintItem, PaintNodeKey, RetainedPaintDelta,
    RetainedPaintView, StablePaintItemKey,
};
use amazing_paint::ImageProvider;
use amazing_text::TextSystem;

/// One affected layout item. `item` carries new geometry, property IDs and
/// display position; the adapter fills its stable key from the live DOM.
/// LAYOUT-17 supplies only affected boxes rather than a next full paint view.
pub(crate) enum LayoutPaintDamage<'a> {
    Remove(StablePaintItemKey),
    Upsert {
        box_: &'a LayoutBox,
        item: PaintItem,
        ordinal: u32,
    },
}

/// Capture the initial full scene while the layout tree and DOM are frozen.
/// The returned scene owns its item pixels and outlives this borrowed view.
pub(crate) fn seed_retained_paint(
    root: &LayoutBox,
    width: u32,
    height: u32,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    controls: &dyn ControlProvider,
    dom: &Dom,
) -> Result<RetainedPaintView, PaintFallbackReason> {
    let view =
        DisplayListView::with_dom_generations(root, width, height, text, images, controls, dom);
    RetainedPaintView::from_view(&view)
}

/// Convert layout's bounded damage and update only those paint chunks. A
/// missing/reused DOM generation yields a full-rebuild reason before mutation.
pub(crate) fn apply_layout_paint_damage(
    retained: &mut RetainedPaintView,
    damage: &[LayoutPaintDamage<'_>],
    dom: &Dom,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
) -> Result<RetainedPaintDelta, PaintFallbackReason> {
    let mut paint_damage = Vec::with_capacity(damage.len());
    for entry in damage {
        match entry {
            LayoutPaintDamage::Remove(key) => paint_damage.push(PaintDamage::Remove(*key)),
            LayoutPaintDamage::Upsert {
                box_,
                item,
                ordinal,
            } => {
                let node = box_.node.ok_or(PaintFallbackReason::AmbiguousGeneration)?;
                let generation = dom
                    .node_generation(node)
                    .ok_or(PaintFallbackReason::AmbiguousGeneration)?;
                let mut item = *item;
                item.key = Some(StablePaintItemKey {
                    node: PaintNodeKey { node, generation },
                    phase: item.kind,
                    ordinal: *ordinal,
                });
                paint_damage.push(PaintDamage::Upsert { item, box_ });
            }
        }
    }
    retained.apply_damage(&paint_damage, text, images)
}
