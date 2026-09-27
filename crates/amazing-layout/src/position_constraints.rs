//! Used absolute-position axis constraints, after content/min/max sizing.
//! Insets and margins are distances from the containing block's physical edges.

#[derive(Clone, Copy)]
pub(super) struct Axis {
    pub extent: f32,
    pub start: Option<f32>,
    pub end: Option<f32>,
    pub margin_start: Option<f32>,
    pub margin_end: Option<f32>,
    pub border_size: f32,
    pub static_edge: f32,
    pub static_end: bool,
    pub end_wins: bool,
    /// CSS2 distributes negative vertical auto margins equally. Horizontal
    /// overflow instead consumes the margin on the containing block's end side.
    pub split_negative: bool,
}

pub(super) struct UsedAxis {
    pub border_start: f32,
    pub margin_start: f32,
    pub margin_end: f32,
}

impl Axis {
    pub fn solve(self) -> UsedAxis {
        let mut before = self.margin_start.unwrap_or(0.0);
        let mut after = self.margin_end.unwrap_or(0.0);
        let position = match (self.start, self.end) {
            (Some(start), Some(end)) => {
                let free = self.extent - start - end - self.border_size;
                match (self.margin_start, self.margin_end) {
                    (None, None) if free >= 0.0 || self.split_negative => {
                        before = free / 2.0;
                        after = before;
                    }
                    (None, None) if self.end_wins => before = free,
                    (None, None) => after = free,
                    (None, Some(_)) => before = free - after,
                    (Some(_), None) => after = free - before,
                    _ => {}
                }
                if self.end_wins {
                    self.extent - end - after - self.border_size
                } else {
                    start + before
                }
            }
            (Some(start), None) => start + before,
            (None, Some(end)) => self.extent - end - after - self.border_size,
            (None, None) if self.static_end => self.static_edge - after - self.border_size,
            (None, None) => self.static_edge + before,
        };
        UsedAxis {
            border_start: position,
            margin_start: before,
            margin_end: after,
        }
    }
}
