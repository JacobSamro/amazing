//! Resolved effect values owned by the compositor. CSS parsing, URL loading and
//! reference-box resolution stay with the producer. Values are in raster pixels.
use super::*;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterOperation {
    Blur(f32),
    DropShadow {
        offset: (f32, f32),
        sigma: f32,
        color: [u8; 4],
    },
    Brightness(f32),
    Contrast(f32),
    Grayscale(f32),
    HueRotate(f32),
    Invert(f32),
    Opacity(f32),
    Saturate(f32),
    Sepia(f32),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffectBlend {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}
impl From<BlendMode> for EffectBlend {
    fn from(value: BlendMode) -> Self {
        match value {
            BlendMode::Normal => Self::Normal,
            BlendMode::Multiply => Self::Multiply,
            BlendMode::Screen => Self::Screen,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}
/// Curves in CSS path() are resolved by the producer into contours at its
/// raster tolerance. All contours participate in one winding-rule evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum EffectClip {
    Inset {
        rect: Rect,
        radii: [f32; 4],
    },
    Ellipse {
        center: (f32, f32),
        radii: (f32, f32),
    },
    Polygon {
        points: Vec<(f32, f32)>,
        rule: FillRule,
    },
    Path {
        contours: Vec<Vec<(f32, f32)>>,
        rule: FillRule,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskMode {
    Alpha,
    Luminance,
}
/// A decoded, origin-checked mask image (including rasterized gradients). The
/// producer resolves mask-size/position/repeat into this immutable surface.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectMask {
    pub image: Arc<Surface>,
    pub rect: Rect,
    pub mode: MaskMode,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectOperations {
    pub filters: Vec<FilterOperation>,
    pub backdrop_filters: Vec<FilterOperation>,
    /// Border-box clipping of the filtered backdrop, before foreground draw.
    pub backdrop_bounds: Option<Rect>,
    pub blend: EffectBlend,
    pub isolate: bool,
    pub clip_path: Option<EffectClip>,
    pub mask: Option<EffectMask>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectError {
    InvalidNode,
    InvalidValue,
    LimitExceeded,
    UnresolvedFilter,
}
pub const MAX_EFFECT_FILTERS: usize = 32;
pub const MAX_EFFECT_POINTS: usize = 16_384;
pub const MAX_EFFECT_PIXELS: u64 = 16 * 1024 * 1024;
impl FilterOperation {
    fn valid(self) -> bool {
        match self {
            Self::DropShadow { offset, sigma, .. } => {
                offset.0.is_finite()
                    && offset.1.is_finite()
                    && offset.0.abs() <= 16384.
                    && offset.1.abs() <= 16384.
                    && sigma.is_finite()
                    && (0. ..=128.).contains(&sigma)
            }
            Self::HueRotate(v) => v.is_finite(),
            Self::Blur(v) => v.is_finite() && (0. ..=128.).contains(&v),
            Self::Grayscale(v) | Self::Invert(v) | Self::Opacity(v) | Self::Sepia(v) => {
                v.is_finite() && (0. ..=1.).contains(&v)
            }
            Self::Brightness(v) | Self::Contrast(v) | Self::Saturate(v) => {
                v.is_finite() && (0. ..=10000.).contains(&v)
            }
        }
    }
}
impl EffectOperations {
    pub fn validate(&self) -> Result<(), EffectError> {
        if self.filters.len() + self.backdrop_filters.len() > MAX_EFFECT_FILTERS {
            return Err(EffectError::LimitExceeded);
        }
        if !self
            .filters
            .iter()
            .chain(&self.backdrop_filters)
            .all(|v| v.valid())
        {
            return Err(EffectError::InvalidValue);
        }
        if !self.backdrop_filters.is_empty() && self.backdrop_bounds.is_none() {
            return Err(EffectError::InvalidValue);
        }
        if self.backdrop_bounds.is_some_and(|r| r.empty()) {
            return Err(EffectError::InvalidValue);
        }
        if let Some(clip) = &self.clip_path {
            clip.validate()?;
        }
        if let Some(mask) = &self.mask {
            let len = u64::from(mask.image.width) * u64::from(mask.image.height);
            if len > MAX_EFFECT_PIXELS {
                return Err(EffectError::LimitExceeded);
            }
            if mask.rect.empty() || len == 0 || len * 4 != mask.image.pixels.len() as u64 {
                return Err(EffectError::InvalidValue);
            }
        }
        Ok(())
    }
    pub fn is_active(&self) -> bool {
        self != &Self::default()
    }
    /// Conservative halo for tile selection; effects never bake into tiles.
    pub fn outset(&self) -> f32 {
        self.filters
            .iter()
            .map(|op| match *op {
                FilterOperation::Blur(s) => (3. * s).ceil(),
                FilterOperation::DropShadow { sigma, offset, .. } => {
                    (3. * sigma).ceil() + offset.0.abs().max(offset.1.abs())
                }
                _ => 0.,
            })
            .sum()
    }
}
impl PropertyTrees {
    pub fn effect_operations(&self, id: EffectId) -> Option<&EffectOperations> {
        self.effect_operations.get(&id)
    }
    pub fn set_effect_operations(
        &mut self,
        id: EffectId,
        operations: EffectOperations,
    ) -> Result<(), EffectError> {
        if id.0 == 0 || id.0 >= self.effects.len() {
            return Err(EffectError::InvalidNode);
        }
        operations.validate()?;
        if operations.is_active()
            && !self.effect_operations.contains_key(&id)
            && self.effect_operations.len() >= MAX_COMPOSITOR_LAYERS
        {
            return Err(EffectError::LimitExceeded);
        }
        if self.effects[id.0].filter.is_some() {
            return Err(EffectError::UnresolvedFilter);
        }
        if self.effect_operations.get(&id) == Some(&operations) {
            return Ok(());
        }
        if operations.is_active() {
            self.effect_operations.insert(id, operations);
        } else {
            self.effect_operations.remove(&id);
        }
        self.effect_revision = self.effect_revision.wrapping_add(1);
        Ok(())
    }
    pub fn has_composited_effects(&self) -> bool {
        !self.effect_operations.is_empty()
    }
}
impl Compositor {
    /// Changes only the retained effect tree. Raster epoch and tile IDs stay
    /// stable; the next cached frame publishes damage and the new operations.
    pub fn set_effect_operations(
        &mut self,
        id: EffectId,
        operations: EffectOperations,
    ) -> Result<(), EffectError> {
        self.trees.set_effect_operations(id, operations)
    }
}

/// Effects are stacking groups, not one pass per tile or per paint chunk.
/// An effect subtree must be contiguous in paint order; otherwise the producer
/// must rebuild the scene instead of accidentally blending a group twice.
#[derive(Default)]
pub(crate) struct EffectPassBuilder {
    stack: Vec<(EffectId, usize)>,
    closed: HashSet<EffectId>,
}
impl EffectPassBuilder {
    pub(crate) fn select(
        &mut self,
        trees: &PropertyTrees,
        id: EffectId,
        viewport: Rect,
        passes: &mut Vec<RenderPass>,
    ) -> Result<usize, SceneError> {
        let mut chain = Vec::new();
        let mut current = id;
        while current.0 != 0 {
            if chain.len() >= 16 {
                return Err(SceneError::InvalidState);
            }
            chain.push(current);
            current = trees
                .effects
                .get(current.0)
                .ok_or(SceneError::InvalidState)?
                .parent;
        }
        chain.reverse();
        let common = self
            .stack
            .iter()
            .zip(&chain)
            .take_while(|(a, b)| a.0 == **b)
            .count();
        for (id, _) in self.stack.drain(common..) {
            self.closed.insert(id);
        }
        for id in chain.into_iter().skip(common) {
            if self.closed.contains(&id) || passes.len() >= 1024 {
                return Err(SceneError::InvalidState);
            }
            let parent = self.stack.last().map_or(0, |p| p.1);
            let effect = trees.effects[id.0];
            if effect.opacity == 1.
                && effect.blend == BlendMode::Normal
                && trees.effect_operations(id).is_none()
            {
                self.stack.push((id, parent));
                continue;
            }
            let pass_id = passes.len();
            passes.push(RenderPass {
                id: pass_id,
                effect_node: Some(id),
                clip_node: None,
                quads: Vec::new(),
            });
            passes[parent].quads.push(DrawQuad::Pass(PassQuad {
                pass_id,
                rect: viewport,
                clip_rect: viewport,
                opacity: effect.opacity,
                blend: effect.blend,
                state: PropertyTreeState::default(),
            }));
            self.stack.push((id, pass_id));
        }
        Ok(self.stack.last().map_or(0, |p| p.1))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoundEffectChange {
    /// Uses the stable owner mapping of the effect's opacity node; a stale
    /// node generation fails the complete transaction before any mutation.
    pub owner: NodePropertyKey,
    pub operations: EffectOperations,
}
impl Compositor {
    pub fn apply_bound_effect_changes(
        &mut self,
        changes: &[BoundEffectChange],
    ) -> Result<(), PropertyBindingError> {
        let mut seen = HashSet::new();
        let mut targets = Vec::with_capacity(changes.len());
        for change in changes {
            if !seen.insert(change.owner) {
                return Err(PropertyBindingError::DuplicateKey);
            }
            let Some(&BoundPropertyNode::Opacity(id)) = self.property_bindings.get(&change.owner)
            else {
                return Err(PropertyBindingError::UnknownKey);
            };
            change
                .operations
                .validate()
                .map_err(|_| PropertyBindingError::NonFinite)?;
            if self
                .trees
                .effects
                .get(id.0)
                .is_none_or(|e| e.filter.is_some())
            {
                return Err(PropertyBindingError::InvalidNode);
            }
            targets.push(id);
        }
        let mut proposed = self.trees.clone();
        for (change, id) in changes.iter().zip(targets) {
            proposed
                .set_effect_operations(id, change.operations.clone())
                .map_err(|_| PropertyBindingError::InvalidNode)?;
        }
        self.trees = proposed;
        Ok(())
    }
}

impl PropertyTrees {
    /// Longest nested filter halo, rather than summing independent siblings.
    pub(crate) fn effect_halo(&self) -> f32 {
        let mut halos = vec![0f32; self.effects.len()];
        for (i, node) in self.effects.iter().enumerate().skip(1) {
            let own = self.effect_operations(EffectId(i)).map_or(0., |op| {
                let mut backdrop = op.clone();
                backdrop.filters = op.backdrop_filters.clone();
                op.outset() + backdrop.outset()
            });
            halos[i] = halos[node.parent.0] + own;
        }
        halos.into_iter().fold(0., f32::max)
    }
}
