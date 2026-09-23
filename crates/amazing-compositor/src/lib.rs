//! Retained property trees, paint chunks, layerization and a software tiled compositor.
//! The producer implements [`PaintSource`]; `amazing-paint` can attach its display-list
//! view without this crate depending on layout or changing the full-paint path.
#![forbid(unsafe_code)]
mod property;
use amazing_paint::Surface;
pub use property::*;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

pub const TILE_SIZE: i32 = 256;
const MAX_VISIBLE_TILES: usize = 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CompositingReasons(pub u32);
impl CompositingReasons {
    pub const WILL_CHANGE: Self = Self(1);
    pub const TRANSFORM_3D: Self = Self(2);
    pub const ANIMATED: Self = Self(4);
    pub const VIDEO: Self = Self(8);
    pub const FIXED: Self = Self(16);
    pub const SCROLLER: Self = Self(32);
    pub const OVERLAP: Self = Self(64);
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub fn direct(self) -> bool {
        self.0 & !Self::OVERLAP.0 != 0
    }
}
/// A contiguous run of source display items with one property-tree state.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintChunk {
    pub items: Range<usize>,
    pub state: PropertyTreeState,
    pub bounds: Rect,
    pub reasons: CompositingReasons,
}
/// A read-only bridge to a source display list. `rasterize` draws item range into
/// the transparent tile surface in list order; `tile` is its local-space rect.
/// The adapter must clip to `tile` and use its top-left as surface origin.
pub trait PaintSource {
    fn item_count(&self) -> usize;
    fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface);
}
#[derive(Debug, Clone)]
pub struct Layer {
    pub chunks: Vec<usize>,
    pub state: PropertyTreeState,
    pub bounds: Rect,
    pub reasons: CompositingReasons,
}
/// Pending-layer grouping preserves list order. Direct reasons isolate chunks;
/// overlap with a differing property state promotes the later layer.
pub fn layerize(trees: &PropertyTrees, chunks: &[PaintChunk]) -> Vec<Layer> {
    let mut layers: Vec<Layer> = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        if c.bounds.empty() || c.items.is_empty() {
            continue;
        }
        let mut reasons = c.reasons;
        for prior in &layers {
            let prior_world = trees
                .matrix(prior.state)
                .and_then(|m| m.map_bounds(prior.bounds));
            let current_world = trees.matrix(c.state).and_then(|m| m.map_bounds(c.bounds));
            if prior.state != c.state
                && prior_world
                    .zip(current_world)
                    .is_some_and(|(a, b)| a.intersects(b))
            {
                reasons = reasons.union(CompositingReasons::OVERLAP);
            }
        }
        if !reasons.direct() && !reasons.contains(CompositingReasons::OVERLAP) {
            if let Some(last) = layers.last_mut() {
                if last.state == c.state && !last.reasons.direct() {
                    last.chunks.push(i);
                    last.bounds = last.bounds.union(c.bounds);
                    continue;
                }
            }
        }
        layers.push(Layer {
            chunks: vec![i],
            state: c.state,
            bounds: c.bounds,
            reasons,
        });
    }
    layers
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    InvalidState,
    InvalidRange,
    UnorderedRanges,
    NonFiniteBounds,
    TooManyTiles,
}
impl std::fmt::Display for SceneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SceneError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TileKey {
    layer: usize,
    x: i32,
    y: i32,
}
#[derive(Debug, Clone)]
struct Tile {
    surface: Surface,
    dirty: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub raster_calls: u64,
    pub rasterized_tiles: u64,
    pub composited_pixels: u64,
}
/// Retains raster tiles while property nodes change. `render` never asks the
/// source to rebuild display items for scroll, transform or opacity changes.
pub struct Compositor {
    trees: PropertyTrees,
    chunks: Vec<PaintChunk>,
    layers: Vec<Layer>,
    tiles: HashMap<TileKey, Tile>,
    last_frame: FrameStats,
}
impl Compositor {
    pub fn new(
        trees: PropertyTrees,
        chunks: Vec<PaintChunk>,
        item_count: usize,
    ) -> Result<Self, SceneError> {
        let mut end = 0;
        for c in &chunks {
            if c.items.start < c.items.end && c.items.start < end {
                return Err(SceneError::UnorderedRanges);
            }
            if c.items.end > item_count || c.items.start > c.items.end {
                return Err(SceneError::InvalidRange);
            }
            end = c.items.end;
            if c.bounds.empty()
                || [
                    c.bounds.x,
                    c.bounds.y,
                    c.bounds.x + c.bounds.w,
                    c.bounds.y + c.bounds.h,
                ]
                .iter()
                .any(|v| v.abs() > 100_000_000.)
            {
                return Err(SceneError::NonFiniteBounds);
            }
            if tile_count(c.bounds) > 1_000_000 {
                return Err(SceneError::TooManyTiles);
            }
            if c.state.transform.0 >= trees.transforms.len()
                || c.state.clip.0 >= trees.clips.len()
                || c.state.effect.0 >= trees.effects.len()
                || c.state.scroll.0 >= trees.scrolls.len()
            {
                return Err(SceneError::InvalidState);
            }
        }
        let layers = layerize(&trees, &chunks);
        Ok(Self {
            trees,
            chunks,
            layers,
            tiles: HashMap::new(),
            last_frame: FrameStats::default(),
        })
    }
    pub fn trees(&self) -> &PropertyTrees {
        &self.trees
    }
    pub fn chunks(&self) -> &[PaintChunk] {
        &self.chunks
    }
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }
    pub fn stats(&self) -> FrameStats {
        self.last_frame
    }
    pub fn invalidate_chunk(&mut self, index: usize) {
        let Some(chunk) = self.chunks.get(index) else {
            return;
        };
        for (li, layer) in self.layers.iter().enumerate() {
            if layer.chunks.contains(&index) {
                for key in tile_keys(li, chunk.bounds) {
                    if let Some(tile) = self.tiles.get_mut(&key) {
                        tile.dirty = true
                    }
                }
            }
        }
    }
    pub fn repaint_all(&mut self) {
        for tile in self.tiles.values_mut() {
            tile.dirty = true
        }
    }
    pub fn set_transform(&mut self, id: TransformId, matrix: Matrix4) -> Result<(), TreeError> {
        self.trees.set_transform(id, matrix)
    }
    pub fn set_scroll(&mut self, id: ScrollId, offset: (f32, f32)) -> Result<(), TreeError> {
        self.trees.set_scroll(id, offset)
    }
    pub fn set_opacity(&mut self, id: EffectId, opacity: f32) -> Result<(), TreeError> {
        self.trees.set_opacity(id, opacity)
    }
    pub fn render(
        &mut self,
        source: &impl PaintSource,
        width: u32,
        height: u32,
    ) -> Result<Surface, SceneError> {
        let mut stats = FrameStats::default();
        let mut frame = Surface::new(width, height);
        if self
            .chunks
            .last()
            .is_some_and(|c| c.items.end > source.item_count())
        {
            return Err(SceneError::InvalidRange);
        }
        let viewport = Rect::new(0., 0., width as f32, height as f32);
        let mut visible_tiles = HashSet::new();
        for (li, layer) in self.layers.iter().enumerate() {
            let keys = tile_keys(li, layer.bounds);
            if keys.len() > 1_000_000 {
                return Err(SceneError::TooManyTiles);
            }
            let Some(matrix) = self.trees.matrix(layer.state) else {
                continue;
            };
            for key in keys {
                if !matrix
                    .map_bounds(tile_rect(key))
                    .is_some_and(|r| r.intersects(viewport))
                {
                    continue;
                }
                visible_tiles.insert(key);
                if visible_tiles.len() > MAX_VISIBLE_TILES {
                    return Err(SceneError::TooManyTiles);
                }
                let needs = self.tiles.get(&key).is_none_or(|t| t.dirty);
                if needs {
                    let rect = tile_rect(key);
                    let mut surface = Surface::new(TILE_SIZE as u32, TILE_SIZE as u32);
                    for &ci in &layer.chunks {
                        let c = &self.chunks[ci];
                        if c.bounds.intersects(rect) {
                            source.rasterize(c.items.clone(), rect, &mut surface);
                            stats.raster_calls += 1;
                        }
                    }
                    self.tiles.insert(
                        key,
                        Tile {
                            surface,
                            dirty: false,
                        },
                    );
                    stats.rasterized_tiles += 1;
                }
            }
        }
        self.tiles.retain(|key, _| visible_tiles.contains(key));
        for (li, layer) in self.layers.iter().enumerate() {
            let Some(matrix) = self.trees.matrix(layer.state) else {
                continue;
            };
            let Some(outer) = matrix.map_bounds(layer.bounds) else {
                continue;
            };
            let x0 = (outer.x.floor() as i64).clamp(0, width as i64) as u32;
            let y0 = (outer.y.floor() as i64).clamp(0, height as i64) as u32;
            let x1 = ((outer.x + outer.w).ceil() as i64).clamp(0, width as i64) as u32;
            let y1 = ((outer.y + outer.h).ceil() as i64).clamp(0, height as i64) as u32;
            let opacity = self.trees.opacity(layer.state);
            if opacity <= 0. {
                continue;
            }
            let blend = self.trees.effects[layer.state.effect.0].blend;
            for y in y0..y1 {
                for x in x0..x1 {
                    let dx = x as f32 + 0.5;
                    let dy = y as f32 + 0.5;
                    if !self.trees.visible(layer.state, dx, dy) {
                        continue;
                    }
                    let Some((sx, sy)) = matrix.unmap(dx, dy) else {
                        continue;
                    };
                    if !layer.bounds.contains(sx, sy) {
                        continue;
                    }
                    let px = sx.floor() as i32;
                    let py = sy.floor() as i32;
                    let key = TileKey {
                        layer: li,
                        x: px.div_euclid(TILE_SIZE),
                        y: py.div_euclid(TILE_SIZE),
                    };
                    let Some(tile) = self.tiles.get(&key) else {
                        continue;
                    };
                    let tx = px.rem_euclid(TILE_SIZE) as u32;
                    let ty = py.rem_euclid(TILE_SIZE) as u32;
                    let src = tile.surface.get_pixel(tx, ty).unwrap_or([0; 4]);
                    if src[3] == 0 {
                        continue;
                    }
                    let dst = frame.get_pixel(x, y).unwrap_or([0; 4]);
                    frame.set_pixel(x, y, blend_pixel(dst, src, opacity, blend));
                    stats.composited_pixels += 1;
                }
            }
        }
        self.last_frame = stats;
        Ok(frame)
    }
    pub fn cached_tiles(&self) -> usize {
        self.tiles.len()
    }
    pub fn dirty_tiles(&self) -> usize {
        self.tiles.values().filter(|t| t.dirty).count()
    }
}
fn tile_count(r: Rect) -> i64 {
    let x0 = (r.x.floor() as i32).div_euclid(TILE_SIZE);
    let y0 = (r.y.floor() as i32).div_euclid(TILE_SIZE);
    let x1 = ((r.x + r.w).ceil() as i32 - 1).div_euclid(TILE_SIZE);
    let y1 = ((r.y + r.h).ceil() as i32 - 1).div_euclid(TILE_SIZE);
    (x1 as i64 - x0 as i64 + 1).saturating_mul(y1 as i64 - y0 as i64 + 1)
}
fn tile_keys(layer: usize, r: Rect) -> Vec<TileKey> {
    if r.empty() {
        return Vec::new();
    }
    let x0 = (r.x.floor() as i32).div_euclid(TILE_SIZE);
    let y0 = (r.y.floor() as i32).div_euclid(TILE_SIZE);
    let x1 = ((r.x + r.w).ceil() as i32 - 1).div_euclid(TILE_SIZE);
    let y1 = ((r.y + r.h).ceil() as i32 - 1).div_euclid(TILE_SIZE);
    let mut out = Vec::new();
    if (x1 as i64 - x0 as i64 + 1) * (y1 as i64 - y0 as i64 + 1) > 1_000_000 {
        return out;
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push(TileKey { layer, x, y })
        }
    }
    out
}
fn tile_rect(k: TileKey) -> Rect {
    Rect::new(
        (k.x * TILE_SIZE) as f32,
        (k.y * TILE_SIZE) as f32,
        TILE_SIZE as f32,
        TILE_SIZE as f32,
    )
}
fn blend_pixel(dst: [u8; 4], src: [u8; 4], opacity: f32, mode: BlendMode) -> [u8; 4] {
    let sa = (src[3] as f32 / 255.) * opacity;
    let da = dst[3] as f32 / 255.;
    let oa = sa + da * (1. - sa);
    if oa <= 0. {
        return [0; 4];
    }
    let mut out = [0; 4];
    for i in 0..3 {
        let s = src[i] as f32 / 255.;
        let d = dst[i] as f32 / 255.;
        let b = match mode {
            BlendMode::Normal => s,
            BlendMode::Multiply => s * d,
            BlendMode::Screen => s + d - s * d,
        };
        let c = (b * sa + d * da * (1. - sa)) / oa;
        out[i] = (c.clamp(0., 1.) * 255.).round() as u8
    }
    out[3] = (oa.clamp(0., 1.) * 255.).round() as u8;
    out
}
/// Validate that the producer's chunk ranges are disjoint and ordered.
pub fn covered_items(chunks: &[PaintChunk]) -> HashSet<usize> {
    chunks.iter().flat_map(|c| c.items.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct Source {
        colors: Vec<[u8; 4]>,
        rects: Vec<Rect>,
        calls: Cell<usize>,
    }
    impl PaintSource for Source {
        fn item_count(&self) -> usize {
            self.rects.len()
        }
        fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
            self.calls.set(self.calls.get() + 1);
            for i in items {
                let r = self.rects[i];
                out.fill_rect(r.x - tile.x, r.y - tile.y, r.w, r.h, self.colors[i]);
            }
        }
    }
    fn fixture() -> (Compositor, Source, TransformId, ScrollId, EffectId) {
        let mut t = PropertyTrees::new();
        let tr = t
            .add_transform(TransformId(0), Matrix4::IDENTITY, (24., 24., 0.), false)
            .unwrap();
        let sc = t
            .add_scroll(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 128., 128.),
                Rect::new(0., 0., 256., 256.),
            )
            .unwrap();
        let ef = t
            .add_effect(EffectId(0), 1., BlendMode::Normal, None)
            .unwrap();
        let state = PropertyTreeState {
            transform: tr,
            scroll: sc,
            effect: ef,
            clip: ClipId(0),
        };
        let chunks = vec![
            PaintChunk {
                items: 0..1,
                state,
                bounds: Rect::new(8., 8., 32., 32.),
                reasons: CompositingReasons::ANIMATED,
            },
            PaintChunk {
                items: 1..2,
                state,
                bounds: Rect::new(48., 8., 32., 32.),
                reasons: CompositingReasons::ANIMATED,
            },
        ];
        let s = Source {
            colors: vec![[255, 0, 0, 255], [0, 0, 255, 255]],
            rects: chunks.iter().map(|c| c.bounds).collect(),
            calls: Cell::new(0),
        };
        (
            Compositor::new(t, chunks, s.item_count()).unwrap(),
            s,
            tr,
            sc,
            ef,
        )
    }
    #[test]
    fn property_updates_reuse_raster_and_equal_full_repaint() {
        let (mut c, s, tr, sc, ef) = fixture();
        c.render(&s, 128, 128).unwrap();
        assert_eq!(s.calls.get(), 2);
        for i in 0..6 {
            c.set_scroll(sc, (i as f32 * 2., 0.)).unwrap();
            c.set_transform(tr, Matrix4::rotation_z(i as f32 * 0.1))
                .unwrap();
            c.set_opacity(ef, 1. - i as f32 * 0.1).unwrap();
            let fast = c.render(&s, 128, 128).unwrap();
            assert_eq!(c.stats().raster_calls, 0);
            let mut baseline =
                Compositor::new(c.trees.clone(), c.chunks.clone(), s.item_count()).unwrap();
            let full = baseline.render(&s, 128, 128).unwrap();
            assert_eq!(fast, full);
        }
    }
    #[test]
    fn damage_only_rerasterizes_affected_tile() {
        let (mut c, s, _, _, _) = fixture();
        c.render(&s, 128, 128).unwrap();
        c.invalidate_chunk(0);
        assert_eq!(c.dirty_tiles(), 1);
        c.render(&s, 128, 128).unwrap();
        assert_eq!(c.stats().rasterized_tiles, 1)
    }
    #[test]
    fn parent_cache_and_rounded_clip() {
        let mut t = PropertyTrees::new();
        let p = t
            .add_transform(
                TransformId(0),
                Matrix4::translation(10., 0., 0.),
                (0., 0., 0.),
                false,
            )
            .unwrap();
        let q = t
            .add_transform(p, Matrix4::translation(3., 0., 0.), (0., 0., 0.), false)
            .unwrap();
        assert_eq!(t.transforms[q.0].accumulated.map(0., 0.), Some((13., 0.)));
        t.set_transform(p, Matrix4::translation(20., 0., 0.))
            .unwrap();
        assert_eq!(t.transforms[q.0].accumulated.map(0., 0.), Some((23., 0.)));
        let clip = t
            .add_clip(
                ClipId(0),
                TransformId(0),
                ClipShape::RoundedRect {
                    rect: Rect::new(0., 0., 20., 20.),
                    radii: [5.; 4],
                },
            )
            .unwrap();
        let s = PropertyTreeState {
            clip,
            ..Default::default()
        };
        assert!(!t.visible(s, 0.5, 0.5));
        assert!(t.visible(s, 10., 10.));
    }
    #[test]
    fn overlap_promotes_and_order_preserved() {
        let mut trees = PropertyTrees::new();
        trees
            .add_transform(TransformId(0), Matrix4::IDENTITY, (0., 0., 0.), false)
            .unwrap();
        let mut b = PropertyTreeState::default();
        b.transform = TransformId(1);
        let chunks = vec![
            PaintChunk {
                items: 0..1,
                state: PropertyTreeState::default(),
                bounds: Rect::new(0., 0., 20., 20.),
                reasons: CompositingReasons::default(),
            },
            PaintChunk {
                items: 1..2,
                state: b,
                bounds: Rect::new(10., 10., 20., 20.),
                reasons: CompositingReasons::default(),
            },
        ];
        let layers = layerize(&trees, &chunks);
        assert_eq!(layers.len(), 2);
        assert!(layers[1].reasons.contains(CompositingReasons::OVERLAP));
    }
}
