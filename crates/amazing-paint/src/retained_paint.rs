//! Owned display items updated from keyed layout/style damage.
//!
//! A full borrowed view seeds the scene once. Later updates receive only
//! changed boxes; unchanged item rasters and resource identities stay owned.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;

use amazing_layout::LayoutBox;

use super::delta::damage_bounds;
use super::{
    DisplayListView, PaintBounds, PaintFallbackReason, PaintItem, PaintItemChange, PaintItemKind,
    PaintResourceKey, StablePaintItemKey,
};
use crate::background::BackgroundPlan;
use crate::display_list::PaintCommand;
use crate::paint::{self, ImageProvider};
use crate::surface::Surface;
use amazing_text::TextSystem;

const MAX_DAMAGE_ITEMS: usize = 512;
const MAX_TILE_PIXELS: usize = 1_048_576;
const MAX_RETAINED_PIXELS: usize = 16_777_216;

/// The changed item metadata comes from layout's geometry/style damage. The
/// layout box is borrowed only while this call rasterizes its changed chunk.
/// `item.index` is the item's position in the resulting display order.
pub enum PaintDamage<'a> {
    Remove(StablePaintItemKey),
    Upsert {
        item: PaintItem,
        box_: &'a LayoutBox,
    },
}

/// One owned, local RGBA raster. The property tree positions it in the scene.
pub struct RetainedRaster<'a> {
    pub item: PaintItem,
    pub origin: (i32, i32),
    pub surface: &'a Surface,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RetainedPaintDelta {
    pub changes: Vec<PaintItemChange>,
    pub changed_ranges: Vec<Range<usize>>,
    pub added_keys: Vec<StablePaintItemKey>,
    pub removed_keys: Vec<StablePaintItemKey>,
    pub retired_resources: Vec<PaintResourceKey>,
}

struct OwnedItem {
    item: PaintItem,
    origin: (i32, i32),
    surface: Surface,
    resources: BTreeSet<PaintResourceKey>,
}

/// Retained paint state independent of the seed layout tree's lifetime.
pub struct RetainedPaintView {
    items: Vec<OwnedItem>,
    ids: HashMap<StablePaintItemKey, usize>,
    property_counts: (usize, usize, usize, usize),
}

impl RetainedPaintView {
    /// Seed from a complete view at scene creation. Unsupported raw chunks
    /// require the existing full painter/compositor path.
    pub fn from_view(view: &DisplayListView<'_>) -> Result<Self, PaintFallbackReason> {
        if view.property_owner_mapping().is_none() {
            return Err(PaintFallbackReason::AmbiguousPropertyOwner);
        }
        if view.items().len() > MAX_DAMAGE_ITEMS {
            return Err(PaintFallbackReason::DiffBudgetExceeded);
        }
        let mut items = Vec::with_capacity(view.items().len());
        let mut ids = HashMap::new();
        let mut compositor_ids = HashMap::new();
        let mut retained_pixels = 0usize;
        for item in view.items() {
            let key = item.key.ok_or(PaintFallbackReason::AmbiguousGeneration)?;
            if ids.insert(key, item.index).is_some() {
                return Err(PaintFallbackReason::DuplicateKey);
            }
            if compositor_ids.insert(key.compositor_id(), key).is_some() {
                return Err(PaintFallbackReason::KeyCollision);
            }
            let (origin, mut surface) = tile(item.local_bounds)?;
            retained_pixels =
                retained_pixels.saturating_add(surface.width as usize * surface.height as usize);
            if retained_pixels > MAX_RETAINED_PIXELS {
                return Err(PaintFallbackReason::DiffBudgetExceeded);
            }
            if surface.width != 0 && surface.height != 0 {
                view.raster_raw_local(item.index..item.index + 1, &mut surface, origin)
                    .map_err(|_| PaintFallbackReason::UnseparableGroup)?;
            }
            items.push(OwnedItem {
                item: *item,
                origin,
                surface,
                resources: resources_for_command(view, item.index)?,
            });
        }
        Ok(Self {
            items,
            ids,
            property_counts: (
                view.transforms().len(),
                view.clips().len(),
                view.effects().len(),
                view.scrolls().len(),
            ),
        })
    }

