//! Property-tree hit testing and compositor-owned wheel scrolling.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitTestResult {
    pub layer: usize,
    pub chunk: usize,
    pub local: (f32, f32),
    pub scroll: ScrollId,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelResult {
    MainThread,
    NoScroller,
    Scrolled { node: ScrollId, offset: (f32, f32) },
}
impl Compositor {
    /// Returns the frontmost chunk whose conservative paint bounds and clip
    /// chain contain the device-pixel point. Exact painted-alpha hit testing
    /// remains the future paint-view adapter's responsibility.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<HitTestResult> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        for (li, layer) in self.layers.iter().enumerate().rev() {
            if !self.trees.visible(layer.state, x, y) {
                continue;
            }
            let Some((u, v)) = self.trees.matrix(layer.state).and_then(|m| m.unmap(x, y)) else {
                continue;
            };
            for &ci in layer.chunks.iter().rev() {
                if self.chunks[ci].bounds.contains(u, v) {
                    return Some(HitTestResult {
                        layer: li,
                        chunk: ci,
                        local: (u, v),
                        scroll: layer.state.scroll,
                    });
                }
            }
        }
        None
    }
    /// Positive deltas increase the scroll offset. A non-passive wheel
    /// listener reserves the event for the main thread without mutation.
    pub fn wheel_scroll(
        &mut self,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
        has_non_passive_listener: bool,
    ) -> Result<WheelResult, TreeError> {
        if has_non_passive_listener {
            return Ok(WheelResult::MainThread);
        }
        if !dx.is_finite() || !dy.is_finite() {
            return Err(TreeError::NonFinite);
        }
        let Some(hit) = self.hit_test(x, y) else {
            return Ok(WheelResult::NoScroller);
        };
        if hit.scroll.0 == 0 {
            return Ok(WheelResult::NoScroller);
        }
        let Some(node) = self.trees.scroll(hit.scroll) else {
            return Ok(WheelResult::NoScroller);
        };
        let max_x = (node.contents.w - node.container.w).max(0.);
        let max_y = (node.contents.h - node.container.h).max(0.);
        let offset = (
            (node.offset.0 + dx).clamp(0., max_x),
            (node.offset.1 + dy).clamp(0., max_y),
        );
        self.trees.set_scroll(hit.scroll, offset)?;
        Ok(WheelResult::Scrolled {
            node: hit.scroll,
            offset,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn topmost_hit_and_wheel_listener_gate() {
        let mut trees = PropertyTrees::new();
        let scroll = trees
            .add_scroll(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 100., 100.),
                Rect::new(0., 0., 300., 300.),
            )
            .unwrap();
        let front = trees
            .add_transform(
                TransformId(0),
                Matrix4::translation(10., 0., 0.),
                (0., 0., 0.),
                false,
            )
            .unwrap();
        let a = PropertyTreeState {
            scroll,
            ..Default::default()
        };
        let b = PropertyTreeState {
            scroll,
            transform: front,
            ..Default::default()
        };
        let chunks = vec![
            PaintChunk {
                items: 0..1,
                state: a,
                bounds: Rect::new(0., 0., 40., 40.),
                reasons: CompositingReasons::SCROLLER,
            },
            PaintChunk {
                items: 1..2,
                state: b,
                bounds: Rect::new(0., 0., 40., 40.),
                reasons: CompositingReasons::WILL_CHANGE,
            },
        ];
        let mut c = Compositor::new(trees, chunks, 2).unwrap();
        let hit = c.hit_test(20., 10.).unwrap();
        assert_eq!(hit.chunk, 1);
        assert_eq!(hit.local, (10., 10.));
        assert_eq!(
            c.wheel_scroll(20., 10., 0., 30., true).unwrap(),
            WheelResult::MainThread
        );
        assert_eq!(c.trees().scroll(scroll).unwrap().offset, (0., 0.));
        assert_eq!(
            c.wheel_scroll(20., 10., 0., 30., false).unwrap(),
            WheelResult::Scrolled {
                node: scroll,
                offset: (0., 30.)
            }
        );
        assert!(c.hit_test(20., 10.).is_none());
    }
    #[test]
    fn clip_rejects_hits_and_wheel_clamps_at_content_edge() {
        let mut trees = PropertyTrees::new();
        let clip = trees
            .add_clip(
                ClipId(0),
                TransformId(0),
                ClipShape::Rect(Rect::new(0., 0., 20., 20.)),
            )
            .unwrap();
        let scroll = trees
            .add_scroll(
                ScrollId(0),
                (0., 0.),
                Rect::new(0., 0., 20., 20.),
                Rect::new(0., 0., 30., 25.),
            )
            .unwrap();
        let state = PropertyTreeState {
            clip,
            scroll,
            ..Default::default()
        };
        let chunk = PaintChunk {
            items: 0..1,
            state,
            bounds: Rect::new(0., 0., 30., 25.),
            reasons: CompositingReasons::SCROLLER,
        };
        let mut c = Compositor::new(trees, vec![chunk], 1).unwrap();
        assert!(c.hit_test(21., 10.).is_none());
        assert_eq!(c.hit_test(10., 10.).unwrap().chunk, 0);
        assert_eq!(
            c.wheel_scroll(10., 10., 100., 100., false).unwrap(),
            WheelResult::Scrolled {
                node: scroll,
                offset: (10., 5.)
            }
        );
        assert_eq!(c.trees().scroll(scroll).unwrap().offset, (10., 5.));
    }
}
