//! Promotion does not override paint ordering. Squashing is only legal for
//! adjacent chunks with identical retained state; crossing a property owner
//! would make subsequent independent animation change unrelated pixels.
use super::*;
pub const MAX_COMPOSITOR_LAYERS: usize = 1024;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromotionHints {
    pub will_change_transform: bool,
    pub will_change_opacity: bool,
    pub animated_transform_or_opacity: bool,
    pub transform_3d: bool,
    pub video: bool,
    pub canvas: bool,
    pub fixed: bool,
    pub sticky: bool,
}
impl PromotionHints {
    pub fn reasons(self) -> CompositingReasons {
        let mut r = CompositingReasons::default();
        for (flag, reason) in [
            (
                self.will_change_transform || self.will_change_opacity,
                CompositingReasons::WILL_CHANGE,
            ),
            (
                self.animated_transform_or_opacity,
                CompositingReasons::ANIMATED,
            ),
            (self.transform_3d, CompositingReasons::TRANSFORM_3D),
            (self.video, CompositingReasons::VIDEO),
            (self.canvas, CompositingReasons::CANVAS),
            (self.fixed, CompositingReasons::FIXED),
            (self.sticky, CompositingReasons::STICKY),
        ] {
            if flag {
                r = r.union(reason);
            }
        }
        r
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerCounts {
    pub chunks: usize,
    pub layers: usize,
    pub promoted: usize,
    pub squashed_chunks: usize,
}
impl Compositor {
    pub fn layer_counts(&self) -> LayerCounts {
        let chunks = self.layers.iter().map(|l| l.chunks.len()).sum();
        LayerCounts {
            chunks,
            layers: self.layers.len(),
            promoted: self.layers.iter().filter(|l| l.reasons.direct()).count(),
            squashed_chunks: chunks - self.layers.len(),
        }
    }
}
pub(crate) fn inferred_reasons(
    trees: &PropertyTrees,
    state: PropertyTreeState,
) -> CompositingReasons {
    let m = trees
        .transform(state.transform)
        .map_or(Matrix4::IDENTITY, |t| t.accumulated)
        .0;
    PromotionHints {
        fixed: state.positioning == Positioning::Fixed,
        sticky: matches!(state.positioning, Positioning::Sticky(_)),
        transform_3d: m[0][2] != 0.
            || m[1][2] != 0.
            || m[2] != [0., 0., 1., 0.]
            || m[3] != [0., 0., 0., 1.],
        ..Default::default()
    }
    .reasons()
}