    pub fn items(&self) -> impl Iterator<Item = PaintItem> + '_ {
        self.items.iter().map(|owned| owned.item)
    }

    pub fn raster(&self, key: StablePaintItemKey) -> Option<RetainedRaster<'_>> {
        let owned = &self.items[*self.ids.get(&key)?];
        Some(RetainedRaster {
            item: owned.item,
            origin: owned.origin,
            surface: &owned.surface,
        })
    }

    /// Apply a bounded keyed damage set without constructing a next full
    /// display view or rasterizing any unchanged item. On error, the scene is
    /// untouched and the caller must rebuild from a complete view.
    pub fn apply_damage(
        &mut self,
        damage: &[PaintDamage<'_>],
        text: &dyn TextSystem,
        images: &dyn ImageProvider,
    ) -> Result<RetainedPaintDelta, PaintFallbackReason> {
        if damage.len() > MAX_DAMAGE_ITEMS {
            return Err(PaintFallbackReason::DiffBudgetExceeded);
        }
        let mut seen = HashSet::new();
        let mut prepared = Vec::with_capacity(damage.len());
        let mut prepared_pixels = 0usize;
        let mut final_pixels: usize = self
            .items
            .iter()
            .map(|owned| owned.surface.width as usize * owned.surface.height as usize)
            .sum();
        let mut order: Vec<_> = self
            .items
            .iter()
            .map(|owned| owned.item.key.unwrap())
            .collect();
        let mut compositor_ids: HashMap<_, _> = order
            .iter()
            .map(|key| (key.compositor_id(), *key))
            .collect();
        for entry in damage {
            match entry {
                PaintDamage::Remove(key) => {
                    if !seen.insert(*key) {
                        return Err(PaintFallbackReason::DuplicateKey);
                    }
                    let Some(index) = order.iter().position(|candidate| candidate == key) else {
                        return Err(PaintFallbackReason::OrderChanged);
                    };
                    order.remove(index);
                    compositor_ids.remove(&key.compositor_id());
                    let old = &self.items[self.ids[key]];
                    final_pixels -= old.surface.width as usize * old.surface.height as usize;
                    prepared.push(Prepared::Remove(*key));
                }
                PaintDamage::Upsert { item, box_ } => {
                    let key = item.key.ok_or(PaintFallbackReason::AmbiguousGeneration)?;
                    if !seen.insert(key) {
                        return Err(PaintFallbackReason::DuplicateKey);
                    }
                    if box_.node != Some(key.node.node) || item.kind == PaintItemKind::Control {
                        return Err(PaintFallbackReason::AmbiguousGeneration);
                    }
                    if !self.valid_properties(item) {
                        return Err(PaintFallbackReason::PropertyStateChanged);
                    }
                    let old = self.ids.get(&key).map(|index| self.items[*index].item);
                    if old.is_some_and(|old| {
                        old.properties != item.properties || old.kind != item.kind
                    }) {
                        return Err(PaintFallbackReason::PropertyStateChanged);
                    }
                    if compositor_ids
                        .get(&key.compositor_id())
                        .is_some_and(|previous| *previous != key)
                    {
                        return Err(PaintFallbackReason::KeyCollision);
                    }
                    if let Some(index) = order.iter().position(|candidate| *candidate == key) {
                        order.remove(index);
                    }
                    if item.index > order.len() {
                        return Err(PaintFallbackReason::OrderChanged);
                    }
                    order.insert(item.index, key);
                    compositor_ids.insert(key.compositor_id(), key);
                    let (origin, mut surface) = tile(item.local_bounds)?;
                    prepared_pixels = prepared_pixels
                        .saturating_add(surface.width as usize * surface.height as usize);
                    if prepared_pixels > MAX_RETAINED_PIXELS {
                        return Err(PaintFallbackReason::DiffBudgetExceeded);
                    }
                    if let Some(index) = self.ids.get(&key) {
                        let old = &self.items[*index];
                        final_pixels -= old.surface.width as usize * old.surface.height as usize;
                    }
                    final_pixels = final_pixels
                        .saturating_add(surface.width as usize * surface.height as usize);
                    if final_pixels > MAX_RETAINED_PIXELS {
                        return Err(PaintFallbackReason::DiffBudgetExceeded);
                    }
                    if surface.width != 0 && surface.height != 0 {
                        let command = match item.kind {
                            PaintItemKind::Decorations => PaintCommand::PaintDecorations(
                                box_,
                                BackgroundPlan::from_style(&box_.style),
                            ),
                            PaintItemKind::Content => PaintCommand::PaintContent(box_),
                            PaintItemKind::Control => unreachable!(),
                        };
                        if !paint::raster_raw_item(&command, &mut surface, origin, text, images) {
                            return Err(PaintFallbackReason::UnseparableGroup);
                        }
                    }
                    prepared.push(Prepared::Upsert(OwnedItem {
                        item: *item,
                        origin,
                        surface,
                        resources: resources_for_box(item.kind, box_, key),
                    }));
                }
            }
        }
        let old_resources = self.resource_set();
        let mut changes = Vec::new();
        let mut added_keys = Vec::new();
        let mut removed_keys = Vec::new();
        for entry in prepared {
            match entry {
                Prepared::Remove(key) => {
                    self.items.remove(self.ids[&key]);
                    removed_keys.push(key);
                }
                Prepared::Upsert(owned) => {
                    let key = owned.item.key.unwrap();
                    let old_bounds = if let Some(index) = self.ids.get(&key) {
                        let previous = self.items.remove(*index);
                        previous.item.local_bounds
                    } else {
                        added_keys.push(key);
                        PaintBounds {
                            x: owned.item.local_bounds.x,
                            y: owned.item.local_bounds.y,
                            width: 0.0,
                            height: 0.0,
                        }
                    };
                    let index = owned.item.index.min(self.items.len());
                    let new_bounds = owned.item.local_bounds;
                    self.items.insert(index, owned);
                    changes.push(PaintItemChange {
                        key,
                        compositor_id: key.compositor_id(),
                        item_index: index,
                        old_bounds: damage_bounds(old_bounds).unwrap_or(old_bounds),
                        new_bounds: damage_bounds(new_bounds).unwrap_or(new_bounds),
                    });
                }
            }
            self.reindex();
        }
        debug_assert_eq!(
            self.items
                .iter()
                .map(|owned| owned.item.key.unwrap())
                .collect::<Vec<_>>(),
            order
        );
        for (index, owned) in self.items.iter_mut().enumerate() {
            owned.item.index = index;
        }
        for change in &mut changes {
            change.item_index = self.ids[&change.key];
        }
        let mut indexes: Vec<_> = changes.iter().map(|change| change.item_index).collect();
        indexes.sort_unstable();
        let mut changed_ranges: Vec<Range<usize>> = Vec::new();
        for index in indexes {
            if let Some(range) = changed_ranges.last_mut() {
                if range.end == index {
                    range.end += 1;
                    continue;
                }
            }
            changed_ranges.push(index..index + 1);
        }
        let new_resources = self.resource_set();
        Ok(RetainedPaintDelta {
            changes,
            changed_ranges,
            added_keys,
            removed_keys,
            retired_resources: old_resources.difference(&new_resources).cloned().collect(),
        })
    }

    fn valid_properties(&self, item: &PaintItem) -> bool {
        let (t, c, e, s) = self.property_counts;
        (item.properties.transform as usize) < t
            && (item.properties.clip as usize) < c
            && (item.properties.effect as usize) < e
            && (item.properties.scroll as usize) < s
    }

    fn reindex(&mut self) {
        self.ids.clear();
        for (index, owned) in self.items.iter().enumerate() {
            self.ids.insert(owned.item.key.unwrap(), index);
        }
    }

    fn resource_set(&self) -> BTreeSet<PaintResourceKey> {
        self.items
            .iter()
            .flat_map(|owned| owned.resources.iter().cloned())
            .collect()
    }
}

