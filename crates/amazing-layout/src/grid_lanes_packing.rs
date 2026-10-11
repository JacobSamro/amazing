//! CSS Grid Lanes3 (21 January2026 WD), §4.3–4.4 skipped-space packing.
//! Original std-only interval algorithm; no third-party implementation copied.
//! Inputs are solved track geometry and genuine outer item sizes. Visual ink,
//! relative positioning and transforms do not participate in skipped spaces.

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hole {
    low: f32,
    high: f32,
}

pub(super) struct Spaces {
    tracks: Vec<Vec<Hole>>,
    work: usize,
    entries: usize,
}
impl Spaces {
    pub(super) fn new(tracks: usize) -> Self {
        Self {
            tracks: vec![Vec::new(); tracks],
            work: 0,
            entries: 0,
        }
    }
    fn charge(&mut self) -> Option<()> {
        self.work = self.work.checked_add(1)?;
        (self.work <= super::MAX_GRID_PLACEMENT_WORK).then_some(())
    }
    pub(super) fn skipped(
        &mut self,
        running: &[f32],
        start: usize,
        span: usize,
        stack: f32,
        gap: f32,
    ) -> Option<()> {
        let high = stack - gap;
        for i in start..start.checked_add(span)?.min(self.tracks.len()) {
            self.charge()?;
            let low = *running.get(i)?;
            if low.is_finite() && high.is_finite() && low < high {
                if self.entries >= super::MAX_OCCUPIED_CELLS {
                    return None;
                }
                self.tracks[i].push(Hole { low, high });
                self.entries += 1;
            }
        }
        Some(())
    }
    /// Intersect the actual per-track free intervals. A start may advance when
    /// another track's next interval starts later, and retries until every
    /// spanned interval contains the same sufficient free extent.
    fn first_fit(
        &mut self,
        start: usize,
        span: usize,
        outer: f32,
        normal: f32,
    ) -> Option<Option<f32>> {
        let end = start.checked_add(span)?;
        if end > self.tracks.len() {
            return None;
        }
        let mut point = 0.0f32;
        loop {
            let mut high = f32::INFINITY;
            let mut advanced = false;
            for i in start..end {
                let mut found = None;
                for j in 0..self.tracks[i].len() {
                    self.charge()?;
                    let hole = self.tracks[i][j];
                    if hole.high >= point + outer {
                        found = Some(hole);
                        break;
                    }
                }
                let Some(hole) = found else {
                    return Some(None);
                };
                if hole.low > point {
                    point = hole.low;
                    advanced = true;
                }
                high = high.min(hole.high);
            }
            if point >= normal {
                return Some(None);
            }
            if !advanced && point + outer <= high {
                return Some(Some(point));
            }
            // Advancing to a later start can invalidate an earlier interval;
            // the next traversal rechecks all tracks. If nothing advanced but
            // the intersection was too short, skip its earliest ending hole.
            if !advanced {
                let mut next: Option<f32> = None;
                for i in start..end {
                    for j in 0..self.tracks[i].len() {
                        self.charge()?;
                        let low = self.tracks[i][j].low;
                        if low > point {
                            next = Some(next.map_or(low, |v| v.min(low)));
                        }
                    }
                }
                let Some(next) = next else {
                    return Some(None);
                };
                point = next;
            }
        }
    }
    pub(super) fn find(
        &mut self,
        span: usize,
        outer: f32,
        normal: f32,
        tolerance: f32,
        locked: Option<usize>,
        compatible: impl Fn(usize) -> bool,
    ) -> Option<Option<(usize, f32)>> {
        if span == 0 || span > self.tracks.len() || !outer.is_finite() || !normal.is_finite() {
            return None;
        }
        self.find_measured(span, normal, tolerance, locked, compatible, |_| Some(outer))
    }
    pub(super) fn find_measured(
        &mut self,
        span: usize,
        normal: f32,
        tolerance: f32,
        locked: Option<usize>,
        compatible: impl Fn(usize) -> bool,
        mut measure: impl FnMut(usize) -> Option<f32>,
    ) -> Option<Option<(usize, f32)>> {
        if span == 0 || span > self.tracks.len() || !normal.is_finite() {
            return None;
        }
        let mut candidates = Vec::new();
        for start in 0..=self.tracks.len() - span {
            self.charge()?;
            if locked.is_some_and(|line| line != start) || !compatible(start) {
                continue;
            }
            let outer = measure(start)?;
            if !outer.is_finite() {
                return None;
            }
            if let Some(point) = self.first_fit(start, span, outer.max(0.0), normal)? {
                candidates.push((start, point));
            }
        }
        let Some(highest) = candidates.iter().map(|(_, p)| *p).min_by(f32::total_cmp) else {
            return Some(None);
        };
        // Dense ties ignore the cursor: lowest logical track wins, even when
        // its hole is slightly later than the strictly highest compatible one.
        Some(
            candidates
                .into_iter()
                .find(|(_, p)| *p <= highest + tolerance),
        )
    }
    pub(super) fn occupy(
        &mut self,
        start: usize,
        span: usize,
        point: f32,
        outer: f32,
        gap: f32,
    ) -> Option<()> {
        let low = point - gap;
        let high = point + outer.max(0.0) + gap;
        for i in start..start.checked_add(span)?.min(self.tracks.len()) {
            let mut next = Vec::new();
            for j in 0..self.tracks[i].len() {
                self.charge()?;
                let old = self.tracks[i][j];
                if high <= old.low || low >= old.high {
                    next.push(old);
                    continue;
                }
                if old.low < low {
                    next.push(Hole {
                        low: old.low,
                        high: low.min(old.high),
                    });
                }
                if high < old.high {
                    next.push(Hole {
                        low: high.max(old.low),
                        high: old.high,
                    });
                }
            }
            let count = self
                .entries
                .checked_sub(self.tracks[i].len())?
                .checked_add(next.len())?;
            if count > super::MAX_OCCUPIED_CELLS {
                return None;
            }
            self.entries = count;
            self.tracks[i] = next;
        }
        Some(())
    }
}

pub(super) fn extent(
    positions: &[f32],
    sizes: &[f32],
    start: usize,
    span: usize,
) -> Option<(f32, f32)> {
    let end = start.checked_add(span)?;
    if span == 0 || end > positions.len() || end > sizes.len() {
        return None;
    }
    let low = positions[start..end]
        .iter()
        .copied()
        .min_by(f32::total_cmp)?;
    let high = (start..end)
        .map(|i| positions[i] + sizes[i])
        .max_by(f32::total_cmp)?;
    (low.is_finite() && high.is_finite()).then_some((low, (high - low).max(0.0)))
}
