//! Damage-driven composition. All rects are conservative device-pixel bounds.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LayerSnapshot {
    matrix: Matrix4,
    opacity: f32,
    blend: BlendMode,
    bounds: Rect,
    scroll: ScrollId,
}
#[derive(Clone, Copy, Debug)]
struct PixelRect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}
impl PixelRect {
    fn viewport(w: u32, h: u32) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: w,
            y1: h,
        }
    }
    fn from_rect(r: Rect, w: u32, h: u32) -> Self {
        Self {
            x0: (r.x.floor() as i64).clamp(0, w as i64) as u32,
            y0: (r.y.floor() as i64).clamp(0, h as i64) as u32,
            x1: ((r.x + r.w).ceil() as i64).clamp(0, w as i64) as u32,
            y1: ((r.y + r.h).ceil() as i64).clamp(0, h as i64) as u32,
        }
    }
    fn empty(self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
    fn intersection(self, b: Self) -> Self {
        Self {
            x0: self.x0.max(b.x0),
            y0: self.y0.max(b.y0),
            x1: self.x1.min(b.x1),
            y1: self.y1.min(b.y1),
        }
    }
    fn union(self, b: Self) -> Self {
        Self {
            x0: self.x0.min(b.x0),
            y0: self.y0.min(b.y0),
            x1: self.x1.max(b.x1),
            y1: self.y1.max(b.y1),
        }
    }
    fn touches(self, b: Self) -> bool {
        self.x0 <= b.x1 && b.x0 <= self.x1 && self.y0 <= b.y1 && b.y0 <= self.y1
    }
    fn area(self) -> u64 {
        (self.x1 - self.x0) as u64 * (self.y1 - self.y0) as u64
    }
}
fn add_damage(rects: &mut Vec<PixelRect>, mut r: PixelRect) {
    if r.empty() {
        return;
    }
    let mut i = 0;
    while i < rects.len() {
        if r.touches(rects[i]) {
            r = r.union(rects.swap_remove(i));
            i = 0
        } else {
            i += 1
        }
    }
    rects.push(r)
}
fn integer_shift(
    old: &[LayerSnapshot],
    new: &[LayerSnapshot],
    trees: &PropertyTrees,
    w: u32,
    h: u32,
) -> Option<(i32, i32)> {
    if old.len() != new.len() || new.is_empty() {
        return None;
    }
    let mut shift = None;
    for (a, b) in old.iter().zip(new) {
        if a.scroll.0 == 0 || a.scroll != b.scroll || a.opacity != b.opacity || a.blend != b.blend {
            return None;
        }
        let scroll = trees.scroll(a.scroll)?;
        if !scroll.container.contains(0., 0.)
            || !scroll.container.contains(w as f32 - 0.5, h as f32 - 0.5)
        {
            return None;
        }
        let am = a.matrix.0;
        let bm = b.matrix.0;
        for i in 0..4 {
            for j in 0..4 {
                if (i == 0 || i == 1) && j == 3 {
                    continue;
                }
                if am[i][j] != bm[i][j] {
                    return None;
                }
            }
        }
        let dx = bm[0][3] - am[0][3];
        let dy = bm[1][3] - am[1][3];
        if !dx.is_finite() || !dy.is_finite() || dx.fract() != 0. || dy.fract() != 0. {
            return None;
        }
        let pair = (dx as i32, dy as i32);
        if shift.is_some_and(|s| s != pair) {
            return None;
        }
        shift = Some(pair)
    }
    shift.filter(|s| s.0 != 0 || s.1 != 0)
}
fn blit_shift(old: &Surface, new: &mut Surface, dx: i32, dy: i32) -> Vec<PixelRect> {
    let w = old.width as i32;
    let h = old.height as i32;
    let x0 = dx.max(0).min(w);
    let y0 = dy.max(0).min(h);
    let x1 = (w + dx).max(0).min(w);
    let y1 = (h + dy).max(0).min(h);
    let mut damage = Vec::new();
    if x0 >= x1 || y0 >= y1 {
        damage.push(PixelRect::viewport(old.width, old.height));
        return damage;
    }
    for y in y0..y1 {
        let src_y = (y - dy) as usize;
        let dst_y = y as usize;
        let src = ((src_y * w as usize) + (x0 - dx) as usize) * 4;
        let dst = ((dst_y * w as usize) + x0 as usize) * 4;
        let len = (x1 - x0) as usize * 4;
        new.pixels[dst..dst + len].copy_from_slice(&old.pixels[src..src + len])
    }
    let (w, h) = (old.width, old.height);
    for r in [
        PixelRect {
            x0: 0,
            y0: 0,
            x1: w,
            y1: y0 as u32,
        },
        PixelRect {
            x0: 0,
            y0: y1 as u32,
            x1: w,
            y1: h,
        },
        PixelRect {
            x0: 0,
            y0: y0 as u32,
            x1: x0 as u32,
            y1: y1 as u32,
        },
        PixelRect {
            x0: x1 as u32,
            y0: y0 as u32,
            x1: w,
            y1: y1 as u32,
        },
    ] {
        if !r.empty() {
            damage.push(r)
        }
    }
    damage
}
fn alpha_over(dst: [u8; 4], src: [u8; 4], opacity: u8, mode: BlendMode) -> [u8; 4] {
    super::effect_pixels::blend(dst, src, f32::from(opacity) / 255., mode.into())
}