enum Prepared {
    Remove(StablePaintItemKey),
    Upsert(OwnedItem),
}

fn tile(bounds: PaintBounds) -> Result<((i32, i32), Surface), PaintFallbackReason> {
    if ![bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter()
        .all(f32::is_finite)
        || bounds.width < 0.0
        || bounds.height < 0.0
    {
        return Err(PaintFallbackReason::UnsafeBounds);
    }
    let left = bounds.x.floor();
    let top = bounds.y.floor();
    let right = (bounds.x + bounds.width).ceil();
    let bottom = (bounds.y + bounds.height).ceil();
    if ![left, top, right, bottom]
        .into_iter()
        .all(|edge| edge >= i32::MIN as f32 && edge <= i32::MAX as f32)
    {
        return Err(PaintFallbackReason::UnsafeBounds);
    }
    let width = (right - left).max(0.0) as u32;
    let height = (bottom - top).max(0.0) as u32;
    if (width as usize)
        .checked_mul(height as usize)
        .is_none_or(|pixels| pixels > MAX_TILE_PIXELS)
    {
        return Err(PaintFallbackReason::DiffBudgetExceeded);
    }
    Ok(((left as i32, top as i32), Surface::new(width, height)))
}

fn resources_for_command(
    view: &DisplayListView<'_>,
    index: usize,
) -> Result<BTreeSet<PaintResourceKey>, PaintFallbackReason> {
    let command = &view.commands[view.item_commands[index]];
    let key = view.items[index]
        .key
        .ok_or(PaintFallbackReason::AmbiguousGeneration)?;
    Ok(match command {
        PaintCommand::PaintDecorations(box_, _) => {
            resources_for_box(PaintItemKind::Decorations, box_, key)
        }
        PaintCommand::PaintContent(box_) => resources_for_box(PaintItemKind::Content, box_, key),
        PaintCommand::PaintControl(_) => return Err(PaintFallbackReason::UnseparableGroup),
        _ => return Err(PaintFallbackReason::RasterFailed),
    })
}

fn resources_for_box(
    kind: PaintItemKind,
    box_: &LayoutBox,
    key: StablePaintItemKey,
) -> BTreeSet<PaintResourceKey> {
    let mut resources = BTreeSet::new();
    if kind == PaintItemKind::Decorations {
        for layer in BackgroundPlan::from_style(&box_.style).layers {
            if let Some(url)=layer.url {
                resources.insert(match layer.source_base {
                    Some(base)=>PaintResourceKey::BackgroundUrlWithBase {url,source_base:base},
                    None=>PaintResourceKey::BackgroundUrl(url),
                });
            }
        }
        if let Some(plan)=crate::border_image::BorderImagePlan::from_style(&box_.style) {
            if let Some(url)=plan.url {resources.insert(PaintResourceKey::BorderImageUrl{url,source_base:plan.source_base});}
        }
        if box_.replaced_size.is_some() || box_.svg_root.is_some() {
            resources.insert(PaintResourceKey::ReplacedNode(key.node));
        }
    }
    resources
}

#[cfg(test)]
mod tests {
    use super::*;
    use amazing_dom::NodeIdx;
    use amazing_layout::{Display, Flow, Rect};
    use amazing_text::FallbackTextSystem;

    fn box_with(node: u32, x: f32, color: &str) -> LayoutBox {
        let mut style = amazing_css::ComputedStyle::default();
        style.set("background-color", color);
        let mut box_ = LayoutBox::new(Some(NodeIdx(node)), Display::Block, style, 16.0, 19.2);
        box_.content_box = Rect {
            x,
            y: 0.0,
            w: 12.0,
            h: 12.0,
        };
        box_.border_box = box_.content_box;
        box_
    }

    fn root(children: Vec<LayoutBox>) -> LayoutBox {
        let mut root = box_with(1, 0.0, "transparent");
        root.content_box = Rect {
            x: 0.0,
            y: 0.0,
            w: 64.0,
            h: 32.0,
        };
        root.border_box = root.content_box;
        root.flow = Flow::Blocks(children);
        root
    }

    fn view<'a>(root: &'a LayoutBox, text: &'a FallbackTextSystem) -> DisplayListView<'a> {
        DisplayListView::with_node_generations(
            root,
            64,
            32,
            text,
            &crate::paint::NoImages,
            &crate::controls::NoControls,
            &|_| Some(4),
        )
    }

    fn flattened(scene: &RetainedPaintView) -> Surface {
        let mut output = Surface::new(64, 32);
        output.clear([255; 4]);
        for item in scene.items() {
            let raster = scene.raster(item.key.unwrap()).unwrap();
            for y in 0..raster.surface.height {
                for x in 0..raster.surface.width {
                    let (dx, dy) = (raster.origin.0 + x as i32, raster.origin.1 + y as i32);
                    if dx < 0 || dy < 0 {
                        continue;
                    }
                    let Some(src) = raster.surface.get_pixel(x, y) else {
                        continue;
                    };
                    let Some(dst) = output.get_pixel(dx as u32, dy as u32) else {
                        continue;
                    };
                    output.set_pixel(
                        dx as u32,
                        dy as u32,
                        crate::blend_source_over(src, dst, 255),
                    );
                }
            }
        }
        output
    }

    #[test]
    fn keyed_damage_rasterizes_only_changed_chunk_and_keeps_other_pixels_owned() {
        let old_root = root(vec![box_with(2, 15.0, "red"), box_with(3, 40.0, "blue")]);
        let new_root = root(vec![box_with(2, 17.0, "green"), box_with(3, 40.0, "blue")]);
        let text = FallbackTextSystem::new();
        let old_view = view(&old_root, &text);
        let next_view = view(&new_root, &text);
        let mut scene = RetainedPaintView::from_view(&old_view).unwrap();
        let changed = next_view
            .items()
            .iter()
            .find(|item| {
                item.key.is_some_and(|key| key.node.node == NodeIdx(2))
                    && item.kind == PaintItemKind::Decorations
            })
            .unwrap();
        let unchanged = next_view
            .items()
            .iter()
            .find(|item| {
                item.key.is_some_and(|key| key.node.node == NodeIdx(3))
                    && item.kind == PaintItemKind::Decorations
            })
            .unwrap();
        let unchanged_pointer = scene
            .raster(unchanged.key.unwrap())
            .unwrap()
            .surface
            .pixels
            .as_ptr();
        let Flow::Blocks(children) = &new_root.flow else {
            unreachable!()
        };
        let delta = scene
            .apply_damage(
                &[PaintDamage::Upsert {
                    item: *changed,
                    box_: &children[0],
                }],
                &text,
                &crate::paint::NoImages,
            )
            .unwrap();
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changed_ranges, [changed.index..changed.index + 1]);
        assert!(delta.added_keys.is_empty());
        assert!(delta.removed_keys.is_empty());
        assert_eq!(
            scene
                .raster(unchanged.key.unwrap())
                .unwrap()
                .surface
                .pixels
                .as_ptr(),
            unchanged_pointer
        );
        let actual = scene.raster(changed.key.unwrap()).unwrap();
        let mut oracle = Surface::new(actual.surface.width, actual.surface.height);
        next_view
            .raster_raw_local(changed.index..changed.index + 1, &mut oracle, actual.origin)
            .unwrap();
        assert_eq!(*actual.surface, oracle);
        let mut full = Surface::new(64, 32);
        full.clear([255; 4]);
        assert!(next_view.raster_range(
            0..next_view.items().len(),
            &mut full,
            (0, 0),
            PaintBounds::viewport(64, 32)
        ));
        assert_eq!(flattened(&scene), full);

        let before = actual.surface.pixels.clone();
        let mut invalid = *changed;
        invalid.properties.transform = 99;
        assert!(matches!(
            scene.apply_damage(
                &[PaintDamage::Upsert {
                    item: invalid,
                    box_: &children[0]
                }],
                &text,
                &crate::paint::NoImages
            ),
            Err(PaintFallbackReason::PropertyStateChanged)
        ));
        assert_eq!(
            scene.raster(changed.key.unwrap()).unwrap().surface.pixels,
            before
        );
    }

    #[test]
    fn removed_item_retires_only_unreferenced_resource() {
        let mut first = box_with(2, 0.0, "transparent");
        first.style.set("background-image", "url(old)");
        let mut second = box_with(3, 20.0, "transparent");
        second.style.set("background-image", "url(shared)");
        let mut third = box_with(4, 40.0, "transparent");
        third.style.set("background-image", "url(shared)");
        let root = root(vec![first, second, third]);
        let text = FallbackTextSystem::new();
        let view = view(&root, &text);
        let mut scene = RetainedPaintView::from_view(&view).unwrap();
        let removed: Vec<_> = view
            .items()
            .iter()
            .filter(|item| {
                item.key
                    .is_some_and(|key| matches!(key.node.node, NodeIdx(2) | NodeIdx(3)))
            })
            .map(|item| PaintDamage::Remove(item.key.unwrap()))
            .collect();
        let delta = scene
            .apply_damage(&removed, &text, &crate::paint::NoImages)
            .unwrap();
        assert_eq!(delta.removed_keys.len(), 4);
        assert_eq!(
            delta.retired_resources,
            [PaintResourceKey::BackgroundUrl("old".into())]
        );
        assert!(scene.items().all(|item| item
            .key
            .is_none_or(|key| key.node.node != NodeIdx(2) && key.node.node != NodeIdx(3))));
    }

    #[test]
    fn new_item_is_inserted_at_its_stable_order_position() {
        let old_root = root(vec![box_with(2, 0.0, "red")]);
        let new_root = root(vec![box_with(2, 0.0, "red"), box_with(3, 20.0, "blue")]);
        let text = FallbackTextSystem::new();
        let old_view = view(&old_root, &text);
        let next_view = view(&new_root, &text);
        let mut scene = RetainedPaintView::from_view(&old_view).unwrap();
        let added = *next_view
            .items()
            .iter()
            .find(|item| {
                item.key.is_some_and(|key| key.node.node == NodeIdx(3))
                    && item.kind == PaintItemKind::Decorations
            })
            .unwrap();
        let Flow::Blocks(children) = &new_root.flow else {
            unreachable!()
        };
        let delta = scene
            .apply_damage(
                &[PaintDamage::Upsert {
                    item: added,
                    box_: &children[1],
                }],
                &text,
                &crate::paint::NoImages,
            )
            .unwrap();
        assert_eq!(delta.added_keys, [added.key.unwrap()]);
        assert_eq!(scene.items().nth(added.index).unwrap().key, added.key);
        assert!(scene
            .raster(added.key.unwrap())
            .unwrap()
            .surface
            .pixels
            .iter()
            .any(|byte| *byte != 0));
    }
}
