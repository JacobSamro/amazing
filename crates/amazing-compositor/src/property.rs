//! Four retained property trees. Node IDs are stable until the scene is rebuilt.
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            (self.x + self.w).max(other.x + other.w) - x,
            (self.y + self.h).max(other.y + other.h) - y,
        )
    }
    pub fn empty(self) -> bool {
        self.w <= 0.0
            || self.h <= 0.0
            || ![self.x, self.y, self.w, self.h]
                .iter()
                .all(|n| n.is_finite())
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix4(pub [[f32; 4]; 4]);
impl Matrix4 {
    pub const IDENTITY: Self = Self([
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]);
    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::IDENTITY;
        m.0[0][3] = x;
        m.0[1][3] = y;
        m.0[2][3] = z;
        m
    }
    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        Self([
            [x, 0., 0., 0.],
            [0., y, 0., 0.],
            [0., 0., z, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn rotation_z(r: f32) -> Self {
        let (s, c) = r.sin_cos();
        Self([
            [c, -s, 0., 0.],
            [s, c, 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn affine(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self([
            [a, c, 0., e],
            [b, d, 0., f],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn mul(self, b: Self) -> Self {
        let mut out = [[0.; 4]; 4];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[i][k] * b.0[k][j]).sum();
            }
        }
        Self(out)
    }
    pub fn map(self, x: f32, y: f32) -> Option<(f32, f32)> {
        let m = self.0;
        let w = m[3][0] * x + m[3][1] * y + m[3][3];
        if !w.is_finite() || w.abs() < 1e-8 {
            return None;
        }
        let a = (m[0][0] * x + m[0][1] * y + m[0][3]) / w;
        let b = (m[1][0] * x + m[1][1] * y + m[1][3]) / w;
        (a.is_finite() && b.is_finite()).then_some((a, b))
    }
    /// Inverts the homography induced on the z=0 plane.
    pub fn unmap(self, x: f32, y: f32) -> Option<(f32, f32)> {
        let m = self.0;
        let h = [
            [m[0][0], m[0][1], m[0][3]],
            [m[1][0], m[1][1], m[1][3]],
            [m[3][0], m[3][1], m[3][3]],
        ];
        let a = h[0][0] - x * h[2][0];
        let b = h[0][1] - x * h[2][1];
        let c = x * h[2][2] - h[0][2];
        let d = h[1][0] - y * h[2][0];
        let e = h[1][1] - y * h[2][1];
        let f = y * h[2][2] - h[1][2];
        let det = a * e - b * d;
        if !det.is_finite() || det.abs() < 1e-8 {
            return None;
        }
        let u = (c * e - b * f) / det;
        let v = (a * f - c * d) / det;
        (u.is_finite() && v.is_finite()).then_some((u, v))
    }
    fn flattened(mut self) -> Self {
        self.0[0][2] = 0.;
        self.0[1][2] = 0.;
        self.0[2] = [0., 0., 1., 0.];
        self.0[3][2] = 0.;
        self
    }
    pub fn map_bounds(self, r: Rect) -> Option<Rect> {
        let p = [
            self.map(r.x, r.y)?,
            self.map(r.x + r.w, r.y)?,
            self.map(r.x, r.y + r.h)?,
            self.map(r.x + r.w, r.y + r.h)?,
        ];
        let (mut x0, mut y0, mut x1, mut y1) = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for (x, y) in p {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y)
        }
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TransformId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ClipId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EffectId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScrollId(pub usize);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PropertyTreeState {
    pub transform: TransformId,
    pub clip: ClipId,
    pub effect: EffectId,
    pub scroll: ScrollId,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformNode {
    pub parent: TransformId,
    pub matrix: Matrix4,
    pub origin: (f32, f32, f32),
    pub flatten: bool,
    pub accumulated: Matrix4,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipShape {
    Rect(Rect),
    RoundedRect { rect: Rect, radii: [f32; 4] },
}
impl ClipShape {
    fn contains(self, x: f32, y: f32) -> bool {
        let (r, rad) = match self {
            Self::Rect(r) => (r, [0.; 4]),
            Self::RoundedRect { rect, radii } => (rect, radii),
        };
        if !r.contains(x, y) {
            return false;
        }
        let corners = [
            (
                r.x + rad[0],
                r.y + rad[0],
                rad[0],
                x < r.x + rad[0] && y < r.y + rad[0],
            ),
            (
                r.x + r.w - rad[1],
                r.y + rad[1],
                rad[1],
                x >= r.x + r.w - rad[1] && y < r.y + rad[1],
            ),
            (
                r.x + r.w - rad[2],
                r.y + r.h - rad[2],
                rad[2],
                x >= r.x + r.w - rad[2] && y >= r.y + r.h - rad[2],
            ),
            (
                r.x + rad[3],
                r.y + r.h - rad[3],
                rad[3],
                x < r.x + rad[3] && y >= r.y + r.h - rad[3],
            ),
        ];
        corners.into_iter().all(|(cx, cy, rr, inside)| {
            !inside || rr <= 0. || (x - cx).powi(2) + (y - cy).powi(2) <= rr.powi(2)
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipNode {
    pub parent: ClipId,
    pub transform: TransformId,
    pub shape: ClipShape,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectNode {
    pub parent: EffectId,
    pub opacity: f32,
    pub blend: BlendMode,
    pub filter: Option<u64>,
    pub accumulated_opacity: f32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollNode {
    pub parent: ScrollId,
    pub offset: (f32, f32),
    pub container: Rect,
    pub contents: Rect,
    pub accumulated: (f32, f32),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    InvalidParent,
    InvalidNode,
    NonFinite,
}
impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for TreeError {}
#[derive(Debug, Clone)]
pub struct PropertyTrees {
    pub(crate) transforms: Vec<TransformNode>,
    pub(crate) clips: Vec<ClipNode>,
    pub(crate) effects: Vec<EffectNode>,
    pub(crate) scrolls: Vec<ScrollNode>,
}
impl Default for PropertyTrees {
    fn default() -> Self {
        Self::new()
    }
}
impl PropertyTrees {
    pub fn transform(&self, id: TransformId) -> Option<&TransformNode> {
        self.transforms.get(id.0)
    }
    pub fn clip(&self, id: ClipId) -> Option<&ClipNode> {
        self.clips.get(id.0)
    }
    pub fn effect(&self, id: EffectId) -> Option<&EffectNode> {
        self.effects.get(id.0)
    }
    pub fn scroll(&self, id: ScrollId) -> Option<&ScrollNode> {
        self.scrolls.get(id.0)
    }
    pub fn new() -> Self {
        Self {
            transforms: vec![TransformNode {
                parent: TransformId(0),
                matrix: Matrix4::IDENTITY,
                origin: (0., 0., 0.),
                flatten: false,
                accumulated: Matrix4::IDENTITY,
            }],
            clips: vec![ClipNode {
                parent: ClipId(0),
                transform: TransformId(0),
                shape: ClipShape::Rect(Rect::new(-1e9, -1e9, 2e9, 2e9)),
            }],
            effects: vec![EffectNode {
                parent: EffectId(0),
                opacity: 1.,
                blend: BlendMode::Normal,
                filter: None,
                accumulated_opacity: 1.,
            }],
            scrolls: vec![ScrollNode {
                parent: ScrollId(0),
                offset: (0., 0.),
                container: Rect::new(-1e9, -1e9, 2e9, 2e9),
                contents: Rect::new(-1e9, -1e9, 2e9, 2e9),
                accumulated: (0., 0.),
            }],
        }
    }
    pub fn add_transform(
        &mut self,
        parent: TransformId,
        matrix: Matrix4,
        origin: (f32, f32, f32),
        flatten: bool,
    ) -> Result<TransformId, TreeError> {
        if parent.0 >= self.transforms.len() {
            return Err(TreeError::InvalidParent);
        }
        if !origin.0.is_finite()
            || !origin.1.is_finite()
            || !origin.2.is_finite()
            || !matrix.0.iter().flatten().all(|v| v.is_finite())
        {
            return Err(TreeError::NonFinite);
        }
        let id = TransformId(self.transforms.len());
        self.transforms.push(TransformNode {
            parent,
            matrix,
            origin,
            flatten,
            accumulated: Matrix4::IDENTITY,
        });
        self.recompute_transforms();
        Ok(id)
    }
    pub fn set_transform(&mut self, id: TransformId, matrix: Matrix4) -> Result<(), TreeError> {
        if id.0 == 0 || id.0 >= self.transforms.len() {
            return Err(TreeError::InvalidNode);
        }
        if !matrix.0.iter().flatten().all(|v| v.is_finite()) {
            return Err(TreeError::NonFinite);
        }
        self.transforms[id.0].matrix = matrix;
        self.recompute_transforms();
        Ok(())
    }
    fn recompute_transforms(&mut self) {
        for i in 1..self.transforms.len() {
            let n = self.transforms[i];
            let parent = self.transforms[n.parent.0];
            let base = if parent.flatten {
                parent.accumulated.flattened()
            } else {
                parent.accumulated
            };
            self.transforms[i].accumulated = base
                .mul(Matrix4::translation(n.origin.0, n.origin.1, n.origin.2))
                .mul(n.matrix)
                .mul(Matrix4::translation(-n.origin.0, -n.origin.1, -n.origin.2));
        }
    }
    pub fn add_clip(
        &mut self,
        parent: ClipId,
        transform: TransformId,
        shape: ClipShape,
    ) -> Result<ClipId, TreeError> {
        if parent.0 >= self.clips.len() || transform.0 >= self.transforms.len() {
            return Err(TreeError::InvalidParent);
        }
        let id = ClipId(self.clips.len());
        self.clips.push(ClipNode {
            parent,
            transform,
            shape,
        });
        Ok(id)
    }
    pub fn add_effect(
        &mut self,
        parent: EffectId,
        opacity: f32,
        blend: BlendMode,
        filter: Option<u64>,
    ) -> Result<EffectId, TreeError> {
        if parent.0 >= self.effects.len() {
            return Err(TreeError::InvalidParent);
        }
        if !opacity.is_finite() {
            return Err(TreeError::NonFinite);
        }
        let id = EffectId(self.effects.len());
        let accumulated_opacity =
            self.effects[parent.0].accumulated_opacity * opacity.clamp(0., 1.);
        self.effects.push(EffectNode {
            parent,
            opacity: opacity.clamp(0., 1.),
            blend,
            filter,
            accumulated_opacity,
        });
        Ok(id)
    }
    pub fn set_opacity(&mut self, id: EffectId, opacity: f32) -> Result<(), TreeError> {
        if id.0 == 0 || id.0 >= self.effects.len() {
            return Err(TreeError::InvalidNode);
        }
        if !opacity.is_finite() {
            return Err(TreeError::NonFinite);
        }
        self.effects[id.0].opacity = opacity.clamp(0., 1.);
        for i in 1..self.effects.len() {
            let p = self.effects[i].parent.0;
            self.effects[i].accumulated_opacity =
                self.effects[p].accumulated_opacity * self.effects[i].opacity;
        }
        Ok(())
    }
    pub fn add_scroll(
        &mut self,
        parent: ScrollId,
        offset: (f32, f32),
        container: Rect,
        contents: Rect,
    ) -> Result<ScrollId, TreeError> {
        if parent.0 >= self.scrolls.len() {
            return Err(TreeError::InvalidParent);
        }
        if !offset.0.is_finite() || !offset.1.is_finite() || container.empty() || contents.empty() {
            return Err(TreeError::NonFinite);
        }
        let id = ScrollId(self.scrolls.len());
        let p = self.scrolls[parent.0].accumulated;
        self.scrolls.push(ScrollNode {
            parent,
            offset,
            container,
            contents,
            accumulated: (p.0 + offset.0, p.1 + offset.1),
        });
        Ok(id)
    }
    pub fn set_scroll(&mut self, id: ScrollId, offset: (f32, f32)) -> Result<(), TreeError> {
        if id.0 == 0 || id.0 >= self.scrolls.len() {
            return Err(TreeError::InvalidNode);
        }
        if !offset.0.is_finite() || !offset.1.is_finite() {
            return Err(TreeError::NonFinite);
        }
        self.scrolls[id.0].offset = offset;
        for i in 1..self.scrolls.len() {
            let p = self.scrolls[self.scrolls[i].parent.0].accumulated;
            let o = self.scrolls[i].offset;
            self.scrolls[i].accumulated = (p.0 + o.0, p.1 + o.1)
        }
        Ok(())
    }
    pub fn matrix(&self, s: PropertyTreeState) -> Option<Matrix4> {
        let t = self.transforms.get(s.transform.0)?.accumulated;
        let o = self.scrolls.get(s.scroll.0)?.accumulated;
        Some(t.mul(Matrix4::translation(-o.0, -o.1, 0.)))
    }
    pub fn visible(&self, s: PropertyTreeState, x: f32, y: f32) -> bool {
        let mut id = s.clip;
        while id.0 != 0 {
            let Some(n) = self.clips.get(id.0) else {
                return false;
            };
            let Some((u, v)) = self.transforms[n.transform.0].accumulated.unmap(x, y) else {
                return false;
            };
            if !n.shape.contains(u, v) {
                return false;
            }
            id = n.parent
        }
        let mut sid = s.scroll;
        while sid.0 != 0 {
            let Some(n) = self.scrolls.get(sid.0) else {
                return false;
            };
            if !n.container.contains(x, y) {
                return false;
            }
            sid = n.parent
        }
        true
    }
    pub fn opacity(&self, s: PropertyTreeState) -> f32 {
        self.effects
            .get(s.effect.0)
            .map_or(0., |e| e.accumulated_opacity)
    }
}
