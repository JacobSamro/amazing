//! Immutable per-quad sampling state. Resolve retained tree ancestry once,
//! without changing the pointwise homogeneous solve or clip predicates.
use super::*;

#[derive(Clone, Copy)]
pub(crate) enum PreparedUnmap {
    Affine { a: f32, b: f32, d: f32, e: f32, tx: f32, ty: f32, w: f32, det: f32 },
    Projective(Matrix4),
}
impl PreparedUnmap {
    pub(crate) fn new(matrix: Matrix4) -> Self {
        let m = matrix.0;
        // The z=0 homography is affine exactly when its denominator is
        // independent of x/y. Keep the original numerator order and divide
        // by the original determinant: a pre-divided inverse would change
        // rounding at texel and rounded-clip boundaries.
        if m[3][0] == 0. && m[3][1] == 0. {
            Self::Affine { a: m[0][0], b: m[0][1], d: m[1][0], e: m[1][1],
                tx: m[0][3], ty: m[1][3], w: m[3][3],
                det: m[0][0] * m[1][1] - m[0][1] * m[1][0] }
        } else { Self::Projective(matrix) }
    }
    pub(crate) fn unmap(self, x: f32, y: f32) -> Option<(f32, f32)> {
        match self {
            Self::Projective(matrix) => matrix.unmap(x, y),
            Self::Affine { a, b, d, e, tx, ty, w, det } => {
                if !x.is_finite() || !y.is_finite() || !det.is_finite() || det.abs() < 1e-8 {
                    return None;
                }
                let c = x * w - tx;
                let f = y * w - ty;
                let u = (c * e - b * f) / det;
                let v = (a * f - c * d) / det;
                (u.is_finite() && v.is_finite()).then_some((u, v))
            }
        }
    }
}

pub(crate) struct PreparedVisibility {
    clips: Vec<(PreparedUnmap, ClipShape)>,
    scroll_containers: Vec<Rect>,
    valid: bool,
}
impl PreparedVisibility {
    pub(crate) fn contains(&self, x: f32, y: f32) -> bool {
        self.valid
            && self.clips.iter().all(|(mapper, shape)| mapper.unmap(x, y)
                .is_some_and(|(u, v)| shape.contains(u, v)))
            && self.scroll_containers.iter().all(|rect| rect.contains(x, y))
    }
}
impl PropertyTrees {
    pub(crate) fn prepare_visibility(&self, state: PropertyTreeState) -> PreparedVisibility {
        let mut result = PreparedVisibility { clips: Vec::new(), scroll_containers: Vec::new(), valid: true };
        let mut clip = state.clip;
        while clip.0 != 0 {
            let Some(node) = self.clips.get(clip.0) else { result.valid = false; return result; };
            let Some(matrix) = self.clip_world_matrix(clip) else { result.valid = false; return result; };
            result.clips.push((PreparedUnmap::new(matrix), node.shape));
            clip = node.parent;
        }
        let mut scroll = if state.positioning == Positioning::Fixed { ScrollId(0) } else { state.scroll };
        while scroll.0 != 0 {
            let Some(node) = self.scrolls.get(scroll.0) else { result.valid = false; return result; };
            let Some(rect) = self.scroll_container_world(scroll, state.transform) else { result.valid = false; return result; };
            result.scroll_containers.push(rect);
            scroll = node.parent;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_unmap_preserves_affine_projective_and_singular_point_solves() {
        let mut perspective = Matrix4::affine(1.25, 0.3, -0.2, 0.75, 11.5, -7.25);
        perspective.0[3] = [0.001, -0.002, 0., 1.];
        let matrices = [Matrix4::IDENTITY, Matrix4::translation(-17.25, 10.4, 0.),
            Matrix4::scale(2., 2., 1.), Matrix4::scale(-1.25, 0.75, 1.),
            Matrix4::rotation_z(0.37), Matrix4::affine(1., 0.7, -0.3, 2., 8., -4.),
            Matrix4::scale(0., 1., 1.), Matrix4::scale(1e-9, 1., 1.), perspective];
        for matrix in matrices {
            let prepared = PreparedUnmap::new(matrix);
            for y in -96..96 {
                for x in -96..96 {
                    let point = (x as f32 * 0.375 + 0.5, y as f32 * 0.625 + 0.5);
                    assert_eq!(prepared.unmap(point.0, point.1), matrix.unmap(point.0, point.1));
                }
            }
            for (x, y) in [(0., -0.), (1e20, -1e20), (f32::NAN, 0.), (0., f32::INFINITY)] {
                assert_eq!(prepared.unmap(x, y), matrix.unmap(x, y));
            }
        }
    }

    #[test]
    fn prepared_visibility_matches_nested_clips_scroll_fixed_sticky_and_updates() {
        let mut trees = PropertyTrees::new();
        let owner = trees.add_transform(TransformId(0), Matrix4::scale(2., 2., 1.), (0., 0., 0.), false).unwrap();
        let child = trees.add_transform(owner, Matrix4::translation(7.25, -3.5, 0.), (0., 0., 0.), false).unwrap();
        let root = trees.add_scroll_with_owner(ScrollId(0), (0., 10.400002),
            Rect::new(0., 0., 64., 48.), Rect::new(0., 0., 128., 240.), Some(owner)).unwrap();
        let nested = trees.add_scroll_with_owner(root, (3., 5.),
            Rect::new(4., 2., 30., 24.), Rect::new(0., 0., 90., 120.), Some(child)).unwrap();
        let rect = trees.add_clip_with_scroll(ClipId(0), owner, root,
            ClipShape::Rect(Rect::new(1., 1., 60., 45.))).unwrap();
        let round = trees.add_clip_with_scroll(rect, child, nested,
            ClipShape::RoundedRect { rect: Rect::new(0., 0., 36., 28.), radii: [2., 5., 9., 13.] }).unwrap();
        let sticky = trees.add_sticky(StickyConstraint { scroll: nested,
            normal: Rect::new(5., 6., 12., 10.), top: Some(0.), left: Some(1.), right: None, bottom: None }).unwrap();
        for offset in [(0., 10.400002), (2.5, 18.75), (-3., 0.)] {
            trees.set_scroll(root, offset).unwrap();
            for transform in [TransformId(0), owner, child] {
                for clip in [ClipId(0), rect, round, ClipId(999)] {
                    for positioning in [Positioning::Flow, Positioning::Fixed, Positioning::Sticky(sticky)] {
                        let state = PropertyTreeState { transform, scroll: nested, clip, positioning, ..Default::default() };
                        let prepared = trees.prepare_visibility(state);
                        for y in -16..160 {
                            for x in -16..160 {
                                let (px, py) = (x as f32 * 0.75 + 0.5, y as f32 * 0.75 + 0.5);
                                assert_eq!(prepared.contains(px, py), trees.visible(state, px, py));
                            }
                        }
                    }
                }
            }
        }
    }
}