fn tile_pixel(tiles: &HashMap<TileKey, Tile>, layer: usize, x: i32, y: i32) -> [u8; 4] {
    let key = TileKey {
        layer,
        x: x.div_euclid(TILE_SIZE),
        y: y.div_euclid(TILE_SIZE),
    };
    let Some(t) = tiles.get(&key) else {
        return [0; 4];
    };
    t.surface
        .get_pixel(
            x.rem_euclid(TILE_SIZE) as u32,
            y.rem_euclid(TILE_SIZE) as u32,
        )
        .unwrap_or([0; 4])
}
/// Bilinear interpolation is performed in premultiplied RGBA; each channel is
/// rounded once to RGBA8. Expected transformed-edge tolerance is one channel.
fn sample(
    tiles: &HashMap<TileKey, Tile>,
    layer: usize,
    sx: f32,
    sy: f32,
    integer: bool,
) -> [u8; 4] {
    if integer {
        return tile_pixel(tiles, layer, sx.floor() as i32, sy.floor() as i32);
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
        tile_pixel(tiles, layer, x, y),
        tile_pixel(tiles, layer, x + 1, y),
        tile_pixel(tiles, layer, x, y + 1),
        tile_pixel(tiles, layer, x + 1, y + 1),
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
fn is_integer_translation(m: Matrix4) -> bool {
    let a = m.0;
    a[0] == [1., 0., 0., a[0][3]]
        && a[1] == [0., 1., 0., a[1][3]]
        && a[3] == [0., 0., 0., 1.]
        && a[0][3].fract() == 0.
        && a[1][3].fract() == 0.
}
impl Compositor {
    pub fn render(
        &mut self,
        source: &impl PaintSource,
        width: u32,
        height: u32,
    ) -> Result<Surface, SceneError> {
        if self.trees.has_composited_effects() {
            let mut frame = self.build_frame(source, width, height)?;
            frame.resources = self.resource_snapshot();
            return ReferenceCpuPresenter::default()
                .present(&frame)
                .map_err(|_| SceneError::InvalidState);
        }
        if self
            .chunks
            .last()
            .is_some_and(|c| c.items.end > source.item_count())
        {
            return Err(SceneError::InvalidRange);
        }
        let viewport = Rect::new(0., 0., width as f32, height as f32);
        let mut stats = FrameStats::default();
        let snapshots: Vec<_> = self
            .layers
            .iter()
            .map(|l| {
                let m = self.trees.matrix(l.state).unwrap_or(Matrix4::IDENTITY);
                LayerSnapshot {
                    matrix: m,
                    opacity: self.trees.opacity(l.state),
                    blend: self.trees.effects[l.state.effect.0].blend,
                    bounds: m.map_bounds(l.bounds).unwrap_or_default(),
                    scroll: l.state.scroll,
                }
            })
            .collect();
        let mut tile_damage = Vec::new();
        let mut dirty_existing_tile = false;
        let mut visible_tiles = HashSet::new();
        for (li, layer) in self.layers.iter().enumerate() {
            let keys = tile_keys(li, layer.bounds);
            if keys.len() > 1_000_000 {
                return Err(SceneError::TooManyTiles);
            }
            let matrix = snapshots[li].matrix;
            for key in keys {
                let rect = tile_rect(key);
                if !matrix
                    .map_bounds(rect)
                    .is_some_and(|r| r.intersects(viewport))
                {
                    continue;
                }
                visible_tiles.insert(key);
                if visible_tiles.len() > MAX_VISIBLE_TILES {
                    return Err(SceneError::TooManyTiles);
                }
                if self.tiles.get(&key).is_none_or(|t| t.dirty) {
                    dirty_existing_tile |= self.tiles.get(&key).is_some_and(|t| t.dirty);
                    let mut surface = self
                        .free_tile_surfaces
                        .pop()
                        .unwrap_or_else(|| Surface::new(TILE_SIZE as u32, TILE_SIZE as u32));
                    surface.clear([0, 0, 0, 0]);
                    for &ci in &layer.chunks {
                        let c = &self.chunks[ci];
                        if c.bounds.intersects(rect) {
                            source.rasterize(c.items.clone(), rect, &mut surface);
                            stats.raster_calls += 1
                        }
                    }
                    install_rasterized_tile(
                        &mut self.tiles,
                        &mut self.free_tile_surfaces,
                        self.tile_budget,
                        &mut self.tile_clock,
                        key,
                        surface,
                    );
                    stats.rasterized_tiles += 1;
                }
                if self.tiles[&key].updated {
                    if let Some(r) = matrix.map_bounds(rect) {
                        add_damage(&mut tile_damage, PixelRect::from_rect(r, width, height))
                    }
                    self.tiles.get_mut(&key).unwrap().updated = false;
                }
                self.tile_clock = self.tile_clock.wrapping_add(1);
                self.tiles.get_mut(&key).unwrap().last_used = self.tile_clock;
            }
        }
        evict_tile_cache(
            &mut self.tiles,
            &mut self.free_tile_surfaces,
            self.tile_budget,
            &visible_tiles,
        );
        let prior = self
            .previous_frame
            .as_ref()
            .filter(|s| s.width == width && s.height == height);
        let mut damage = Vec::new();
        let mut frame;
        let mut shifted = false;
        if let Some((dx, dy)) = prior.and_then(|_| {
            if !dirty_existing_tile {
                integer_shift(
                    &self.previous_layers,
                    &snapshots,
                    &self.trees,
                    width,
                    height,
                )
            } else {
                None
            }
        }) {
            frame = Surface::new(width, height);
            damage = blit_shift(prior.unwrap(), &mut frame, dx, dy);
            shifted = true;
            stats.blitted_pixels =
                (width as u64 * height as u64) - damage.iter().map(|r| r.area()).sum::<u64>();
        } else if let Some(old) = prior {
            frame = old.clone();
            for (a, b) in self.previous_layers.iter().zip(&snapshots) {
                if a != b {
                    add_damage(&mut damage, PixelRect::from_rect(a.bounds, width, height));
                    add_damage(&mut damage, PixelRect::from_rect(b.bounds, width, height))
                }
            }
            if self.previous_layers.len() != snapshots.len() {
                damage.clear();
                damage.push(PixelRect::viewport(width, height))
            }
        } else {
            frame = Surface::new(width, height);
            damage.push(PixelRect::viewport(width, height))
        }
        if !shifted {
            for r in tile_damage {
                add_damage(&mut damage, r)
            }
        }
        for r in damage {
            stats.damaged_pixels += r.area();
            self.paint_rect(&mut frame, r, &snapshots, &mut stats)
        }
        self.previous_frame = Some(frame.clone());
        self.previous_layers = snapshots;
        self.last_frame = stats;
        Ok(frame)
    }
    fn paint_rect(
        &self,
        frame: &mut Surface,
        r: PixelRect,
        layers: &[LayerSnapshot],
        stats: &mut FrameStats,
    ) {
        for y in r.y0..r.y1 {
            let begin = ((y * frame.width + r.x0) * 4) as usize;
            let end = ((y * frame.width + r.x1) * 4) as usize;
            frame.pixels[begin..end].fill(0)
        }
        for (li, layer) in self.layers.iter().enumerate() {
            let snap = layers[li];
            if snap.opacity <= 0. {
                continue;
            }
            let draw = r.intersection(PixelRect::from_rect(snap.bounds, frame.width, frame.height));
            if draw.empty() {
                continue;
            }
            let integer = is_integer_translation(snap.matrix);
            let opacity = amazing_paint::quantize_opacity(snap.opacity);
            for y in draw.y0..draw.y1 {
                for x in draw.x0..draw.x1 {
                    let dx = x as f32 + 0.5;
                    let dy = y as f32 + 0.5;
                    if !self.trees.visible(layer.state, dx, dy) {
                        continue;
                    }
                    let Some((sx, sy)) = snap.matrix.unmap(dx, dy) else {
                        continue;
                    };
                    if !layer.bounds.contains(sx, sy) {
                        continue;
                    }
                    let src = sample(&self.tiles, li, sx, sy, integer);
                    if src[3] == 0 {
                        continue;
                    }
                    let dst = frame.get_pixel(x, y).unwrap_or([0; 4]);
                    frame.set_pixel(x, y, alpha_over(dst, src, opacity, snap.blend));
                    stats.composited_pixels += 1
                }
            }
        }
    }
}

#[cfg(test)]
mod blend_tests {
    use super::{alpha_over, BlendMode};

    #[test]
    fn retained_frame_uses_paint_integer_source_over() {
        assert_eq!(
            alpha_over([191, 191, 191, 255], [0, 0, 0, 255], 128, BlendMode::Normal),
            [95, 95, 95, 255],
        );
    }
}
