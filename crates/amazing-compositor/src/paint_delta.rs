//! A conservative bridge from stable paint item keys to retained tile damage.
//! Changes that alter item order or escape a chunk's conservative bounds need
//! a fresh scene until the painter exposes retained chunks and resource deltas.

use super::{tile_keys, Compositor, Rect, TileKey};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaintItemKey(pub u64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintItemChange {
    pub key: PaintItemKey,
    /// Index in the new display list. A changed index requires a scene rebuild.
    pub item_index: usize,
    /// Conservative local ink bounds before and after the paint update.
    pub old_bounds: Rect,
    pub new_bounds: Rect,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaintDelta {
    pub changes: Vec<PaintItemChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintDeltaError {
    MissingKeys,
    DuplicateKey,
    UnknownKey,
    /// Item count/order, chunk bounds, or layer structure may have changed.
    RequiresFullRebuild,
    InvalidBounds,
}

fn within(outer: Rect, inner: Rect) -> bool {
    !inner.empty()
        && inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.w <= outer.x + outer.w
        && inner.y + inner.h <= outer.y + outer.h
}

impl Compositor {
    /// Registers the painter's stable keys in display-list order. Bind on a
    /// fresh scene; a different mapping after raster needs a full rebuild.
    pub fn bind_paint_item_keys(&mut self, keys: &[PaintItemKey]) -> Result<(), PaintDeltaError> {
        if keys.len() != self.item_count {
            return Err(PaintDeltaError::RequiresFullRebuild);
        }
        let mut mapping = HashMap::with_capacity(keys.len());
        for (index, key) in keys.iter().copied().enumerate() {
            if mapping.insert(key, index).is_some() {
                return Err(PaintDeltaError::DuplicateKey);
            }
        }
        if !self.tiles.is_empty() && self.paint_item_keys.as_ref() != Some(&mapping) {
            return Err(PaintDeltaError::RequiresFullRebuild);
        }
        self.paint_item_keys = Some(mapping);
        Ok(())
    }

    /// Maps changed stable keys and old/new local ink bounds to cached tiles.
    /// Validation is atomic: an unsupported delta leaves all tiles untouched.
    /// The caller must use a source with the updated item pixels on next frame.
    pub fn apply_paint_delta(&mut self, delta: &PaintDelta) -> Result<usize, PaintDeltaError> {
        let keys = self
            .paint_item_keys
            .as_ref()
            .ok_or(PaintDeltaError::MissingKeys)?;
        let mut seen = HashSet::new();
        let mut affected = HashMap::<TileKey, Rect>::new();
        for change in &delta.changes {
            if !seen.insert(change.key) {
                return Err(PaintDeltaError::DuplicateKey);
            }
            let index = *keys.get(&change.key).ok_or(PaintDeltaError::UnknownKey)?;
            if index != change.item_index {
                return Err(PaintDeltaError::RequiresFullRebuild);
            }
            if change.old_bounds.empty() || change.new_bounds.empty() {
                return Err(PaintDeltaError::InvalidBounds);
            }
            let chunk_index = self
                .chunks
                .iter()
                .position(|chunk| chunk.items.contains(&index))
                .ok_or(PaintDeltaError::RequiresFullRebuild)?;
            let chunk = &self.chunks[chunk_index];
            if !within(chunk.bounds, change.old_bounds) || !within(chunk.bounds, change.new_bounds)
            {
                return Err(PaintDeltaError::RequiresFullRebuild);
            }
            let damage = change.old_bounds.union(change.new_bounds);
            for (layer_index, layer) in self.layers.iter().enumerate() {
                if layer.chunks.contains(&chunk_index) {
                    for key in tile_keys(layer_index, damage) {
                        affected
                            .entry(key)
                            .and_modify(|rect| *rect = rect.union(damage))
                            .or_insert(damage);
                    }
                }
            }
        }
        let mut dirtied = 0;
        if !affected.is_empty() {
            self.raster_epoch = self.raster_epoch.wrapping_add(1);
        }
        for (key, damage) in affected {
            if let Some(tile) = self.tiles.get_mut(&key) {
                if !tile.dirty {
                    tile.damage = Some(damage);
                } else if let Some(previous) = tile.damage {
                    tile.damage = Some(previous.union(damage));
                }
                tile.dirty = true;
                dirtied += 1;
            }
        }
        Ok(dirtied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BlendMode, CompositingReasons, DrawQuad, EffectId, Matrix4, PaintChunk, PaintSource,
        PropertyTreeState, PropertyTrees, TransformId,
    };
    use amazing_paint::Surface;
    use std::cell::Cell;
    use std::ops::Range;

    struct Source {
        colors: Vec<[u8; 4]>,
        rects: Vec<Rect>,
        paint_calls: Cell<usize>,
    }

    impl PaintSource for Source {
        fn item_count(&self) -> usize {
            self.rects.len()
        }

        fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
            self.paint_calls.set(self.paint_calls.get() + 1);
            for index in items {
                let rect = self.rects[index];
                out.fill_rect(
                    rect.x - tile.x,
                    rect.y - tile.y,
                    rect.w,
                    rect.h,
                    self.colors[index],
                );
            }
        }
    }

    #[test]
    fn stable_item_delta_rerasterizes_only_its_tile_and_matches_full_repaint() {
        let rects = [Rect::new(8., 8., 32., 32.), Rect::new(280., 8., 32., 32.)];
        let chunks = rects
            .iter()
            .enumerate()
            .map(|(index, bounds)| PaintChunk {
                items: index..index + 1,
                state: PropertyTreeState::default(),
                bounds: *bounds,
                reasons: CompositingReasons::default(),
            })
            .collect::<Vec<_>>();
        let mut source = Source {
            colors: vec![[200, 30, 20, 255], [30, 40, 200, 255]],
            rects: rects.to_vec(),
            paint_calls: Cell::new(0),
        };
        let mut compositor = Compositor::new(PropertyTrees::new(), chunks.clone(), 2).unwrap();
        compositor
            .bind_paint_item_keys(&[PaintItemKey(10), PaintItemKey(20)])
            .unwrap();
        compositor.build_frame(&source, 512, 64).unwrap();
        assert_eq!(compositor.cached_tiles(), 2);
        let paint_before = source.paint_calls.get();
        source.colors[1] = [20, 180, 70, 255];
        let delta = PaintDelta {
            changes: vec![PaintItemChange {
                key: PaintItemKey(20),
                item_index: 1,
                old_bounds: rects[1],
                new_bounds: rects[1],
            }],
        };
        assert_eq!(compositor.apply_paint_delta(&delta), Ok(1));
        assert_eq!(compositor.dirty_tiles(), 1);
        assert_eq!(
            compositor
                .tiles
                .get(&TileKey {
                    layer: 0,
                    x: 1,
                    y: 0
                })
                .and_then(|tile| tile.damage),
            Some(rects[1])
        );
        let frame = compositor.build_frame(&source, 512, 64).unwrap();
        assert_eq!(frame.resources.uploads.len(), 1);
        assert_eq!(source.paint_calls.get() - paint_before, 1);
        let mut full = Compositor::new(PropertyTrees::new(), chunks, 2).unwrap();
        assert_eq!(
            compositor.render(&source, 512, 64).unwrap(),
            full.render(&source, 512, 64).unwrap()
        );

        let unsupported = PaintDelta {
            changes: vec![PaintItemChange {
                new_bounds: Rect::new(280., 8., 300., 32.),
                ..delta.changes[0]
            }],
        };
        assert_eq!(
            compositor.apply_paint_delta(&unsupported),
            Err(PaintDeltaError::RequiresFullRebuild)
        );
        assert_eq!(compositor.dirty_tiles(), 0);
    }

    #[test]
    fn property_only_frame_skips_style_layout_paint_and_tile_uploads() {
        let style_calls = Cell::new(0);
        let layout_calls = Cell::new(0);
        let source = Source {
            colors: vec![[100, 80, 200, 255]],
            rects: vec![Rect::new(8., 8., 32., 32.)],
            paint_calls: Cell::new(0),
        };
        let mut trees = PropertyTrees::new();
        let transform = trees
            .add_transform(TransformId(0), Matrix4::IDENTITY, (0., 0., 0.), false)
            .unwrap();
        let effect = trees
            .add_effect(EffectId(0), 1., BlendMode::Normal, None)
            .unwrap();
        let mut compositor = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    transform,
                    effect,
                    ..Default::default()
                },
                bounds: source.rects[0],
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        let first = compositor.build_frame(&source, 64, 64).unwrap();
        assert_eq!(first.resources.uploads.len(), 1);
        let paint_before = source.paint_calls.get();
        compositor
            .set_transform(transform, Matrix4::rotation_z(0.2))
            .unwrap();
        compositor.set_opacity(effect, 0.5).unwrap();
        let frame = compositor.build_frame(&source, 64, 64).unwrap();
        assert_eq!(style_calls.get(), 0);
        assert_eq!(layout_calls.get(), 0);
        assert_eq!(source.paint_calls.get() - paint_before, 0);
        assert_eq!(frame.resources.uploads.len(), 0);
        assert_eq!(compositor.stats().raster_calls, 0);
        assert!(frame
            .root()
            .quads
            .iter()
            .any(|quad| matches!(quad, DrawQuad::Tile(_))));
    }
}
