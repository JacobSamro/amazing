//! Viewport-prioritized tile preparation. Workers only read an immutable paint
//! source; results are installed in priority order after every worker joins.
use super::*;
use crate::quad::clip_rect;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileRasterMode {
    /// Stable job order and one worker for pixel oracles.
    Deterministic,
    /// Zero selects all available cores except the presenting core.
    Parallel { workers: usize },
}

#[derive(Debug, Clone, Copy)]
pub struct TileRasterRequest {
    pub width: u32,
    pub height: u32,
    /// Direction of the scroll offset in device pixels. A viewport is
    /// prepainted ahead of scrolling whenever either component is nonzero.
    pub scroll_direction: (f32, f32),
    pub mode: TileRasterMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TileRasterBatch {
    pub visible_tiles: usize,
    pub prepaint_tiles: usize,
    pub rasterized_tiles: usize,
    pub workers: usize,
}

struct TileJob {
    key: TileKey,
    local: Rect,
    ranges: Vec<Range<usize>>,
    surface: Surface,
    visible: bool,
}

fn center_distance(rect: Rect, viewport: Rect) -> u64 {
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let vx = viewport.x + viewport.w * 0.5;
    let vy = viewport.y + viewport.h * 0.5;
    ((cx - vx).powi(2) + (cy - vy).powi(2)).max(0.) as u64
}

impl Compositor {
    fn collect_tile_jobs(
        &mut self,
        request: TileRasterRequest,
    ) -> Result<(Vec<TileJob>, HashSet<TileKey>), SceneError> {
        let halo = self.effect_raster_halo();
        let viewport = Rect::new(
            -halo,
            -halo,
            request.width as f32 + 2. * halo,
            request.height as f32 + 2. * halo,
        );
        let direction = request.scroll_direction;
        let ahead = Rect::new(
            viewport.x
                + if direction.0 == 0. {
                    0.
                } else {
                    direction.0.signum() * viewport.w
                },
            viewport.y
                + if direction.1 == 0. {
                    0.
                } else {
                    direction.1.signum() * viewport.h
                },
            viewport.w,
            viewport.h,
        );
        let prepaint = direction.0 != 0. || direction.1 != 0.;
        let mut candidates = Vec::new();
        let mut pinned = HashSet::new();
        for (li, layer) in self.layers.iter().enumerate() {
            let matrix = self.trees.matrix(layer.state).unwrap_or(Matrix4::IDENTITY);
            let scissor = if self.trees.has_composited_effects() || self.reserved_effect_halo > 0. {
                viewport
            } else {
                clip_rect(&self.trees, layer.state, viewport)
            };
            for key in tile_keys(li, layer.bounds) {
                let local = tile_rect(key);
                let Some(device) = matrix.map_bounds(local) else {
                    continue;
                };
                let visible = device.intersects(scissor);
                if visible {
                    pinned.insert(key);
                }
                if !visible && !(prepaint && device.intersects(ahead)) {
                    continue;
                }
                if self.tiles.get(&key).is_some_and(|tile| !tile.dirty) {
                    continue;
                }
                let ranges = layer
                    .chunks
                    .iter()
                    .filter_map(|&ci| {
                        let chunk = &self.chunks[ci];
                        chunk.bounds.intersects(local).then(|| chunk.items.clone())
                    })
                    .collect();
                candidates.push((
                    key,
                    local,
                    ranges,
                    visible,
                    center_distance(device, if visible { viewport } else { ahead }),
                ));
            }
        }
        if candidates.len() > MAX_VISIBLE_TILES * 4 {
            return Err(SceneError::TooManyTiles);
        }
        if pinned.len() > MAX_VISIBLE_TILES {
            return Err(SceneError::TooManyTiles);
        }
        candidates.sort_by_key(|(key, _, _, visible, distance)| {
            (!*visible, *distance, key.layer, key.y, key.x)
        });
        candidates.truncate(self.tile_budget.max(pinned.len()));
        let mut jobs = Vec::with_capacity(candidates.len());
        for (key, local, ranges, visible, _) in candidates {
            let surface = self
                .free_tile_surfaces
                .pop()
                .unwrap_or_else(|| Surface::new(TILE_SIZE as u32, TILE_SIZE as u32));
            jobs.push(TileJob {
                key,
                local,
                ranges,
                surface,
                visible,
            });
        }
        Ok((jobs, pinned))
    }
    /// Prepare visible tiles first, then one viewport ahead of the scroll.
    /// Existing tiles stay resident until their replacement has completed,
    /// so a presenter can continue using the previous frame while this runs.
    /// Parallel mode requires a thread-safe, immutable paint source.
    pub fn prepare_tiles<S: PaintSource + Sync>(
        &mut self,
        source: &S,
        request: TileRasterRequest,
    ) -> Result<TileRasterBatch, SceneError> {
        if self
            .chunks
            .last()
            .is_some_and(|c| c.items.end > source.item_count())
        {
            return Err(SceneError::InvalidRange);
        }
        let (mut jobs, pinned) = self.collect_tile_jobs(request)?;
        let worker_count = match request.mode {
            TileRasterMode::Deterministic => 1,
            TileRasterMode::Parallel { workers: 0 } => {
                std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1))
            }
            TileRasterMode::Parallel { workers } => workers.max(1),
        }
        .min(jobs.len().max(1));
        let chunk_size = jobs.len().div_ceil(worker_count).max(1);
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for batch in jobs.chunks_mut(chunk_size) {
                handles.push(scope.spawn(move || {
                    for job in batch {
                        job.surface.clear([0, 0, 0, 0]);
                        for range in &job.ranges {
                            source.rasterize(range.clone(), job.local, &mut job.surface);
                        }
                    }
                }));
            }
            for handle in handles {
                handle.join().expect("tile raster worker panicked");
            }
        });
        let mut result = TileRasterBatch {
            workers: worker_count,
            ..TileRasterBatch::default()
        };
        for job in jobs {
            result.rasterized_tiles += 1;
            if job.visible {
                result.visible_tiles += 1
            } else {
                result.prepaint_tiles += 1
            }
            install_rasterized_tile(
                &mut self.tiles,
                &mut self.free_tile_surfaces,
                self.tile_budget,
                &mut self.tile_clock,
                job.key,
                job.surface,
            );
        }
        evict_tile_cache(
            &mut self.tiles,
            &mut self.free_tile_surfaces,
            self.tile_budget,
            &pinned,
        );
        Ok(result)
    }

    /// Prepare in parallel and then build the retained quad frame. Tests can
    /// choose deterministic mode while production uses N-1 worker cores.
    pub fn build_parallel_frame<S: PaintSource + Sync>(
        &mut self,
        source: &S,
        request: TileRasterRequest,
    ) -> Result<(CompositorFrame, TileRasterBatch), SceneError> {
        let batch = self.prepare_tiles(source, request)?;
        let frame = self.build_frame(source, request.width, request.height)?;
        Ok((frame, batch))
    }
}

