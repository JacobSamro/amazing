//! Conservative old/new display-list comparison at one frozen DOM epoch.
//!
//! This first seam compares bounded item rasters from two complete views.
//! Callers retain the new full view as the oracle and presentation fallback.
//! Upstream geometry/style deltas may later restrict which items are compared.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use super::{DisplayListView, PaintBounds, PaintCommand, PaintNodeKey, StablePaintItemKey};
use crate::surface::Surface;

const MAX_DIFF_ITEMS: usize = 512;
const MAX_DIFF_PIXELS: usize = 1_048_576;

/// Resource identities that must be released when absent from the new view.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaintResourceKey {
    BackgroundUrl(String),
    BorderImageUrl { url: String, source_base: Option<String> },
    GeneratedUrl(String),
    ReplacedNode(PaintNodeKey),
}

/// Fields map directly to the compositor's `PaintItemChange` after converting
/// `PaintBounds` to its rectangle type and wrapping `compositor_id` in its key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintItemChange {
    pub key: StablePaintItemKey,
    pub compositor_id: u64,
    pub item_index: usize,
    pub old_bounds: PaintBounds,
    pub new_bounds: PaintBounds,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaintViewDelta {
    pub changes: Vec<PaintItemChange>,
    pub changed_ranges: Vec<Range<usize>>,
    pub retired_resources: Vec<PaintResourceKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintFallbackReason {
    AmbiguousGeneration,
    AmbiguousPropertyOwner,
    DuplicateKey,
    KeyCollision,
    OrderChanged,
    PropertyStateChanged,
    UnseparableGroup,
    UnsafeBounds,
    RasterFailed,
    ViewportChanged,
    DiffBudgetExceeded,
}

/// The caller must present the new full view on `RequiresFullRebuild`.
#[derive(Debug, Clone, PartialEq)]
pub enum PaintViewUpdate {
    Delta(PaintViewDelta),
    RequiresFullRebuild {
        reason: PaintFallbackReason,
        retired_resources: Vec<PaintResourceKey>,
    },
}

impl DisplayListView<'_> {
    /// Compare two full views for one mutation epoch. This is deliberately
    /// bounded and refuses ambiguous identity, order, property-tree, group or
    /// ink changes that retained chunks cannot yet express safely.
    pub fn update(&self, next: &DisplayListView<'_>) -> PaintViewUpdate {
        let bounded_items = self.items.len() <= MAX_DIFF_ITEMS && next.items.len() <= MAX_DIFF_ITEMS;
        let old_resources = bounded_items.then(|| self.resources()).flatten();
        let new_resources = bounded_items.then(|| next.resources()).flatten();
        let retired_resources = if bounded_items {
            old_resources.as_ref().zip(new_resources.as_ref())
                .map(|(old, new)| old.difference(new).cloned().collect())
                .unwrap_or_default()
        } else {
            Vec::new() // full scene rebuild releases the whole old resource set
        };
        let fallback = |reason| PaintViewUpdate::RequiresFullRebuild {
            reason,
            retired_resources: retired_resources.clone(),
        };
        if !bounded_items {
            return fallback(PaintFallbackReason::DiffBudgetExceeded);
        }
        if self.ambiguous_generation || next.ambiguous_generation
            || old_resources.is_none() || new_resources.is_none()
        {
            return fallback(PaintFallbackReason::AmbiguousGeneration);
        }
        if self.property_owner_mapping().is_none() || next.property_owner_mapping().is_none() {
            return fallback(PaintFallbackReason::AmbiguousPropertyOwner);
        }
        if self.width != next.width || self.height != next.height {
            return fallback(PaintFallbackReason::ViewportChanged);
        }
        let Some(pixels) = (self.width as usize).checked_mul(self.height as usize) else {
            return fallback(PaintFallbackReason::DiffBudgetExceeded);
        };
        if pixels > MAX_DIFF_PIXELS || self.items.len() > MAX_DIFF_ITEMS
            || next.items.len() > MAX_DIFF_ITEMS
        {
            return fallback(PaintFallbackReason::DiffBudgetExceeded);
        }
        let mut old_ids = HashMap::new();
        let mut new_ids = HashMap::new();
        for (items, ids) in [(&self.items, &mut old_ids), (&next.items, &mut new_ids)] {
            for item in items {
                let Some(key) = item.key else {
                    return fallback(PaintFallbackReason::AmbiguousGeneration);
                };
                let id = key.compositor_id();
                if let Some(previous) = ids.insert(id, key) {
                    return fallback(if previous == key {
                        PaintFallbackReason::DuplicateKey
                    } else {
                        PaintFallbackReason::KeyCollision
                    });
                }
            }
        }
        if self.items.len() != next.items.len()
            || self.items.iter().zip(&next.items).any(|(old, new)| old.key != new.key)
        {
            return fallback(PaintFallbackReason::OrderChanged);
        }
        if self.transforms != next.transforms || self.clips != next.clips
            || self.effects != next.effects || self.scrolls != next.scrolls
        {
            return fallback(PaintFallbackReason::PropertyStateChanged);
        }

        let mut changes = Vec::new();
        let mut changed_ranges: Vec<Range<usize>> = Vec::new();
        for (old, new) in self.items.iter().zip(&next.items) {
            if old.properties != new.properties || old.hints != new.hints
                || old.sticky != new.sticky || old.kind != new.kind
            {
                return fallback(PaintFallbackReason::PropertyStateChanged);
            }
            let Some(old_pixels) = self.item_pixels(old.index) else {
                return fallback(PaintFallbackReason::RasterFailed);
            };
            let Some(new_pixels) = next.item_pixels(new.index) else {
                return fallback(PaintFallbackReason::RasterFailed);
            };
            if old.local_bounds.width == 0.0 && old.local_bounds.height == 0.0
                && new.local_bounds.width == 0.0 && new.local_bounds.height == 0.0
                && old_pixels == new_pixels && old_pixels.iter().all(|pixel| *pixel == 0)
            {
                continue;
            }
            let Some(old_bounds) = damage_bounds(old.local_bounds) else {
                return fallback(PaintFallbackReason::UnsafeBounds);
            };
            let Some(new_bounds) = damage_bounds(new.local_bounds) else {
                return fallback(PaintFallbackReason::UnsafeBounds);
            };
            if old_bounds == new_bounds && old_pixels == new_pixels {
                continue;
            }
            if old.properties.effect != 0 || new.properties.effect != 0 {
                return fallback(PaintFallbackReason::UnseparableGroup);
            }
            let index = new.index;
            changes.push(PaintItemChange {
                key: new.key.expect("validated above"),
                compositor_id: new.key.expect("validated above").compositor_id(),
                item_index: index,
                old_bounds,
                new_bounds,
            });
            if let Some(range) = changed_ranges.last_mut() {
                if range.end == index {
                    range.end += 1;
                    continue;
                }
            }
            changed_ranges.push(index..index + 1);
        }
        PaintViewUpdate::Delta(PaintViewDelta {
            changes,
            changed_ranges,
            retired_resources,
        })
    }

    fn item_pixels(&self, index: usize) -> Option<Vec<u8>> {
        let mut surface = Surface::new(self.width, self.height);
        self.raster_range(
            index..index + 1,
            &mut surface,
            (0, 0),
            PaintBounds::viewport(self.width, self.height),
        ).then_some(surface.pixels)
    }

    fn resources(&self) -> Option<BTreeSet<PaintResourceKey>> {
        let mut resources = BTreeSet::new();
        for item in &self.items {
            let command = &self.commands[self.item_commands[item.index]];
            match command {
                PaintCommand::PaintDecorations(box_, plan) => {
                    for url in plan.layers.iter().filter_map(|layer| layer.url.as_ref()) {
                        resources.insert(PaintResourceKey::BackgroundUrl(url.clone()));
                    }
                    if let Some(plan)=crate::border_image::BorderImagePlan::from_style(&box_.style) {
                        if let Some(url)=plan.url {resources.insert(PaintResourceKey::BorderImageUrl{url,source_base:plan.source_base});}
                    }
                    if let Some(key) = item.key {
                        if box_.replaced_size.is_some() || box_.svg_root.is_some() {
                            resources.insert(PaintResourceKey::ReplacedNode(key.node));
                        }
                    }
                }
                PaintCommand::PaintContent(box_) => {
                    for fragment in box_.styled_replaced_fragments() {
                        if let Some(url) = fragment.generated_url {
                            resources.insert(PaintResourceKey::GeneratedUrl(url));
                        } else {
                            resources.insert(PaintResourceKey::ReplacedNode(PaintNodeKey {
                                node: fragment.node,
                                generation: *self.node_generations.get(&fragment.node)?,
                            }));
                        }
                    }
                }
                PaintCommand::PaintControl(_) => {
                    if let Some(key) = item.key {
                        resources.insert(PaintResourceKey::ReplacedNode(key.node));
                    }
                }
                _ => {}
            }
        }
        Some(resources)
    }
}

pub(super) fn damage_bounds(bounds: PaintBounds) -> Option<PaintBounds> {
    if ![bounds.x, bounds.y, bounds.width, bounds.height]
        .into_iter().all(f32::is_finite)
        || bounds.width <= 0.0 || bounds.height <= 0.0
    {
        return None;
    }
    let expanded = PaintBounds {
        x: bounds.x - 1.0,
        y: bounds.y - 1.0,
        width: bounds.width + 2.0,
        height: bounds.height + 2.0,
    };
    [expanded.x, expanded.y, expanded.width, expanded.height]
        .into_iter().all(f32::is_finite).then_some(expanded)
}
