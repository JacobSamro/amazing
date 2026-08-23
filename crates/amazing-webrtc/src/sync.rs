//! RTCP Sender Report anchored media clocks and A/V synchronization.

use crate::{RtcpSenderReport, WebRtcError};

const NANOSECONDS_PER_SECOND: i128 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaClockAnchor {
    pub ssrc: u32,
    pub rtp_timestamp: u32,
    pub ntp_timestamp: u64,
    pub clock_rate: u32,
}

impl MediaClockAnchor {
    pub fn from_sender_report(
        report: RtcpSenderReport,
        clock_rate: u32,
    ) -> Result<Self, WebRtcError> {
        if clock_rate == 0 {
            return Err(WebRtcError::InvalidPacket(
                "media clock rate must be positive",
            ));
        }
        Ok(Self {
            ssrc: report.ssrc,
            rtp_timestamp: report.rtp_timestamp,
            ntp_timestamp: report.ntp_timestamp,
            clock_rate,
        })
    }

    pub fn presentation_nanoseconds(&self, timestamp: u32) -> i128 {
        let seconds = (self.ntp_timestamp >> 32) as i128;
        let fraction = (self.ntp_timestamp & 0xffff_ffff) as i128;
        let anchor_ns = seconds
            .saturating_mul(NANOSECONDS_PER_SECOND)
            .saturating_add(fraction.saturating_mul(NANOSECONDS_PER_SECOND) / (1i128 << 32));
        let delta = timestamp.wrapping_sub(self.rtp_timestamp) as i32 as i128;
        anchor_ns
            .saturating_add(delta.saturating_mul(NANOSECONDS_PER_SECOND) / self.clock_rate as i128)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvSyncDecision {
    Present,
    Hold { milliseconds: u64 },
    Drop { milliseconds: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvSyncTimeline {
    pub audio: MediaClockAnchor,
    pub video: MediaClockAnchor,
    pub tolerance_ms: u64,
}

impl AvSyncTimeline {
    pub fn new(
        audio: MediaClockAnchor,
        video: MediaClockAnchor,
        tolerance_ms: u64,
    ) -> Result<Self, WebRtcError> {
        if audio.ssrc == video.ssrc || tolerance_ms > 5_000 {
            return Err(WebRtcError::InvalidPacket(
                "A/V sync anchors or tolerance are invalid",
            ));
        }
        Ok(Self {
            audio,
            video,
            tolerance_ms,
        })
    }

    pub fn decide(&self, audio_timestamp: u32, video_timestamp: u32) -> AvSyncDecision {
        let audio_ns = self.audio.presentation_nanoseconds(audio_timestamp);
        let video_ns = self.video.presentation_nanoseconds(video_timestamp);
        let delta_ms = (video_ns - audio_ns) / 1_000_000;
        if delta_ms > self.tolerance_ms as i128 {
            AvSyncDecision::Hold {
                milliseconds: delta_ms.min(u64::MAX as i128) as u64,
            }
        } else if delta_ms < -(self.tolerance_ms as i128) {
            AvSyncDecision::Drop {
                milliseconds: (-delta_ms).min(u64::MAX as i128) as u64,
            }
        } else {
            AvSyncDecision::Present
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(
        ssrc: u32,
        rtp_timestamp: u32,
        ntp_timestamp: u64,
        clock_rate: u32,
    ) -> MediaClockAnchor {
        MediaClockAnchor::from_sender_report(
            RtcpSenderReport {
                ssrc,
                ntp_timestamp,
                rtp_timestamp,
                packet_count: 1,
                octet_count: 1,
            },
            clock_rate,
        )
        .unwrap()
    }

    #[test]
    fn sender_report_clock_maps_fraction_and_rtp_wrap() {
        let clock = anchor(1, u32::MAX - 47_999, 10u64 << 32, 48_000);
        assert_eq!(
            clock.presentation_nanoseconds(0),
            11 * NANOSECONDS_PER_SECOND
        );
        let half = anchor(1, 0, (10u64 << 32) | 0x8000_0000, 90_000);
        assert_eq!(
            half.presentation_nanoseconds(45_000),
            11 * NANOSECONDS_PER_SECOND
        );
    }

    #[test]
    fn timeline_holds_presents_and_drops_against_audio_master() {
        let timeline = AvSyncTimeline::new(
            anchor(1, 0, 20u64 << 32, 48_000),
            anchor(2, 0, 20u64 << 32, 90_000),
            20,
        )
        .unwrap();
        assert_eq!(timeline.decide(24_000, 45_000), AvSyncDecision::Present);
        assert_eq!(
            timeline.decide(24_000, 54_000),
            AvSyncDecision::Hold { milliseconds: 100 }
        );
        assert_eq!(
            timeline.decide(28_800, 45_000),
            AvSyncDecision::Drop { milliseconds: 100 }
        );
    }
}