struct PoolWork {
    job: TileJob,
    source: Arc<dyn PaintSource + Send + Sync>,
    epoch: u64,
}
enum PoolMessage {
    Raster(PoolWork),
    Stop,
}
struct PoolResult {
    job: TileJob,
    epoch: u64,
}

/// Persistent N-1-core raster workers for owned RC-20 paint snapshots. The
/// presenter may continue showing its last complete frame while jobs run.
pub struct TileRasterPool {
    sender: mpsc::Sender<PoolMessage>,
    completed: mpsc::Receiver<PoolResult>,
    workers: Vec<std::thread::JoinHandle<()>>,
    pending: HashSet<(TileKey, u64)>,
    cancelled: Arc<AtomicBool>,
}

impl TileRasterPool {
    pub fn new(workers: usize) -> Self {
        let count = if workers == 0 {
            std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1))
        } else {
            workers
        };
        let (sender, receive) = mpsc::channel::<PoolMessage>();
        let receive = Arc::new(Mutex::new(receive));
        let (complete, completed) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::with_capacity(count);
        for _ in 0..count {
            let receive = Arc::clone(&receive);
            let complete = complete.clone();
            let cancelled = Arc::clone(&cancelled);
            handles.push(std::thread::spawn(move || loop {
                let message = receive.lock().unwrap().recv();
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(PoolMessage::Raster(mut work)) = message else {
                    break;
                };
                work.job.surface.clear([0, 0, 0, 0]);
                for range in &work.job.ranges {
                    work.source
                        .rasterize(range.clone(), work.job.local, &mut work.job.surface);
                }
                if complete
                    .send(PoolResult {
                        job: work.job,
                        epoch: work.epoch,
                    })
                    .is_err()
                {
                    break;
                }
            }));
        }
        Self {
            sender,
            completed,
            workers: handles,
            pending: HashSet::new(),
            cancelled,
        }
    }

    pub fn pending_tiles(&self) -> usize {
        self.pending.len()
    }
}

