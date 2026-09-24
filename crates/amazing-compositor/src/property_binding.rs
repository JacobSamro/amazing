//! Stable DOM identity to retained property-node bindings for one scene.
//! A scene rebuild installs a new table; reused DOM arena slots cannot inherit
//! a prior binding because the node generation is part of the key.

use super::{Compositor, EffectId, Matrix4, ScrollId, TransformId};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundProperty {
    Transform,
    Opacity,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodePropertyKey {
    pub node: u32,
    pub generation: u64,
    pub property: BoundProperty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundPropertyNode {
    Transform(TransformId),
    Opacity(EffectId),
    Scroll(ScrollId),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoundPropertyValue {
    Transform(Matrix4),
    Opacity(f32),
    Scroll((f32, f32)),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundPropertyChange {
    pub key: NodePropertyKey,
    pub value: BoundPropertyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyBindingError {
    DuplicateKey,
    AliasedNode,
    InvalidNode,
    MappingChanged,
    UnknownKey,
    WrongProperty,
    NonFinite,
}

impl Compositor {
    /// Install the paint view's owner mapping. A live scene cannot silently
    /// retarget a DOM node to a different property node.
    pub fn bind_property_nodes(
        &mut self,
        bindings: &[(NodePropertyKey, BoundPropertyNode)],
    ) -> Result<(), PropertyBindingError> {
        let mut mapping = HashMap::with_capacity(bindings.len());
        let mut targets = HashSet::with_capacity(bindings.len());
        for &(key, target) in bindings {
            let valid = match (key.property, target) {
                (BoundProperty::Transform, BoundPropertyNode::Transform(id)) => {
                    id.0 != 0 && self.trees.transform(id).is_some()
                }
                (BoundProperty::Opacity, BoundPropertyNode::Opacity(id)) => {
                    id.0 != 0 && self.trees.effect(id).is_some()
                }
                (BoundProperty::Scroll, BoundPropertyNode::Scroll(id)) => {
                    id.0 != 0 && self.trees.scroll(id).is_some()
                }
                _ => return Err(PropertyBindingError::WrongProperty),
            };
            if !valid {
                return Err(PropertyBindingError::InvalidNode);
            }
            if mapping.insert(key, target).is_some() {
                return Err(PropertyBindingError::DuplicateKey);
            }
            if !targets.insert(target) {
                return Err(PropertyBindingError::AliasedNode);
            }
        }
        if !self.property_bindings.is_empty() && self.property_bindings != mapping {
            return Err(PropertyBindingError::MappingChanged);
        }
        self.property_bindings = mapping;
        Ok(())
    }

    /// Validate the entire batch before changing any tree. Missing or stale
    /// bindings require the caller to take the normal style/layout/paint path.
    pub fn apply_bound_property_changes(
        &mut self,
        changes: &[BoundPropertyChange],
    ) -> Result<(), PropertyBindingError> {
        let mut seen = HashSet::with_capacity(changes.len());
        for change in changes {
            if !seen.insert(change.key) {
                return Err(PropertyBindingError::DuplicateKey);
            }
            let target = self.property_bindings.get(&change.key)
                .ok_or(PropertyBindingError::UnknownKey)?;
            match (target, change.value) {
                (BoundPropertyNode::Transform(_), BoundPropertyValue::Transform(matrix))
                    if matrix.0.iter().flatten().all(|value| value.is_finite()) => {}
                (BoundPropertyNode::Opacity(_), BoundPropertyValue::Opacity(value))
                    if value.is_finite() => {}
                (BoundPropertyNode::Scroll(_), BoundPropertyValue::Scroll((x, y)))
                    if x.is_finite() && y.is_finite() => {}
                (BoundPropertyNode::Transform(_), BoundPropertyValue::Transform(_))
                | (BoundPropertyNode::Opacity(_), BoundPropertyValue::Opacity(_))
                | (BoundPropertyNode::Scroll(_), BoundPropertyValue::Scroll(_)) => {
                    return Err(PropertyBindingError::NonFinite);
                }
                _ => return Err(PropertyBindingError::WrongProperty),
            }
        }
        for change in changes {
            match (self.property_bindings[&change.key], change.value) {
                (BoundPropertyNode::Transform(id), BoundPropertyValue::Transform(matrix)) => {
                    self.set_transform(id, matrix).expect("validated transform binding");
                }
                (BoundPropertyNode::Opacity(id), BoundPropertyValue::Opacity(value)) => {
                    self.set_opacity(id, value).expect("validated opacity binding");
                }
                (BoundPropertyNode::Scroll(id), BoundPropertyValue::Scroll(offset)) => {
                    self.set_scroll(id, offset).expect("validated scroll binding");
                }
                _ => unreachable!("validated property kind"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlendMode, CompositingReasons, PaintChunk, PaintSource, PropertyTreeState,
        PropertyTrees, Rect, ReferenceCpuPresenter, Presenter, SceneError};
    use amazing_paint::Surface;
    use std::{cell::Cell, ops::Range};

    struct CountingSource<'a>(&'a Cell<usize>);
    impl PaintSource for CountingSource<'_> {
        fn item_count(&self) -> usize { 1 }
        fn rasterize(&self, _: Range<usize>, _: Rect, output: &mut Surface) {
            self.0.set(self.0.get() + 1);
            output.clear([40, 80, 120, 255]);
        }
    }

    #[test]
    fn bound_opacity_recomposes_without_paint_or_tile_upload() {
        let mut trees = PropertyTrees::new();
        let effect = trees.add_effect(EffectId(0), 1., BlendMode::Normal, None).unwrap();
        let state = PropertyTreeState { effect, ..Default::default() };
        let mut compositor = Compositor::new(trees, vec![PaintChunk {
            items: 0..1,
            state,
            bounds: Rect::new(0., 0., 64., 64.),
            reasons: CompositingReasons::WILL_CHANGE,
        }], 1).unwrap();
        let key = NodePropertyKey { node: 7, generation: 42, property: BoundProperty::Opacity };
        compositor.bind_property_nodes(&[(key, BoundPropertyNode::Opacity(effect))]).unwrap();
        let raster_calls = Cell::new(0);
        let first = compositor.build_frame(&CountingSource(&raster_calls), 64, 64).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        let before = presenter.present(&first).unwrap();
        assert_eq!(raster_calls.get(), 1);
        compositor.apply_bound_property_changes(&[BoundPropertyChange {
            key, value: BoundPropertyValue::Opacity(0.5),
        }]).unwrap();
        let second = compositor.build_cached_frame(64, 64).unwrap();
        let after = presenter.present(&second).unwrap();
        assert_eq!(raster_calls.get(), 1);
        assert!(second.resources.uploads.is_empty());
        assert_eq!(compositor.dirty_tiles(), 0);
        assert_ne!(before.pixels, after.pixels);
        assert_eq!(after.get_pixel(0, 0), Some([40, 80, 120, 128]));

        let mut full_trees = PropertyTrees::new();
        let full_effect = full_trees.add_effect(EffectId(0), 0.5, BlendMode::Normal, None).unwrap();
        let mut full = Compositor::new(full_trees, vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState { effect: full_effect, ..Default::default() },
            bounds: Rect::new(0., 0., 64., 64.),
            reasons: CompositingReasons::WILL_CHANGE,
        }], 1).unwrap();
        let freshly_painted = full.build_frame(&CountingSource(&raster_calls), 64, 64).unwrap();
        let expected = ReferenceCpuPresenter::default().present(&freshly_painted).unwrap();
        assert_eq!(after.pixels, expected.pixels, "cached property frame versus full render");
    }

    #[test]
    fn stale_or_invalid_batches_do_not_mutate_tree() {
        let mut trees = PropertyTrees::new();
        let effect = trees.add_effect(EffectId(0), 1., BlendMode::Normal, None).unwrap();
        let mut compositor = Compositor::new(trees, Vec::new(), 0).unwrap();
        let key = NodePropertyKey { node: 7, generation: 42, property: BoundProperty::Opacity };
        compositor.bind_property_nodes(&[(key, BoundPropertyNode::Opacity(effect))]).unwrap();
        let stale = NodePropertyKey { generation: 43, ..key };
        assert_eq!(compositor.apply_bound_property_changes(&[
            BoundPropertyChange { key, value: BoundPropertyValue::Opacity(0.5) },
            BoundPropertyChange { key: stale, value: BoundPropertyValue::Opacity(0.25) },
        ]), Err(PropertyBindingError::UnknownKey));
        assert_eq!(compositor.trees().effect(effect).unwrap().opacity, 1.);
        assert_eq!(compositor.apply_bound_property_changes(&[
            BoundPropertyChange { key, value: BoundPropertyValue::Opacity(f32::NAN) },
        ]), Err(PropertyBindingError::NonFinite));
        assert_eq!(compositor.trees().effect(effect).unwrap().opacity, 1.);
    }

    #[test]
    fn newly_visible_uncached_tile_requires_paint() {
        let mut trees = PropertyTrees::new();
        let transform = trees.add_transform(TransformId(0), Matrix4::translation(300., 0., 0.),
            (0., 0., 0.), false).unwrap();
        let mut compositor = Compositor::new(trees, vec![PaintChunk {
            items: 0..1,
            state: PropertyTreeState { transform, ..Default::default() },
            bounds: Rect::new(0., 0., 64., 64.),
            reasons: CompositingReasons::WILL_CHANGE,
        }], 1).unwrap();
        let key = NodePropertyKey { node: 7, generation: 42, property: BoundProperty::Transform };
        compositor.bind_property_nodes(&[(key, BoundPropertyNode::Transform(transform))]).unwrap();
        let raster_calls = Cell::new(0);
        compositor.build_frame(&CountingSource(&raster_calls), 64, 64).unwrap();
        assert_eq!(raster_calls.get(), 0);
        compositor.apply_bound_property_changes(&[BoundPropertyChange {
            key, value: BoundPropertyValue::Transform(Matrix4::IDENTITY),
        }]).unwrap();
        assert!(matches!(compositor.build_cached_frame(64, 64),
            Err(SceneError::MissingCachedTile)));
        assert_eq!(raster_calls.get(), 0);
    }
}
