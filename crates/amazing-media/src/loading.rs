//! Fetch and decode scheduling policy for HTML images. The embedding driver
//! owns viewport tracking, task queues, network requests, and JS promises.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loading {
    Eager,
    Lazy,
}
impl Loading {
    pub fn from_attribute(value: Option<&str>) -> Self {
        if value.is_some_and(|v| v.eq_ignore_ascii_case("lazy")) { Self::Lazy } else { Self::Eager }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoding {
    Sync,
    Async,
    Auto,
}
impl Decoding {
    pub fn from_attribute(value: Option<&str>) -> Self {
        match value {
            Some(v) if v.eq_ignore_ascii_case("sync") => Self::Sync,
            Some(v) if v.eq_ignore_ascii_case("async") => Self::Async,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl ViewportRect {
    fn bounds(self) -> Option<(f64, f64, f64, f64)> {
        if ![self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite())
            || self.width < 0.0 || self.height < 0.0 { return None; }
        Some((self.x as f64, self.y as f64,
            self.x as f64 + self.width as f64,
            self.y as f64 + self.height as f64))
    }
}

/// Returns whether an image may start fetching now. `margin` is a nonnegative
/// CSS pixel root margin chosen by the driver. Unknown geometry loads eagerly
/// so an image never remains stranded after a layout failure.
pub fn should_fetch(
    loading: Loading,
    image: Option<ViewportRect>,
    viewport: ViewportRect,
    margin: f32,
) -> bool {
    if loading == Loading::Eager { return true; }
    let (Some((ix0, iy0, ix1, iy1)), Some((vx0, vy0, vx1, vy1))) =
        (image.and_then(ViewportRect::bounds), viewport.bounds()) else { return true; };
    let margin = if margin.is_finite() { margin.max(0.0) as f64 } else { 0.0 };
    ix0 <= vx1 + margin && ix1 >= vx0 - margin
        && iy0 <= vy1 + margin && iy1 >= vy0 - margin
}

/// A generation token lets the driver discard stale fetch/decode completions
/// after `src`, `srcset`, or `<picture>` selection changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    pub current_src: Option<String>,
    pub generation: u64,
    pub fetch_started: bool,
    pub complete: bool,
    pub decoded: bool,
}
impl Default for ImageRequest {
    fn default() -> Self {
        Self { current_src: None, generation: 0, fetch_started: false, complete: true, decoded: false }
    }
}
impl ImageRequest {
    /// Returns `true` only when a new source invalidates the old request.
    pub fn select(&mut self, url: Option<&str>) -> bool {
        if self.current_src.as_deref() == url { return false; }
        self.current_src = url.map(str::to_owned);
        self.generation = self.generation.wrapping_add(1);
        self.fetch_started = false;
        self.complete = url.is_none();
        self.decoded = false;
        true
    }

    /// `decode()` must wait for this generation's fetch and pixel decode. The
    /// JS binding resolves or rejects its promise from the matching completion.
    pub fn finish(&mut self, generation: u64, decoded: bool) -> bool {
        if generation != self.generation || !self.fetch_started { return false; }
        self.complete = true;
        self.decoded = decoded;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_loads_at_viewport_intersection_and_rechecks_after_scroll() {
        let viewport = ViewportRect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        let below = ViewportRect { x: 0.0, y: 700.0, width: 100.0, height: 100.0 };
        assert!(!should_fetch(Loading::Lazy, Some(below), viewport, 0.0));
        assert!(should_fetch(Loading::Lazy, Some(below), viewport, 100.0));
        assert!(should_fetch(Loading::Eager, Some(below), viewport, 0.0));
        assert!(should_fetch(Loading::Lazy, None, viewport, 0.0));
    }
    #[test]
    fn source_change_invalidates_pending_decode_completion() {
        let mut request = ImageRequest::default();
        assert!(request.select(Some("a.png")));
        let old = request.generation;
        assert!(request.select(Some("b.png")));
        assert!(!request.finish(old, true));
        assert!(!request.complete);
        request.fetch_started = true;
        assert!(request.finish(request.generation, true));
        assert!(request.complete && request.decoded);
    }
}