impl Drop for TileRasterPool {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        for _ in &self.workers {
            let _ = self.sender.send(PoolMessage::Stop);
        }
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }
    }
}

impl Compositor {
    /// Queue visible tiles before prepaint tiles. A pool job owns its paint
    /// snapshot and surface, so the main thread can return before raster ends.
    pub fn schedule_tiles(
        &mut self,
        pool: &mut TileRasterPool,
        source: Arc<dyn PaintSource + Send + Sync>,
        request: TileRasterRequest,
    ) -> Result<usize, SceneError> {
        if self
            .chunks
            .last()
            .is_some_and(|c| c.items.end > source.item_count())
        {
            return Err(SceneError::InvalidRange);
        }
        let (jobs, _) = self.collect_tile_jobs(request)?;
        let mut scheduled = 0;
        for job in jobs {
            if pool.pending.len() >= self.tile_budget.saturating_mul(2).max(1) {
                break;
            }
            let key = (job.key, self.raster_epoch);
            if !pool.pending.insert(key) {
                if self.free_tile_surfaces.len() < self.tile_budget.saturating_sub(self.tiles.len())
                {
                    self.free_tile_surfaces.push(job.surface);
                }
                continue;
            }
            pool.sender
                .send(PoolMessage::Raster(PoolWork {
                    job,
                    source: Arc::clone(&source),
                    epoch: self.raster_epoch,
                }))
                .expect("tile raster pool closed while scheduling");
            scheduled += 1;
        }
        Ok(scheduled)
    }

    /// Install completed tiles only if their paint generation is still live.
    /// Old tile pixels remain available until this atomic replacement step.
    pub fn collect_ready_tiles(&mut self, pool: &mut TileRasterPool) -> usize {
        let mut installed = 0;
        while let Ok(result) = pool.completed.try_recv() {
            pool.pending.remove(&(result.job.key, result.epoch));
            if result.epoch == self.raster_epoch {
                install_rasterized_tile(
                    &mut self.tiles,
                    &mut self.free_tile_surfaces,
                    self.tile_budget,
                    &mut self.tile_clock,
                    result.job.key,
                    result.job.surface,
                );
                installed += 1;
            } else if self.free_tile_surfaces.len()
                < self.tile_budget.saturating_sub(self.tiles.len())
            {
                self.free_tile_surfaces.push(result.job.surface);
            }
        }
        installed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Article(AtomicUsize);
    impl PaintSource for Article {
        fn item_count(&self) -> usize {
            1
        }
        fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
            self.0.fetch_add(1, Ordering::Relaxed);
            let row = (tile.y as i32 / TILE_SIZE) as u8;
            out.clear([row.wrapping_mul(23), row.wrapping_mul(17), 80, 255]);
        }
    }

