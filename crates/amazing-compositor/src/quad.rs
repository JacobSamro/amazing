//! A retained tile becomes a textured quad. Building this frame does no pixel
//! composition; the presenter owns texture upload, pass execution and display.
//! See `PRESENTER.md` for the boundary contract.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceId(pub u64);
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceUpload {
    pub id: ResourceId,
    pub width: u32,
    pub height: u32,
    /// Row-major, top-left-origin, sRGB straight-alpha RGBA8. See PRESENTER.md.
    pub pixels: Vec<u8>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceDelta {
    pub uploads: Vec<ResourceUpload>,
    pub evictions: Vec<ResourceId>,
}
/// Producer or presenter-side map of live tile bitmaps. Applying deltas is
/// idempotent for uploads; evicted IDs must not appear in later quads.
#[derive(Debug, Clone, Default)]
pub struct ResourceTable {
    entries: HashMap<ResourceId, Surface>,
}
impl ResourceTable {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn get(&self, id: ResourceId) -> Option<&Surface> {
        self.entries.get(&id)
    }
    pub fn apply(&mut self, delta: &ResourceDelta) -> Result<(), PresenterError> {
        for u in &delta.uploads {
            let expected = (u.width as usize)
                .checked_mul(u.height as usize)
                .and_then(|v| v.checked_mul(4))
                .ok_or(PresenterError::InvalidResource)?;
            if u.pixels.len() != expected {
                return Err(PresenterError::InvalidResource);
            }
        }
        for id in &delta.evictions {
            self.entries.remove(id);
        }
        for u in &delta.uploads {
            self.entries.insert(
                u.id,
                Surface {
                    width: u.width,
                    height: u.height,
                    pixels: u.pixels.clone(),
                },
            );
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct TileQuad {
    pub resource_id: ResourceId,
    pub rect: Rect,
    pub content_rect: Rect,
    pub transform: Matrix4,
    pub clip_rect: Rect,
    pub opacity: f32,
    pub blend: BlendMode,
    pub state: PropertyTreeState,
    pub layer: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SolidColorQuad {
    pub rect: Rect,
    pub color: [u8; 4],
    pub clip_rect: Rect,
    pub opacity: f32,
    pub blend: BlendMode,
}
#[derive(Debug, Clone, PartialEq)]
pub struct PassQuad {
    pub pass_id: usize,
    pub rect: Rect,
    pub clip_rect: Rect,
    pub opacity: f32,
    pub blend: BlendMode,
    pub state: PropertyTreeState,
}
#[derive(Debug, Clone, PartialEq)]
pub enum DrawQuad {
    Tile(TileQuad),
    SolidColor(SolidColorQuad),
    Pass(PassQuad),
}
#[derive(Debug, Clone, PartialEq)]
pub struct RenderPass {
    pub id: usize,
    pub effect_node: Option<EffectId>,
    pub clip_node: Option<ClipId>,
    pub quads: Vec<DrawQuad>,
}
#[derive(Debug, Clone)]
pub struct CompositorFrame {
    pub width: u32,
    pub height: u32,
    pub passes: Vec<RenderPass>,
    pub resources: ResourceDelta,
    pub damage: Vec<Rect>,
    pub trees: PropertyTrees,
    tile_lookup: HashMap<(usize, i32, i32), ResourceId>,
}
impl CompositorFrame {
    pub fn root(&self) -> &RenderPass {
        &self.passes[0]
    }
    pub fn quad_count(&self) -> usize {
        self.passes.iter().map(|p| p.quads.len()).sum()
    }
    pub fn resource_id_at(&self, layer: usize, x: i32, y: i32) -> Option<ResourceId> {
        self.tile_lookup.get(&(layer, x, y)).copied()
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct QuadSnapshot {
    matrix: Matrix4,
    opacity: f32,
    blend: BlendMode,
    bounds: Rect,
    clip: ClipId,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenterError {
    InvalidResource,
    MissingResource(ResourceId),
    MissingPass(usize),
    PassCycle,
}
impl std::fmt::Display for PresenterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PresenterError {}
/// GPUI implements this in the shell. The frame borrows no compositor state;
/// all tile content arrives in `ResourceDelta` before quads reference it.
pub trait Presenter {
    type Output;
    type Error;
    fn present(&mut self, frame: &CompositorFrame) -> Result<Self::Output, Self::Error>;
}
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    Rect::new(x, y, (right - x).max(0.), (bottom - y).max(0.))
}
fn clip_rect(trees: &PropertyTrees, state: PropertyTreeState, viewport: Rect) -> Rect {
    let mut result = viewport;
    let mut id = state.clip;
    while id.0 != 0 {
        let Some(node) = trees.clips.get(id.0) else {
            break;
        };
        let r = match node.shape {
            ClipShape::Rect(r) => r,
            ClipShape::RoundedRect { rect, .. } => rect,
        };
        if let Some(world) = trees.transforms[node.transform.0].accumulated.map_bounds(r) {
            result = intersect(result, world)
        }
        id = node.parent
    }
    let mut sid = state.scroll;
    while sid.0 != 0 {
        let Some(node) = trees.scrolls.get(sid.0) else {
            break;
        };
        result = intersect(result, node.container);
        sid = node.parent
    }
    result
}
fn add_damage(damage: &mut Vec<Rect>, rect: Rect) {
    if rect.empty() {
        return;
    }
    let mut current = rect;
    let mut i = 0;
    while i < damage.len() {
        let d = damage[i];
        let touch = current.x <= d.x + d.w
            && d.x <= current.x + current.w
            && current.y <= d.y + d.h
            && d.y <= current.y + current.h;
        if touch {
            current = current.union(d);
            damage.swap_remove(i);
            i = 0
        } else {
            i += 1
        }
    }
    damage.push(current)
}
impl Compositor {
    /// Builds passes and resource deltas without flattening any layer into an
    /// output frame. A renderer may upload deltas, then execute pass 0.
    pub fn build_frame(
        &mut self,
        source: &impl PaintSource,
        width: u32,
        height: u32,
    ) -> Result<CompositorFrame, SceneError> {
        if self
            .chunks
            .last()
            .is_some_and(|c| c.items.end > source.item_count())
        {
            return Err(SceneError::InvalidRange);
        }
        let viewport = Rect::new(0., 0., width as f32, height as f32);
        let mut passes = vec![RenderPass {
            id: 0,
            effect_node: None,
            clip_node: None,
            quads: Vec::new(),
        }];
        let mut delta = ResourceDelta::default();
        let mut lookup = HashMap::new();
        let mut visible = HashSet::new();
        let mut snapshots = Vec::new();
        let mut damage = Vec::new();
        for (li, layer) in self.layers.iter().enumerate() {
            let matrix = self.trees.matrix(layer.state).unwrap_or(Matrix4::IDENTITY);
            let bounds = matrix.map_bounds(layer.bounds).unwrap_or_default();
            let opacity = self.trees.opacity(layer.state);
            let blend = self.trees.effects[layer.state.effect.0].blend;
            let clip = layer.state.clip;
            snapshots.push(QuadSnapshot {
                matrix,
                opacity,
                blend,
                bounds,
                clip,
            });
            let scissor = clip_rect(&self.trees, layer.state, viewport);
            let needs_pass = opacity < 1. || blend != BlendMode::Normal || clip.0 != 0;
            let pass_id = if needs_pass {
                let id = passes.len();
                passes.push(RenderPass {
                    id,
                    effect_node: (layer.state.effect.0 != 0).then_some(layer.state.effect),
                    clip_node: (clip.0 != 0).then_some(clip),
                    quads: Vec::new(),
                });
                passes[0].quads.push(DrawQuad::Pass(PassQuad {
                    pass_id: id,
                    rect: bounds,
                    clip_rect: scissor,
                    opacity,
                    blend,
                    state: layer.state,
                }));
                id
            } else {
                0
            };
            let keys = tile_keys(li, layer.bounds);
            if keys.len() > 1_000_000 {
                return Err(SceneError::TooManyTiles);
            }
            for key in keys {
                let local = tile_rect(key);
                if !matrix
                    .map_bounds(local)
                    .is_some_and(|r| r.intersects(scissor))
                {
                    continue;
                }
                visible.insert(key);
                if visible.len() > MAX_VISIBLE_TILES {
                    return Err(SceneError::TooManyTiles);
                }
                let needs = self.tiles.get(&key).is_none_or(|t| t.dirty);
                if needs {
                    let mut surface = Surface::new(TILE_SIZE as u32, TILE_SIZE as u32);
                    for &ci in &layer.chunks {
                        let chunk = &self.chunks[ci];
                        if chunk.bounds.intersects(local) {
                            source.rasterize(chunk.items.clone(), local, &mut surface)
                        }
                    }
                    self.tiles.insert(
                        key,
                        Tile {
                            surface,
                            dirty: false,
                        },
                    );
                    if let Some(r) = matrix.map_bounds(local) {
                        add_damage(&mut damage, intersect(r, viewport))
                    }
                }
                let id = if let Some(id) = self.tile_resource_ids.get(&key) {
                    *id
                } else {
                    let id = ResourceId(self.next_resource_id);
                    self.next_resource_id = self
                        .next_resource_id
                        .checked_add(1)
                        .ok_or(SceneError::TooManyTiles)?;
                    self.tile_resource_ids.insert(key, id);
                    id
                };
                let surface = &self.tiles[&key].surface;
                if self.resources.get(id) != Some(surface) {
                    let upload = ResourceUpload {
                        id,
                        width: surface.width,
                        height: surface.height,
                        pixels: surface.pixels.clone(),
                    };
                    self.resources
                        .apply(&ResourceDelta {
                            uploads: vec![upload.clone()],
                            evictions: Vec::new(),
                        })
                        .map_err(|_| SceneError::TooManyTiles)?;
                    delta.uploads.push(upload)
                }
                lookup.insert((li, key.x, key.y), id);
                passes[pass_id].quads.push(DrawQuad::Tile(TileQuad {
                    resource_id: id,
                    rect: local,
                    content_rect: layer.bounds,
                    transform: matrix,
                    clip_rect: scissor,
                    opacity: if needs_pass { 1. } else { opacity },
                    blend: if needs_pass { BlendMode::Normal } else { blend },
                    state: layer.state,
                    layer: li,
                }));
            }
        }
        let old_keys: Vec<_> = self
            .tile_resource_ids
            .keys()
            .copied()
            .filter(|k| !visible.contains(k))
            .collect();
        for key in old_keys {
            if let Some(id) = self.tile_resource_ids.remove(&key) {
                delta.evictions.push(id);
                self.resources.entries.remove(&id);
            }
        }
        delta.evictions.sort_unstable();
        self.tiles.retain(|key, _| visible.contains(key));
        if self.previous_quad_size != Some((width, height))
            || self.previous_quad_layers.len() != snapshots.len()
        {
            damage.clear();
            damage.push(viewport)
        } else {
            for (old, new) in self.previous_quad_layers.iter().zip(&snapshots) {
                if old != new {
                    add_damage(&mut damage, intersect(old.bounds, viewport));
                    add_damage(&mut damage, intersect(new.bounds, viewport))
                }
            }
        }
        self.previous_quad_layers = snapshots;
        self.previous_quad_size = Some((width, height));
        Ok(CompositorFrame {
            width,
            height,
            passes,
            resources: delta,
            damage,
            trees: self.trees.clone(),
            tile_lookup: lookup,
        })
    }
}
/// Software presenter used as the quad-path pixel oracle and for headless QA.
#[derive(Default)]
pub struct ReferenceCpuPresenter {
    pub resources: ResourceTable,
}
fn resource_pixel(
    resources: &ResourceTable,
    frame: &CompositorFrame,
    layer: usize,
    x: i32,
    y: i32,
) -> [u8; 4] {
    let tx = x.div_euclid(TILE_SIZE);
    let ty = y.div_euclid(TILE_SIZE);
    let Some(id) = frame.resource_id_at(layer, tx, ty) else {
        return [0; 4];
    };
    let Some(surface) = resources.get(id) else {
        return [0; 4];
    };
    surface
        .get_pixel(
            x.rem_euclid(TILE_SIZE) as u32,
            y.rem_euclid(TILE_SIZE) as u32,
        )
        .unwrap_or([0; 4])
}
fn sample(
    resources: &ResourceTable,
    frame: &CompositorFrame,
    layer: usize,
    sx: f32,
    sy: f32,
    integer: bool,
) -> [u8; 4] {
    if integer {
        return resource_pixel(
            resources,
            frame,
            layer,
            sx.floor() as i32,
            sy.floor() as i32,
        );
    }
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x = fx.floor() as i32;
    let y = fy.floor() as i32;
    let tx = fx - x as f32;
    let ty = fy - y as f32;
    let weights = [
        (1. - tx) * (1. - ty),
        tx * (1. - ty),
        (1. - tx) * ty,
        tx * ty,
    ];
    let samples = [
        resource_pixel(resources, frame, layer, x, y),
        resource_pixel(resources, frame, layer, x + 1, y),
        resource_pixel(resources, frame, layer, x, y + 1),
        resource_pixel(resources, frame, layer, x + 1, y + 1),
    ];
    let mut a = 0.;
    let mut p = [0.; 3];
    for (c, w) in samples.into_iter().zip(weights) {
        let alpha = c[3] as f32 * w;
        a += alpha;
        for k in 0..3 {
            p[k] += c[k] as f32 * alpha
        }
    }
    if a <= 0. {
        return [0; 4];
    }
    [
        (p[0] / a).round().clamp(0., 255.) as u8,
        (p[1] / a).round().clamp(0., 255.) as u8,
        (p[2] / a).round().clamp(0., 255.) as u8,
        a.round().clamp(0., 255.) as u8,
    ]
}
fn blend(dst: [u8; 4], src: [u8; 4], opacity: f32, mode: BlendMode) -> [u8; 4] {
    let opacity = (opacity.clamp(0., 1.) * 255.).round() as u8;
    let sa = (src[3] as u32 * opacity as u32 + 127) / 255;
    if sa == 0 {
        return dst;
    }
    if sa == 255 && mode == BlendMode::Normal {
        return [src[0], src[1], src[2], 255];
    }
    let da = dst[3] as u32;
    let oa = sa + (da * (255 - sa) + 127) / 255;
    if oa == 0 {
        return [0; 4];
    }
    let mut out = [0; 4];
    for c in 0..3 {
        let s = src[c] as u32;
        let d = dst[c] as u32;
        let b = match mode {
            BlendMode::Normal => s,
            BlendMode::Multiply => (s * d + 127) / 255,
            BlendMode::Screen => 255 - ((255 - s) * (255 - d) + 127) / 255,
        };
        let premul = b * sa * 255 + d * da * (255 - sa);
        out[c] = ((premul + oa * 127) / (oa * 255)).min(255) as u8
    }
    out[3] = oa as u8;
    out
}
fn integer_translation(m: Matrix4) -> bool {
    let a = m.0;
    a[0] == [1., 0., 0., a[0][3]]
        && a[1] == [0., 1., 0., a[1][3]]
        && a[3] == [0., 0., 0., 1.]
        && a[0][3].fract() == 0.
        && a[1][3].fract() == 0.
}
impl ReferenceCpuPresenter {
    fn draw_pass(
        &self,
        frame: &CompositorFrame,
        pass_id: usize,
        depth: usize,
    ) -> Result<Surface, PresenterError> {
        if depth > 16 {
            return Err(PresenterError::PassCycle);
        }
        let pass = frame
            .passes
            .get(pass_id)
            .ok_or(PresenterError::MissingPass(pass_id))?;
        let mut out = Surface::new(frame.width, frame.height);
        for quad in &pass.quads {
            match quad {
                DrawQuad::Tile(q) => {
                    if self.resources.get(q.resource_id).is_none() {
                        return Err(PresenterError::MissingResource(q.resource_id));
                    }
                    let local = intersect(q.rect, q.content_rect);
                    let Some(world) = q.transform.map_bounds(local) else {
                        continue;
                    };
                    let draw = intersect(
                        intersect(world, q.clip_rect),
                        Rect::new(0., 0., frame.width as f32, frame.height as f32),
                    );
                    for y in (draw.y.floor() as i64).max(0) as u32
                        ..((draw.y + draw.h).ceil() as i64).clamp(0, frame.height as i64) as u32
                    {
                        for x in (draw.x.floor() as i64).max(0) as u32
                            ..((draw.x + draw.w).ceil() as i64).clamp(0, frame.width as i64) as u32
                        {
                            let dx = x as f32 + 0.5;
                            let dy = y as f32 + 0.5;
                            if !frame.trees.visible(q.state, dx, dy) {
                                continue;
                            }
                            let Some((sx, sy)) = q.transform.unmap(dx, dy) else {
                                continue;
                            };
                            if !q.content_rect.contains(sx, sy)
                                || !q.rect.contains(sx.floor(), sy.floor())
                            {
                                continue;
                            }
                            let src = sample(
                                &self.resources,
                                frame,
                                q.layer,
                                sx,
                                sy,
                                integer_translation(q.transform),
                            );
                            if src[3] == 0 {
                                continue;
                            }
                            let dst = out.get_pixel(x, y).unwrap_or([0; 4]);
                            out.set_pixel(x, y, blend(dst, src, q.opacity, q.blend))
                        }
                    }
                }
                DrawQuad::SolidColor(q) => {
                    let draw = intersect(
                        intersect(q.rect, q.clip_rect),
                        Rect::new(0., 0., frame.width as f32, frame.height as f32),
                    );
                    for y in (draw.y.floor() as i64).max(0) as u32
                        ..((draw.y + draw.h).ceil() as i64).clamp(0, frame.height as i64) as u32
                    {
                        for x in (draw.x.floor() as i64).max(0) as u32
                            ..((draw.x + draw.w).ceil() as i64).clamp(0, frame.width as i64) as u32
                        {
                            if !draw.contains(x as f32 + 0.5, y as f32 + 0.5) {
                                continue;
                            }
                            let dst = out.get_pixel(x, y).unwrap_or([0; 4]);
                            out.set_pixel(x, y, blend(dst, q.color, q.opacity, q.blend))
                        }
                    }
                }
                DrawQuad::Pass(q) => {
                    let sub = self.draw_pass(frame, q.pass_id, depth + 1)?;
                    let draw = intersect(
                        intersect(q.rect, q.clip_rect),
                        Rect::new(0., 0., frame.width as f32, frame.height as f32),
                    );
                    for y in (draw.y.floor() as i64).max(0) as u32
                        ..((draw.y + draw.h).ceil() as i64).clamp(0, frame.height as i64) as u32
                    {
                        for x in (draw.x.floor() as i64).max(0) as u32
                            ..((draw.x + draw.w).ceil() as i64).clamp(0, frame.width as i64) as u32
                        {
                            let dx = x as f32 + 0.5;
                            let dy = y as f32 + 0.5;
                            if !frame.trees.visible(q.state, dx, dy) {
                                continue;
                            }
                            let src = sub.get_pixel(x, y).unwrap_or([0; 4]);
                            if src[3] == 0 {
                                continue;
                            }
                            let dst = out.get_pixel(x, y).unwrap_or([0; 4]);
                            out.set_pixel(x, y, blend(dst, src, q.opacity, q.blend))
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}
impl Presenter for ReferenceCpuPresenter {
    type Output = Surface;
    type Error = PresenterError;
    fn present(&mut self, frame: &CompositorFrame) -> Result<Surface, PresenterError> {
        self.resources.apply(&frame.resources)?;
        self.draw_pass(frame, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Colors(Cell<u8>);
    impl PaintSource for Colors {
        fn item_count(&self) -> usize {
            2
        }
        fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
            for i in items {
                let (r, color) = if i == 0 {
                    (Rect::new(0., 0., 40., 40.), [200, 20, 20, 255])
                } else {
                    (Rect::new(10., 10., 40., 40.), [20, 20, self.0.get(), 255])
                };
                out.fill_rect(r.x - tile.x, r.y - tile.y, r.w, r.h, color);
            }
        }
    }
    #[test]
    fn passes_resources_damage_and_rounded_clip_match_flattened() {
        let mut trees = PropertyTrees::new();
        let clip = trees
            .add_clip(
                ClipId(0),
                TransformId(0),
                ClipShape::RoundedRect {
                    rect: Rect::new(8., 8., 40., 40.),
                    radii: [8.; 4],
                },
            )
            .unwrap();
        let effect = trees
            .add_effect(EffectId(0), 0.5, BlendMode::Normal, None)
            .unwrap();
        let chunks = vec![
            PaintChunk {
                items: 0..1,
                state: PropertyTreeState::default(),
                bounds: Rect::new(0., 0., 40., 40.),
                reasons: CompositingReasons::WILL_CHANGE,
            },
            PaintChunk {
                items: 1..2,
                state: PropertyTreeState {
                    clip,
                    effect,
                    ..Default::default()
                },
                bounds: Rect::new(10., 10., 40., 40.),
                reasons: CompositingReasons::WILL_CHANGE,
            },
        ];
        let mut flat = Compositor::new(trees.clone(), chunks.clone(), 2).unwrap();
        let mut quad = Compositor::new(trees, chunks, 2).unwrap();
        let mut presenter = ReferenceCpuPresenter::default();
        let source = Colors(Cell::new(220));
        let first = quad.build_frame(&source, 64, 64).unwrap();
        assert_eq!(first.passes.len(), 2);
        assert_eq!(first.resources.uploads.len(), 2);
        assert_eq!(first.damage, vec![Rect::new(0., 0., 64., 64.)]);
        assert_eq!(
            presenter.present(&first).unwrap(),
            flat.render(&source, 64, 64).unwrap()
        );
        let second = quad.build_frame(&source, 64, 64).unwrap();
        assert!(second.resources.uploads.is_empty());
        assert!(second.damage.is_empty());
        source.0.set(190);
        quad.invalidate_chunk(1);
        flat.invalidate_chunk(1);
        let changed = quad.build_frame(&source, 64, 64).unwrap();
        assert_eq!(changed.resources.uploads.len(), 1);
        assert!(!changed.damage.is_empty());
        assert_eq!(
            presenter.present(&changed).unwrap(),
            flat.render(&source, 64, 64).unwrap()
        );
        let resized = quad.build_frame(&source, 50, 50).unwrap();
        assert_eq!(resized.damage, vec![Rect::new(0., 0., 50., 50.)]);
        assert_eq!(
            presenter.present(&resized).unwrap(),
            flat.render(&source, 50, 50).unwrap()
        );
    }
}
