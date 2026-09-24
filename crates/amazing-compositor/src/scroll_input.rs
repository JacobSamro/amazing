//! Compositor-owned scroll input, animation and asynchronous event delivery.
//! The caller snapshots DOM listener eligibility before forwarding cancelable
//! wheel/touch input; frames use retained property nodes and cached tiles.
use crate::{Compositor, ScrollId, TreeError, WheelResult};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis { Horizontal, Vertical }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollBehavior { Instant, Smooth }
impl ScrollBehavior {
    /// Resolve scrollTo's explicit behavior against computed scroll-behavior.
    /// `auto` (or an omitted option) follows the CSS value.
    pub fn resolve(computed_css: Option<&str>, requested: Option<&str>) -> Self {
        match requested.map(str::trim) {
            Some("instant") => Self::Instant,
            Some("smooth") => Self::Smooth,
            _ if computed_css.is_some_and(|value| value.trim() == "smooth") => Self::Smooth,
            _ => Self::Instant,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollKey { ArrowUp, ArrowDown, ArrowLeft, ArrowRight, PageUp, PageDown, Home, End }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollInput {
    Wheel { x: f32, y: f32, delta: (f32, f32) },
    Trackpad { x: f32, y: f32, delta: (f32, f32) },
    TouchPan { x: f32, y: f32, delta: (f32, f32) },
    Keyboard { node: ScrollId, key: ScrollKey },
    ScrollbarDrag { node: ScrollId, axis: ScrollAxis, fraction: f32 },
    ScrollTo { node: ScrollId, target: (f32, f32), behavior: ScrollBehavior },
    /// Velocity in CSS pixels per millisecond at the end of a trackpad/touch gesture.
    Momentum { node: ScrollId, velocity: (f32, f32) },
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollInputResult {
    MainThread,
    Unhandled,
    Scheduled { node: ScrollId },
    Scrolled { node: ScrollId, offset: (f32, f32) },
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsyncScrollEvent {
    pub node: ScrollId,
    pub offset: (f32, f32),
}
#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    Smooth {
        from: (f32, f32), to: (f32, f32),
        start_ms: f64, duration_ms: f64,
    },
    Momentum { velocity: (f32, f32), last_ms: f64 },
}
#[derive(Debug, Default)]
pub struct ScrollController {
    motion: HashMap<ScrollId, Motion>,
    pending_events: BTreeMap<usize, AsyncScrollEvent>,
}
impl ScrollController {
    pub fn active_count(&self) -> usize { self.motion.len() }
    pub fn pending_scroll_events(&self) -> impl Iterator<Item = AsyncScrollEvent> + '_ {
        self.pending_events.values().copied()
    }
    pub fn take_scroll_events(&mut self) -> Vec<AsyncScrollEvent> {
        std::mem::take(&mut self.pending_events).into_values().collect()
    }
    fn record(&mut self, node: ScrollId, offset: (f32, f32)) {
        self.pending_events.insert(node.0, AsyncScrollEvent { node, offset });
    }
    fn moved(&mut self, result: WheelResult) -> ScrollInputResult {
        match result {
            WheelResult::MainThread => ScrollInputResult::MainThread,
            WheelResult::NoScroller => ScrollInputResult::Unhandled,
            WheelResult::Scrolled { node, offset } => {
                self.motion.remove(&node);
                self.record(node, offset);
                ScrollInputResult::Scrolled { node, offset }
            }
        }
    }
    /// `non_passive_listener` is the DOM event path's resolved wheel/touch
    /// listener flag. If true, input is reserved for the main thread.
    pub fn handle_input(
        &mut self, compositor: &mut Compositor, input: ScrollInput,
        non_passive_listener: bool, now_ms: f64,
    ) -> Result<ScrollInputResult, TreeError> {
        if !now_ms.is_finite() { return Err(TreeError::NonFinite); }
        match input {
            ScrollInput::Wheel { x, y, delta }
            | ScrollInput::Trackpad { x, y, delta }
            | ScrollInput::TouchPan { x, y, delta } => {
                if non_passive_listener { return Ok(ScrollInputResult::MainThread); }
                if let Some(hit) = compositor.hit_test(x, y) {
                    self.motion.remove(&hit.scroll);
                }
                Ok(self.moved(compositor.wheel_scroll(x, y, delta.0, delta.1, false)?))
            }
            ScrollInput::Keyboard { node, key } => {
                let scroll = compositor.trees().scroll(node).ok_or(TreeError::InvalidNode)?;
                let page = scroll.container.h * 0.9;
                let delta = match key {
                    ScrollKey::ArrowUp => (0., -40.),
                    ScrollKey::ArrowDown => (0., 40.),
                    ScrollKey::ArrowLeft => (-40., 0.),
                    ScrollKey::ArrowRight => (40., 0.),
                    ScrollKey::PageUp => (0., -page),
                    ScrollKey::PageDown => (0., page),
                    ScrollKey::Home => (0., -f32::MAX),
                    ScrollKey::End => (0., f32::MAX),
                };
                Ok(self.moved(compositor.scroll_by_node(node, delta.0, delta.1)?))
            }
            ScrollInput::ScrollbarDrag { node, axis, fraction } => {
                if !fraction.is_finite() { return Err(TreeError::NonFinite); }
                let range = compositor.scroll_range(node).ok_or(TreeError::InvalidNode)?;
                let old = compositor.trees().scroll(node).ok_or(TreeError::InvalidNode)?.offset;
                let fraction = fraction.clamp(0., 1.);
                let target = match axis {
                    ScrollAxis::Horizontal => (range.0 * fraction, old.1),
                    ScrollAxis::Vertical => (old.0, range.1 * fraction),
                };
                self.motion.remove(&node);
                self.set_direct(compositor, node, target)
            }
            ScrollInput::ScrollTo { node, target, behavior } => {
                if !target.0.is_finite() || !target.1.is_finite() {
                    return Err(TreeError::NonFinite);
                }
                let range = compositor.scroll_range(node).ok_or(TreeError::InvalidNode)?;
                let target = (target.0.clamp(0., range.0), target.1.clamp(0., range.1));
                let from = compositor.trees().scroll(node).ok_or(TreeError::InvalidNode)?.offset;
                self.motion.remove(&node);
                if from == target { return Ok(ScrollInputResult::Unhandled); }
                if behavior == ScrollBehavior::Smooth {
                    self.motion.insert(node, Motion::Smooth {
                        from, to: target, start_ms: now_ms, duration_ms: 250.,
                    });
                    Ok(ScrollInputResult::Scheduled { node })
                } else {
                    self.set_direct(compositor, node, target)
                }
            }
            ScrollInput::Momentum { node, velocity } => {
                if !velocity.0.is_finite() || !velocity.1.is_finite() {
                    return Err(TreeError::NonFinite);
                }
                compositor.scroll_range(node).ok_or(TreeError::InvalidNode)?;
                self.motion.remove(&node);
                if velocity.0.abs() < 0.02 && velocity.1.abs() < 0.02 {
                    return Ok(ScrollInputResult::Unhandled);
                }
                self.motion.insert(node, Motion::Momentum { velocity, last_ms: now_ms });
                Ok(ScrollInputResult::Scheduled { node })
            }
        }
    }
    fn set_direct(
        &mut self, compositor: &mut Compositor, node: ScrollId, target: (f32, f32),
    ) -> Result<ScrollInputResult, TreeError> {
        let Some(offset) = compositor.scroll_to_clamped(node, target)? else {
            return Ok(ScrollInputResult::Unhandled);
        };
        self.record(node, offset);
        Ok(ScrollInputResult::Scrolled { node, offset })
    }
    /// Sample motions at vsync. A changed offset queues one coalesced scroll
    /// event per node; the driver drains events on a later main-thread task.
    pub fn tick(
        &mut self, compositor: &mut Compositor, now_ms: f64,
    ) -> Result<Vec<AsyncScrollEvent>, TreeError> {
        if !now_ms.is_finite() { return Err(TreeError::NonFinite); }
        let mut changed = Vec::new();
        let active = self.motion.iter().map(|(&id, &motion)| (id, motion)).collect::<Vec<_>>();
        for (id, motion) in active {
            let (target, next) = match motion {
                Motion::Smooth { from, to, start_ms, duration_ms } => {
                    let progress = ((now_ms - start_ms) / duration_ms).clamp(0., 1.);
                    let ease = (progress * progress * (3. - 2. * progress)) as f32;
                    let target = (from.0 + (to.0 - from.0) * ease,
                        from.1 + (to.1 - from.1) * ease);
                    (target, (progress < 1.).then_some(motion))
                }
                Motion::Momentum { velocity, last_ms } => {
                    let dt = (now_ms - last_ms).max(0.).min(100.);
                    let decay = (-dt / 320.).exp() as f32;
                    let distance = (320. * (1. - decay as f64)) as f32;
                    let old = compositor.trees().scroll(id).ok_or(TreeError::InvalidNode)?.offset;
                    let target = (old.0 + velocity.0 * distance, old.1 + velocity.1 * distance);
                    let next_velocity = (velocity.0 * decay, velocity.1 * decay);
                    let still_moving = next_velocity.0.abs() >= 0.02
                        || next_velocity.1.abs() >= 0.02;
                    (target, still_moving.then_some(Motion::Momentum {
                        velocity: next_velocity, last_ms: now_ms,
                    }))
                }
            };
            let old = compositor.trees().scroll(id).ok_or(TreeError::InvalidNode)?.offset;
            let offset = compositor.scroll_to_clamped(id, target)?;
            if let Some(offset) = offset {
                self.record(id, offset);
                changed.push(AsyncScrollEvent { node: id, offset });
            }
            let at_edge = matches!(motion, Motion::Momentum { .. }) && offset.is_none()
                && target != old;
            if at_edge || next.is_none() { self.motion.remove(&id); }
            else if let Some(next) = next { self.motion.insert(id, next); }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompositingReasons, PaintChunk, PaintSource, Positioning, Presenter,
        PropertyTreeState, PropertyTrees, Rect, ReferenceCpuPresenter, StickyConstraint};
    use amazing_paint::Surface;
    use std::{cell::Cell, ops::Range};

    fn scene() -> (Compositor, ScrollId) {
        let mut trees = PropertyTrees::new();
        let scroll = trees.add_scroll(ScrollId(0), (0., 0.),
            Rect::new(0., 0., 64., 64.), Rect::new(0., 0., 64., 192.)).unwrap();
        let chunk = PaintChunk {
            items: 0..1, state: PropertyTreeState { scroll, ..Default::default() },
            bounds: Rect::new(0., 0., 64., 192.),
            reasons: CompositingReasons::SCROLLER,
        };
        (Compositor::new(trees, vec![chunk], 1).unwrap(), scroll)
    }
    struct Rows<'a>(&'a Cell<usize>);
    impl PaintSource for Rows<'_> {
        fn item_count(&self) -> usize { 1 }
        fn rasterize(&self, _: Range<usize>, tile: Rect, out: &mut Surface) {
            self.0.set(self.0.get() + 1);
            for y in 0..out.height {
                let sy = tile.y as i32 + y as i32;
                if !(0..192).contains(&sy) { continue; }
                for x in 0..64 {
                    out.set_pixel(x, y, [sy as u8, 40, 160, 255]);
                }
            }
        }
    }

    #[test]
    fn listener_gate_inputs_clamp_and_async_events_coalesce() {
        let (mut compositor, scroll) = scene();
        let mut controller = ScrollController::default();
        let wheel = ScrollInput::Wheel { x: 10., y: 10., delta: (0., 20.) };
        assert_eq!(controller.handle_input(&mut compositor, wheel, true, 0.).unwrap(),
            ScrollInputResult::MainThread);
        assert_eq!(compositor.trees().scroll(scroll).unwrap().offset, (0., 0.));
        assert!(controller.take_scroll_events().is_empty());
        assert_eq!(controller.handle_input(&mut compositor, wheel, false, 0.).unwrap(),
            ScrollInputResult::Scrolled { node: scroll, offset: (0., 20.) });
        assert_eq!(controller.handle_input(&mut compositor,
            ScrollInput::Keyboard { node: scroll, key: ScrollKey::End }, false, 1.).unwrap(),
            ScrollInputResult::Scrolled { node: scroll, offset: (0., 128.) });
        assert_eq!(controller.handle_input(&mut compositor,
            ScrollInput::ScrollbarDrag { node: scroll, axis: ScrollAxis::Vertical, fraction: 0.5 },
            false, 2.).unwrap(),
            ScrollInputResult::Scrolled { node: scroll, offset: (0., 64.) });
        assert_eq!(controller.take_scroll_events(),
            vec![AsyncScrollEvent { node: scroll, offset: (0., 64.) }]);
        assert_eq!(controller.handle_input(&mut compositor,
            ScrollInput::TouchPan { x: 10., y: 10., delta: (0., 20.) }, true, 3.).unwrap(),
            ScrollInputResult::MainThread);
    }

    #[test]
    fn smooth_scroll_and_momentum_sample_at_vsync_and_stop_at_edges() {
        assert_eq!(ScrollBehavior::resolve(Some("smooth"), Some("auto")), ScrollBehavior::Smooth);
        assert_eq!(ScrollBehavior::resolve(Some("smooth"), Some("instant")), ScrollBehavior::Instant);
        let (mut compositor, scroll) = scene();
        let mut controller = ScrollController::default();
        assert_eq!(controller.handle_input(&mut compositor, ScrollInput::ScrollTo {
            node: scroll, target: (0., 200.), behavior: ScrollBehavior::Smooth,
        }, false, 0.).unwrap(), ScrollInputResult::Scheduled { node: scroll });
        assert_eq!(controller.tick(&mut compositor, 125.).unwrap(),
            vec![AsyncScrollEvent { node: scroll, offset: (0., 64.) }]);
        assert_eq!(controller.tick(&mut compositor, 250.).unwrap(),
            vec![AsyncScrollEvent { node: scroll, offset: (0., 128.) }]);
        assert_eq!(controller.active_count(), 0);
        assert_eq!(controller.handle_input(&mut compositor, ScrollInput::ScrollTo {
            node: scroll, target: (0., 0.), behavior: ScrollBehavior::Instant,
        }, false, 300.).unwrap(), ScrollInputResult::Scrolled { node: scroll, offset: (0., 0.) });
        assert_eq!(controller.handle_input(&mut compositor,
            ScrollInput::Momentum { node: scroll, velocity: (0., 1.) }, false, 300.).unwrap(),
            ScrollInputResult::Scheduled { node: scroll });
        assert!(!controller.tick(&mut compositor, 316.).unwrap().is_empty());
        for t in (1..30).map(|n| 316. + n as f64 * 100.) {
            controller.tick(&mut compositor, t).unwrap();
        }
        assert_eq!(compositor.trees().scroll(scroll).unwrap().offset.1, 128.);
        assert_eq!(controller.active_count(), 0);
    }

    #[test]
    fn scroll_frame_uses_cached_tiles_and_matches_full_repaint() {
        let (mut compositor, scroll) = scene();
        let calls = Cell::new(0);
        let mut presenter = ReferenceCpuPresenter::default();
        let first = compositor.build_frame(&Rows(&calls), 64, 64).unwrap();
        presenter.present(&first).unwrap();
        let initial_raster_calls = calls.get();
        let mut controller = ScrollController::default();
        let style_calls = Cell::new(0);
        let layout_calls = Cell::new(0);
        let paint_view_calls = Cell::new(0);
        assert_eq!(controller.handle_input(&mut compositor,
            ScrollInput::Trackpad { x: 10., y: 10., delta: (0., 32.) }, false, 16.).unwrap(),
            ScrollInputResult::Scrolled { node: scroll, offset: (0., 32.) });
        let frame = compositor.build_cached_frame(64, 64).unwrap();
        let actual = presenter.present(&frame).unwrap();
        assert!(frame.resources.uploads.is_empty());
        assert_eq!(calls.get(), initial_raster_calls);
        assert_eq!((style_calls.get(), layout_calls.get(), paint_view_calls.get()), (0, 0, 0));
        let (mut full, full_scroll) = scene();
        full.set_scroll(full_scroll, (0., 32.)).unwrap();
        let expected_frame = full.build_frame(&Rows(&Cell::new(0)), 64, 64).unwrap();
        let expected = ReferenceCpuPresenter::default().present(&expected_frame).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
    }

    struct OverlaySource;
    impl PaintSource for OverlaySource {
        fn item_count(&self) -> usize { 3 }
        fn rasterize(&self, items: Range<usize>, tile: Rect, out: &mut Surface) {
            for item in items {
                let (rect, color) = match item {
                    0 => (Rect::new(0., 0., 64., 192.), [210, 210, 210, 255]),
                    1 => (Rect::new(20., 0., 10., 10.), [200, 0, 0, 255]),
                    _ => (Rect::new(0., 30., 10., 10.), [0, 180, 0, 255]),
                };
                out.fill_rect(rect.x - tile.x, rect.y - tile.y, rect.w, rect.h, color);
            }
        }
    }
    fn sticky_scene() -> (Compositor, ScrollId) {
        let mut trees = PropertyTrees::new();
        let scroll = trees.add_scroll(ScrollId(0), (0., 0.),
            Rect::new(0., 0., 64., 64.), Rect::new(0., 0., 64., 192.)).unwrap();
        let sticky = trees.add_sticky(StickyConstraint {
            scroll, normal: Rect::new(0., 30., 10., 10.),
            top: Some(0.), right: None, bottom: None, left: None,
        }).unwrap();
        let chunks = vec![
            PaintChunk { items: 0..1, state: PropertyTreeState {
                scroll, ..Default::default() }, bounds: Rect::new(0., 0., 64., 192.),
                reasons: CompositingReasons::SCROLLER },
            PaintChunk { items: 1..2, state: PropertyTreeState {
                scroll, positioning: Positioning::Fixed, ..Default::default() },
                bounds: Rect::new(20., 0., 10., 10.), reasons: CompositingReasons::FIXED },
            PaintChunk { items: 2..3, state: PropertyTreeState {
                scroll, positioning: Positioning::Sticky(sticky), ..Default::default() },
                bounds: Rect::new(0., 30., 10., 10.), reasons: CompositingReasons::SCROLLER },
        ];
        (Compositor::new(trees, chunks, 3).unwrap(), scroll)
    }

    #[test]
    fn fixed_and_sticky_pixels_follow_compositor_scroll_constraints() {
        let (mut compositor, scroll) = sticky_scene();
        let mut presenter = ReferenceCpuPresenter::default();
        presenter.present(&compositor.build_frame(&OverlaySource, 64, 64).unwrap()).unwrap();
        let mut controller = ScrollController::default();
        controller.handle_input(&mut compositor,
            ScrollInput::Keyboard { node: scroll, key: ScrollKey::ArrowDown },
            false, 16.).unwrap();
        let frame = compositor.build_cached_frame(64, 64).unwrap();
        assert!(frame.resources.uploads.is_empty());
        let actual = presenter.present(&frame).unwrap();
        assert_eq!(actual.get_pixel(22, 2), Some([200, 0, 0, 255]));
        assert_eq!(actual.get_pixel(2, 2), Some([0, 180, 0, 255]));
        let (mut full, full_scroll) = sticky_scene();
        full.set_scroll(full_scroll, (0., 40.)).unwrap();
        let expected = ReferenceCpuPresenter::default()
            .present(&full.build_frame(&OverlaySource, 64, 64).unwrap()).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
    }
}