    fn article() -> (Compositor, ScrollId) {
        let mut trees = PropertyTrees::new();
        let scroll = trees
            .add_scroll(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 256., 256.),
                Rect::new(0., 0., 256., 2048.),
            )
            .unwrap();
        let state = PropertyTreeState {
            scroll,
            ..PropertyTreeState::default()
        };
        let scene = Compositor::new(
            trees,
            vec![PaintChunk {
                items: 0..1,
                state,
                bounds: Rect::new(0., 0., 256., 2048.),
                reasons: CompositingReasons::WILL_CHANGE,
            }],
            1,
        )
        .unwrap();
        (scene, scroll)
    }

    fn request(mode: TileRasterMode) -> TileRasterRequest {
        TileRasterRequest {
            width: 256,
            height: 256,
            scroll_direction: (0., 1.),
            mode,
        }
    }

    #[test]
    fn prepaint_and_lru_keep_reverse_scroll_cached_without_checkerboard() {
        let (mut scene, scroll) = article();
        scene.set_tile_budget(3);
        let source = Article(AtomicUsize::new(0));
        let mut request = request(TileRasterMode::Deterministic);
        request.scroll_direction = (0., 1.);
        let (first, batch) = scene.build_parallel_frame(&source, request).unwrap();
        assert_eq!((batch.visible_tiles, batch.prepaint_tiles), (1, 1));
        assert_eq!(scene.cached_tiles(), 2);
        let mut presenter = ReferenceCpuPresenter::default();
        assert_eq!(
            presenter.present(&first).unwrap().get_pixel(10, 10),
            Some([0, 0, 80, 255])
        );
        scene.set_scroll(scroll, (0., 256.)).unwrap();
        let (second, batch) = scene.build_parallel_frame(&source, request).unwrap();
        assert_eq!((batch.visible_tiles, batch.prepaint_tiles), (0, 1));
        assert_eq!(
            presenter.present(&second).unwrap().get_pixel(10, 10),
            Some([23, 17, 80, 255])
        );
        scene.set_scroll(scroll, (0., 0.)).unwrap();
        let (_, batch) = scene.build_parallel_frame(&source, request).unwrap();
        assert_eq!(batch.visible_tiles, 0);
        assert!(scene.cached_tiles() <= 3);
    }

    #[test]
    fn parallel_and_deterministic_raster_produce_identical_pixels() {
        let (mut serial, _) = article();
        let (mut parallel, _) = article();
        let source = Article(AtomicUsize::new(0));
        let (a, sa) = serial
            .build_parallel_frame(&source, request(TileRasterMode::Deterministic))
            .unwrap();
        let (b, sb) = parallel
            .build_parallel_frame(&source, request(TileRasterMode::Parallel { workers: 4 }))
            .unwrap();
        assert_eq!(sa.rasterized_tiles, sb.rasterized_tiles);
        assert_eq!(sa.workers, 1);
        assert_eq!(sb.workers, 2);
        let mut pa = ReferenceCpuPresenter::default();
        let mut pb = ReferenceCpuPresenter::default();
        assert_eq!(pa.present(&a).unwrap(), pb.present(&b).unwrap());
    }

    #[test]
    fn worker_pool_keeps_old_tile_until_new_generation_is_ready() {
        struct Slow;
        impl PaintSource for Slow {
            fn item_count(&self) -> usize {
                1
            }
            fn rasterize(&self, _: Range<usize>, _: Rect, out: &mut Surface) {
                std::thread::sleep(std::time::Duration::from_millis(20));
                out.clear([7, 8, 9, 255]);
            }
        }
        let (mut scene, _) = article();
        let source = Article(AtomicUsize::new(0));
        let mut request = request(TileRasterMode::Deterministic);
        request.scroll_direction = (0., 0.);
        scene.build_parallel_frame(&source, request).unwrap();
        let key = TileKey {
            layer: 0,
            x: 0,
            y: 0,
        };
        let before = scene.tiles[&key].surface.get_pixel(0, 0);
        scene.invalidate_chunk(0);
        let mut pool = TileRasterPool::new(2);
        assert_eq!(
            scene
                .schedule_tiles(&mut pool, Arc::new(Slow), request)
                .unwrap(),
            1
        );
        assert_eq!(scene.tiles[&key].surface.get_pixel(0, 0), before);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while scene.collect_ready_tiles(&mut pool) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            scene.tiles[&key].surface.get_pixel(0, 0),
            Some([7, 8, 9, 255])
        );
        assert_eq!(pool.pending_tiles(), 0);
        let frame = scene.build_frame(&Slow, 256, 256).unwrap();
        assert_eq!(frame.resources.uploads.len(), 1);
        assert!(!frame.damage.is_empty());
    }
}
