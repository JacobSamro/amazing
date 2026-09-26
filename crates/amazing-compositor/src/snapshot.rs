//! Bounded, owned frame snapshots for an IPC transport. Encoding is owned by
//! the caller; these types carry no `amazing-ipc` dependency or borrowed data.
use super::*;
use std::collections::HashSet;

pub const SNAPSHOT_VERSION: u16 = 1;
const MAX_DIMENSION: u32 = 16_384;
const MAX_FRAME_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_PASSES: usize = 1_024;
const MAX_QUADS: usize = 1_000_000;
const MAX_NODES: usize = 65_536;
const MAX_DAMAGE: usize = 8_192;
const MAX_RESOURCES: usize = 4_096;
const MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_DELTA_BYTES: usize = 256 * 1024 * 1024;
const MAX_SNAP_POINTS: usize = 262_144;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion,
    LimitExceeded,
    InvalidGeometry,
    InvalidTree,
    InvalidPass,
    InvalidResource,
    DuplicateId,
    InvalidLookup,
}
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SnapshotError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceDeltaSnapshot {
    pub version: u16,
    pub uploads: Vec<ResourceUpload>,
    pub evictions: Vec<ResourceId>,
}

impl ResourceDelta {
    pub fn to_snapshot(&self) -> ResourceDeltaSnapshot {
        ResourceDeltaSnapshot {
            version: SNAPSHOT_VERSION,
            uploads: self.uploads.clone(),
            evictions: self.evictions.clone(),
        }
    }
    pub fn from_snapshot(snapshot: ResourceDeltaSnapshot) -> Result<Self, SnapshotError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        if snapshot.uploads.len() > MAX_RESOURCES || snapshot.evictions.len() > MAX_RESOURCES {
            return Err(SnapshotError::LimitExceeded);
        }
        let mut ids = HashSet::new();
        let mut total = 0usize;
        for upload in &snapshot.uploads {
            if upload.id.0 == 0 || !ids.insert(upload.id) {
                return Err(SnapshotError::DuplicateId);
            }
            let bytes = (upload.width as usize)
                .checked_mul(upload.height as usize)
                .and_then(|v| v.checked_mul(4))
                .ok_or(SnapshotError::LimitExceeded)?;
            if upload.width == 0
                || upload.height == 0
                || upload.width > MAX_DIMENSION
                || upload.height > MAX_DIMENSION
                || bytes > MAX_RESOURCE_BYTES
                || bytes != upload.pixels.len()
            {
                return Err(SnapshotError::InvalidResource);
            }
            total = total
                .checked_add(bytes)
                .ok_or(SnapshotError::LimitExceeded)?;
            if total > MAX_DELTA_BYTES {
                return Err(SnapshotError::LimitExceeded);
            }
        }
        for id in &snapshot.evictions {
            if id.0 == 0 || !ids.insert(*id) {
                return Err(SnapshotError::DuplicateId);
            }
        }
        Ok(Self {
            uploads: snapshot.uploads,
            evictions: snapshot.evictions,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PropertyTreesSnapshot {
    pub version: u16,
    pub transforms: Vec<TransformNode>,
    pub clips: Vec<ClipNode>,
    pub effects: Vec<EffectNode>,
    pub scrolls: Vec<ScrollNode>,
    pub sticky: Vec<StickyConstraint>,
    pub snap_points: Vec<(Vec<f32>, Vec<f32>)>,
}

impl PropertyTrees {
    pub fn to_snapshot(&self) -> PropertyTreesSnapshot {
        PropertyTreesSnapshot {
            // V1 has no effect payload. Mark it unsupported so an old GPU
            // transport cannot accept a frame after stripping its effects.
            version: if self.has_composited_effects() {
                EFFECT_SNAPSHOT_VERSION
            } else {
                SNAPSHOT_VERSION
            },
            transforms: self.transforms.clone(),
            clips: self.clips.clone(),
            effects: self.effects.clone(),
            scrolls: self.scrolls.clone(),
            sticky: self.sticky.clone(),
            snap_points: self.snap_points.clone(),
        }
    }
    pub fn from_snapshot(s: PropertyTreesSnapshot) -> Result<Self, SnapshotError> {
        if s.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        if [
            s.transforms.len(),
            s.clips.len(),
            s.effects.len(),
            s.scrolls.len(),
        ]
        .iter()
        .any(|&v| v == 0 || v > MAX_NODES)
            || s.sticky.len() > MAX_NODES
            || s.snap_points.len() != s.scrolls.len()
        {
            return Err(SnapshotError::LimitExceeded);
        }
        let points = s
            .snap_points
            .iter()
            .try_fold(0usize, |n, (x, y)| {
                n.checked_add(x.len())?.checked_add(y.len())
            })
            .ok_or(SnapshotError::LimitExceeded)?;
        if points > MAX_SNAP_POINTS {
            return Err(SnapshotError::LimitExceeded);
        }
        let mut trees = Self::new();
        if s.transforms[0] != trees.transforms[0]
            || s.clips[0] != trees.clips[0]
            || s.effects[0] != trees.effects[0]
            || s.scrolls[0] != trees.scrolls[0]
            || s.snap_points[0] != (Vec::new(), Vec::new())
        {
            return Err(SnapshotError::InvalidTree);
        }
        for (i, node) in s.transforms.iter().enumerate().skip(1) {
            if node.parent.0 >= i
                || !node.matrix.0.iter().flatten().all(|v| v.is_finite())
                || !node.accumulated.0.iter().flatten().all(|v| v.is_finite())
            {
                return Err(SnapshotError::InvalidTree);
            }
            trees
                .add_transform(node.parent, node.matrix, node.origin, node.flatten)
                .map_err(|_| SnapshotError::InvalidTree)?;
            if trees.transforms[i] != *node {
                return Err(SnapshotError::InvalidTree);
            }
        }
        for (i, node) in s.clips.iter().enumerate().skip(1) {
            if node.parent.0 >= i
                || node.transform.0 >= trees.transforms.len()
                || !valid_clip(node.shape)
            {
                return Err(SnapshotError::InvalidTree);
            }
            trees
                .add_clip(node.parent, node.transform, node.shape)
                .map_err(|_| SnapshotError::InvalidTree)?;
        }
        for (i, node) in s.effects.iter().enumerate().skip(1) {
            if node.parent.0 >= i || !unit(node.opacity) || !unit(node.accumulated_opacity) {
                return Err(SnapshotError::InvalidTree);
            }
            trees
                .add_effect(node.parent, node.opacity, node.blend, node.filter)
                .map_err(|_| SnapshotError::InvalidTree)?;
            if trees.effects[i] != *node {
                return Err(SnapshotError::InvalidTree);
            }
        }
        for (i, node) in s.scrolls.iter().enumerate().skip(1) {
            if node.parent.0 >= i
                || !valid_rect(node.container)
                || !valid_rect(node.contents)
                || ![node.accumulated.0, node.accumulated.1]
                    .iter()
                    .all(|v| v.is_finite())
            {
                return Err(SnapshotError::InvalidTree);
            }
            trees
                .add_scroll(node.parent, node.offset, node.container, node.contents)
                .map_err(|_| SnapshotError::InvalidTree)?;
            if trees.scrolls[i] != *node {
                return Err(SnapshotError::InvalidTree);
            }
        }
        for node in s.sticky {
            if !valid_rect(node.normal) {
                return Err(SnapshotError::InvalidTree);
            }
            trees
                .add_sticky(node)
                .map_err(|_| SnapshotError::InvalidTree)?;
        }
        for (i, (x, y)) in s.snap_points.into_iter().enumerate().skip(1) {
            trees
                .set_snap_points(ScrollId(i), x.clone(), y.clone())
                .map_err(|_| SnapshotError::InvalidTree)?;
            if trees.snap_points[i] != (x, y) {
                return Err(SnapshotError::InvalidTree);
            }
        }
        Ok(trees)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileResourceSnapshot {
    pub layer: usize,
    pub x: i32,
    pub y: i32,
    pub id: ResourceId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompositorFrameSnapshot {
    pub version: u16,
    pub width: u32,
    pub height: u32,
    pub passes: Vec<RenderPass>,
    pub resources: ResourceDeltaSnapshot,
    pub damage: Vec<Rect>,
    pub trees: PropertyTreesSnapshot,
    pub tile_lookup: Vec<TileResourceSnapshot>,
}

impl CompositorFrame {
    pub fn to_snapshot(&self) -> CompositorFrameSnapshot {
        let mut tile_lookup = self
            .tile_lookup
            .iter()
            .map(|(&(layer, x, y), &id)| TileResourceSnapshot { layer, x, y, id })
            .collect::<Vec<_>>();
        tile_lookup.sort_by_key(|item| (item.layer, item.x, item.y));
        CompositorFrameSnapshot {
            version: SNAPSHOT_VERSION,
            width: self.width,
            height: self.height,
            passes: self.passes.clone(),
            resources: self.resources.to_snapshot(),
            damage: self.damage.clone(),
            trees: self.trees.to_snapshot(),
            tile_lookup,
        }
    }
    pub fn from_snapshot(s: CompositorFrameSnapshot) -> Result<Self, SnapshotError> {
        if s.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        if s.width == 0
            || s.height == 0
            || s.width > MAX_DIMENSION
            || s.height > MAX_DIMENSION
            || (s.width as u64) * (s.height as u64) > MAX_FRAME_PIXELS
        {
            return Err(SnapshotError::LimitExceeded);
        }
        if s.passes.is_empty()
            || s.passes.len() > MAX_PASSES
            || s.damage.len() > MAX_DAMAGE
            || s.tile_lookup.len() > MAX_QUADS
        {
            return Err(SnapshotError::LimitExceeded);
        }
        let trees = PropertyTrees::from_snapshot(s.trees)?;
        let resources = ResourceDelta::from_snapshot(s.resources)?;
        if s.damage.iter().any(|r| !valid_rect(*r)) {
            return Err(SnapshotError::InvalidGeometry);
        }
        let mut total = 0usize;
        let mut expected_lookup = HashMap::new();
        for (i, pass) in s.passes.iter().enumerate() {
            if pass.id != i
                || pass
                    .effect_node
                    .is_some_and(|id| id.0 >= trees.effects.len())
                || pass.clip_node.is_some_and(|id| id.0 >= trees.clips.len())
            {
                return Err(SnapshotError::InvalidPass);
            }
            total = total
                .checked_add(pass.quads.len())
                .ok_or(SnapshotError::LimitExceeded)?;
            if total > MAX_QUADS {
                return Err(SnapshotError::LimitExceeded);
            }
            for quad in &pass.quads {
                match quad {
                    DrawQuad::Tile(q) => {
                        if q.resource_id.0 == 0
                            || q.layer > MAX_QUADS
                            || !valid_rect(q.rect)
                            || !valid_rect(q.content_rect)
                            || !valid_rect(q.clip_rect)
                            || !unit(q.opacity)
                            || !valid_matrix(q.transform)
                            || !valid_state(q.state, &trees)
                        {
                            return Err(SnapshotError::InvalidGeometry);
                        }
                        if q.rect.w != TILE_SIZE as f32
                            || q.rect.h != TILE_SIZE as f32
                            || q.rect.x % (TILE_SIZE as f32) != 0.
                            || q.rect.y % (TILE_SIZE as f32) != 0.
                        {
                            return Err(SnapshotError::InvalidLookup);
                        }
                        let key = (
                            q.layer,
                            (q.rect.x as i32).div_euclid(TILE_SIZE),
                            (q.rect.y as i32).div_euclid(TILE_SIZE),
                        );
                        if expected_lookup.insert(key, q.resource_id).is_some() {
                            return Err(SnapshotError::InvalidLookup);
                        }
                    }
                    DrawQuad::SolidColor(q) => {
                        if !valid_rect(q.rect) || !valid_rect(q.clip_rect) || !unit(q.opacity) {
                            return Err(SnapshotError::InvalidGeometry);
                        }
                    }
                    DrawQuad::Pass(q) => {
                        if q.pass_id == 0
                            || q.pass_id >= s.passes.len()
                            || !valid_rect(q.rect)
                            || !valid_rect(q.clip_rect)
                            || !unit(q.opacity)
                            || !valid_state(q.state, &trees)
                        {
                            return Err(SnapshotError::InvalidPass);
                        }
                    }
                }
            }
        }
        if pass_cycle(&s.passes) {
            return Err(SnapshotError::InvalidPass);
        }
        let evicted = resources.evictions.iter().copied().collect::<HashSet<_>>();
        if expected_lookup.values().any(|id| evicted.contains(id)) {
            return Err(SnapshotError::InvalidResource);
        }
        let mut lookup = HashMap::new();
        for item in s.tile_lookup {
            if item.layer > MAX_QUADS
                || lookup
                    .insert((item.layer, item.x, item.y), item.id)
                    .is_some()
            {
                return Err(SnapshotError::InvalidLookup);
            }
        }
        if lookup != expected_lookup {
            return Err(SnapshotError::InvalidLookup);
        }
        Ok(Self {
            width: s.width,
            height: s.height,
            passes: s.passes,
            resources,
            damage: s.damage,
            trees,
            tile_lookup: lookup,
        })
    }
}

fn valid_rect(r: Rect) -> bool {
    r.w.is_finite()
        && r.h.is_finite()
        && r.w > 0.
        && r.h > 0.
        && [r.x, r.y, r.x + r.w, r.y + r.h]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 1e9)
}
fn valid_clip(c: ClipShape) -> bool {
    match c {
        ClipShape::Rect(r) => valid_rect(r),
        ClipShape::RoundedRect { rect, radii } => {
            valid_rect(rect) && radii.iter().all(|v| v.is_finite() && *v >= 0.)
        }
    }
}
fn valid_matrix(m: Matrix4) -> bool {
    m.0.iter().flatten().all(|v| v.is_finite())
}
fn unit(v: f32) -> bool {
    v.is_finite() && (0. ..=1.).contains(&v)
}
fn valid_state(s: PropertyTreeState, t: &PropertyTrees) -> bool {
    s.transform.0 < t.transforms.len()
        && s.clip.0 < t.clips.len()
        && s.effect.0 < t.effects.len()
        && s.scroll.0 < t.scrolls.len()
        && match s.positioning {
            Positioning::Sticky(id) => id.0 < t.sticky.len(),
            _ => true,
        }
}
fn pass_cycle(passes: &[RenderPass]) -> bool {
    fn visit(i: usize, passes: &[RenderPass], mark: &mut [u8]) -> bool {
        if mark[i] == 1 {
            return true;
        }
        if mark[i] == 2 {
            return false;
        }
        mark[i] = 1;
        for quad in &passes[i].quads {
            if let DrawQuad::Pass(q) = quad {
                if visit(q.pass_id, passes, mark) {
                    return true;
                }
            }
        }
        mark[i] = 2;
        false
    }
    let mut mark = vec![0; passes.len()];
    (0..passes.len()).any(|i| visit(i, passes, &mut mark))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

    struct Paint;
    impl PaintSource for Paint {
        fn item_count(&self) -> usize {
            1
        }
        fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
            out.fill_rect(4. - tile.x, 4. - tile.y, 24., 24., [50, 100, 200, 255]);
        }
    }
    fn frame() -> CompositorFrame {
        let mut trees = PropertyTrees::new();
        let transform = trees
            .add_transform(
                TransformId(0),
                Matrix4::translation(2., 3., 0.),
                (0., 0., 0.),
                false,
            )
            .unwrap();
        let clip = trees
            .add_clip(
                ClipId(0),
                transform,
                ClipShape::Rect(Rect::new(0., 0., 64., 64.)),
            )
            .unwrap();
        let effect = trees
            .add_effect(EffectId(0), 0.8, BlendMode::Normal, None)
            .unwrap();
        let scroll = trees
            .add_scroll(
                ScrollId(0),
                (3., 0.),
                Rect::new(0., 0., 64., 64.),
                Rect::new(0., 0., 128., 64.),
            )
            .unwrap();
        trees
            .set_snap_points(scroll, vec![0., 32., 64.], vec![0.])
            .unwrap();
        let sticky = trees
            .add_sticky(StickyConstraint {
                scroll,
                normal: Rect::new(4., 4., 24., 24.),
                top: Some(0.),
                right: None,
                bottom: None,
                left: None,
            })
            .unwrap();
        let mut compositor = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state: PropertyTreeState {
                    transform,
                    clip,
                    effect,
                    scroll,
                    positioning: Positioning::Sticky(sticky),
                },
                bounds: Rect::new(4., 4., 24., 24.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        compositor.build_frame(&Paint, 64, 64).unwrap()
    }

    #[test]
    fn frame_and_resource_deltas_roundtrip_owned_state() {
        let original = frame();
        let snapshot = original.to_snapshot();
        assert_eq!(snapshot.version, SNAPSHOT_VERSION);
        assert_eq!(snapshot.resources.version, SNAPSHOT_VERSION);
        let restored = CompositorFrame::from_snapshot(snapshot.clone()).unwrap();
        assert_eq!(restored.to_snapshot(), snapshot);
        assert_eq!(restored.root().quads, original.root().quads);
        assert_eq!(
            restored.resource_id_at(0, 0, 0),
            original.resource_id_at(0, 0, 0)
        );
        let delta = original.resources.to_snapshot();
        assert_eq!(
            ResourceDelta::from_snapshot(delta.clone())
                .unwrap()
                .to_snapshot(),
            delta
        );
    }

    #[test]
    fn rejects_version_bounds_ids_pass_cycles_and_corrupt_tree() {
        let valid = frame().to_snapshot();
        let mut s = valid.clone();
        s.version = 2;
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::UnsupportedVersion)
        ));
        let mut s = valid.clone();
        s.width = MAX_DIMENSION + 1;
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::LimitExceeded)
        ));
        let mut s = valid.clone();
        s.trees.transforms[1].parent = TransformId(1);
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::InvalidTree)
        ));
        let mut s = valid.clone();
        s.passes[0].id = 1;
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::InvalidPass)
        ));
        let mut s = valid.clone();
        s.damage.push(Rect::new(f32::NAN, 0., 1., 1.));
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::InvalidGeometry)
        ));
        let mut s = valid.clone();
        s.tile_lookup[0].id = ResourceId(999_999);
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::InvalidLookup)
        ));
        let mut s = valid.clone();
        s.passes.push(RenderPass {
            id: 1,
            effect_node: None,
            clip_node: None,
            quads: vec![DrawQuad::Pass(PassQuad {
                pass_id: 1,
                rect: Rect::new(0., 0., 1., 1.),
                clip_rect: Rect::new(0., 0., 1., 1.),
                opacity: 1.,
                blend: BlendMode::Normal,
                state: PropertyTreeState::default(),
            })],
        });
        assert!(matches!(
            CompositorFrame::from_snapshot(s),
            Err(SnapshotError::InvalidPass)
        ));
        let mut delta = valid.resources;
        delta.uploads[0].pixels.pop();
        assert!(matches!(
            ResourceDelta::from_snapshot(delta),
            Err(SnapshotError::InvalidResource)
        ));
    }
}
