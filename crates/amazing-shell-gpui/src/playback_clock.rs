const DEFAULT_INTERVAL_MS: u64 = 500;
const MIN_DELAY_MS: u64 = 16;
const MAX_DELAY_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackUpdate {
    pub frame_index: usize,
    pub dropped_total: u64,
    pub next_delay_ms: u64,
}

pub struct VideoPlaybackClock {
    presentation_ms: Vec<u64>,
    origin_ms: u64,
    sequence: u64,
    dropped_total: u64,
}

impl VideoPlaybackClock {
    pub fn new(presentation_ms: Vec<u64>, origin_ms: u64) -> Result<Self, &'static str> {
        validate_timeline(&presentation_ms)?;
        Ok(Self {
            presentation_ms,
            origin_ms,
            sequence: 0,
            dropped_total: 0,
        })
    }

    pub fn replace_timeline(
        &mut self,
        presentation_ms: Vec<u64>,
        origin_ms: u64,
    ) -> Result<(), &'static str> {
        validate_timeline(&presentation_ms)?;
        self.presentation_ms = presentation_ms;
        self.origin_ms = origin_ms;
        self.sequence = 0;
        self.dropped_total = 0;
        Ok(())
    }

    pub fn update(&mut self, now_ms: u64) -> PlaybackUpdate {
        let elapsed_ms = now_ms.saturating_sub(self.origin_ms);
        let interval_ms = self.loop_interval_ms();
        let cycle_ms = self
            .presentation_ms
            .last()
            .copied()
            .unwrap_or_default()
            .saturating_add(interval_ms);
        let cycle = elapsed_ms / cycle_ms;
        let within_cycle_ms = elapsed_ms % cycle_ms;
        let frame_index = self
            .presentation_ms
            .partition_point(|presentation| *presentation <= within_cycle_ms)
            .saturating_sub(1);
        let target_sequence = cycle
            .saturating_mul(self.presentation_ms.len() as u64)
            .saturating_add(frame_index as u64);
        let skipped = target_sequence
            .saturating_sub(self.sequence)
            .saturating_sub(1);
        self.dropped_total = self.dropped_total.saturating_add(skipped);
        self.sequence = target_sequence;

        let next_within_cycle_ms = self
            .presentation_ms
            .get(frame_index + 1)
            .copied()
            .unwrap_or(cycle_ms);
        let next_deadline_ms = cycle
            .saturating_mul(cycle_ms)
            .saturating_add(next_within_cycle_ms);
        let next_delay_ms = next_deadline_ms
            .saturating_sub(elapsed_ms)
            .clamp(MIN_DELAY_MS, MAX_DELAY_MS);
        PlaybackUpdate {
            frame_index,
            dropped_total: self.dropped_total,
            next_delay_ms,
        }
    }

    fn loop_interval_ms(&self) -> u64 {
        if self.presentation_ms.len() < 2 {
            DEFAULT_INTERVAL_MS
        } else {
            let last = self.presentation_ms.len() - 1;
            self.presentation_ms[last].saturating_sub(self.presentation_ms[last - 1])
        }
    }
}

fn validate_timeline(presentation_ms: &[u64]) -> Result<(), &'static str> {
    if presentation_ms.first() != Some(&0) {
        return Err("video presentation timeline must start at zero");
    }
    if presentation_ms.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("video presentation timeline must be strictly increasing");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_validation_rejects_missing_origin_and_backward_time() {
        assert!(VideoPlaybackClock::new(Vec::new(), 0).is_err());
        assert!(VideoPlaybackClock::new(vec![1], 0).is_err());
        assert!(VideoPlaybackClock::new(vec![0, 100, 100], 0).is_err());
        assert!(VideoPlaybackClock::new(vec![0, 100, 99], 0).is_err());
    }

    #[test]
    fn absolute_deadlines_follow_irregular_timeline_and_loop_interval() {
        let mut clock = VideoPlaybackClock::new(vec![0, 100, 300], 1_000).unwrap();
        assert_eq!(clock.update(1_000).next_delay_ms, 100);
        let second = clock.update(1_100);
        assert_eq!(
            (
                second.frame_index,
                second.dropped_total,
                second.next_delay_ms
            ),
            (1, 0, 200)
        );
        let third = clock.update(1_350);
        assert_eq!(
            (third.frame_index, third.dropped_total, third.next_delay_ms),
            (2, 0, 150)
        );
        let wrapped = clock.update(1_500);
        assert_eq!(
            (
                wrapped.frame_index,
                wrapped.dropped_total,
                wrapped.next_delay_ms
            ),
            (0, 0, 100)
        );
    }

    #[test]
    fn late_update_catches_up_once_and_counts_skipped_frames() {
        let mut clock = VideoPlaybackClock::new(vec![0, 100, 300], 0).unwrap();
        let late = clock.update(350);
        assert_eq!(late.frame_index, 2);
        assert_eq!((late.dropped_total, late.next_delay_ms), (1, 150));
        let same = clock.update(360);
        assert_eq!(
            (same.frame_index, same.dropped_total, same.next_delay_ms),
            (2, 1, 140)
        );
    }

    #[test]
    fn replacement_resets_origin_sequence_and_drop_count() {
        let mut clock = VideoPlaybackClock::new(vec![0, 100], 0).unwrap();
        assert_eq!(clock.update(250).dropped_total, 1);
        clock.replace_timeline(vec![0, 500], 10_000).unwrap();
        let reset = clock.update(10_000);
        assert_eq!(
            (reset.frame_index, reset.dropped_total, reset.next_delay_ms),
            (0, 0, 500)
        );
    }

    #[test]
    fn timer_delay_is_bounded_for_tiny_and_large_intervals() {
        let mut tiny = VideoPlaybackClock::new(vec![0, 1], 0).unwrap();
        assert_eq!(tiny.update(0).next_delay_ms, MIN_DELAY_MS);
        let mut large = VideoPlaybackClock::new(vec![0, 10_000], 0).unwrap();
        assert_eq!(large.update(0).next_delay_ms, MAX_DELAY_MS);
    }
}
