//! Bounded RTP video-frame assembly after jitter ordering.

use std::collections::BTreeMap;

use crate::{JitterOutput, RtpPacket, WebRtcError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPlayoutConfig {
    pub clock_rate: u32,
    pub delay_ms: u64,
    pub capacity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoQueuePush {
    Queued,
    Duplicate,
    Late,
    Overflow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VideoQueueStats {
    pub queued: u64,
    pub emitted: u64,
    pub duplicates: u64,
    pub late: u64,
    pub overflow: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedVideoFrame<T> {
    pub timestamp: u32,
    pub extended_timestamp: u64,
    pub frame: T,
}

pub struct VideoPlayoutQueue<T> {
    config: VideoPlayoutConfig,
    queue: BTreeMap<u64, (u32, T)>,
    base_timestamp: Option<u64>,
    highest_timestamp: Option<u64>,
    last_emitted: Option<u64>,
    base_arrival_ms: u64,
    stats: VideoQueueStats,
}

impl<T> VideoPlayoutQueue<T> {
    pub fn new(config: VideoPlayoutConfig) -> Result<Self, WebRtcError> {
        if config.clock_rate == 0 || config.capacity == 0 {
            return Err(WebRtcError::InvalidPacket(
                "video playout queue configuration is invalid",
            ));
        }
        Ok(Self {
            config,
            queue: BTreeMap::new(),
            base_timestamp: None,
            highest_timestamp: None,
            last_emitted: None,
            base_arrival_ms: 0,
            stats: VideoQueueStats::default(),
        })
    }

    pub fn push(&mut self, timestamp: u32, frame: T, arrival_ms: u64) -> VideoQueuePush {
        let extended = match self.highest_timestamp {
            Some(reference) => extend_timestamp(timestamp, reference),
            None => {
                let first = timestamp as u64;
                self.base_timestamp = Some(first);
                self.base_arrival_ms = arrival_ms;
                first
            }
        };
        if self.last_emitted.is_some_and(|last| extended <= last) {
            self.stats.late = self.stats.late.saturating_add(1);
            return VideoQueuePush::Late;
        }
        if self.queue.contains_key(&extended) {
            self.stats.duplicates = self.stats.duplicates.saturating_add(1);
            return VideoQueuePush::Duplicate;
        }
        if self.queue.len() >= self.config.capacity {
            self.stats.overflow = self.stats.overflow.saturating_add(1);
            return VideoQueuePush::Overflow;
        }
        self.highest_timestamp = Some(
            self.highest_timestamp
                .map_or(extended, |highest| highest.max(extended)),
        );
        self.queue.insert(extended, (timestamp, frame));
        self.stats.queued = self.stats.queued.saturating_add(1);
        VideoQueuePush::Queued
    }

    pub fn pop_due(&mut self, now_ms: u64) -> Option<TimedVideoFrame<T>> {
        let (&extended, _) = self.queue.first_key_value()?;
        let base = self.base_timestamp?;
        let media_ms =
            extended.saturating_sub(base).saturating_mul(1_000) / self.config.clock_rate as u64;
        let due = self
            .base_arrival_ms
            .saturating_add(self.config.delay_ms)
            .saturating_add(media_ms);
        if now_ms < due {
            return None;
        }
        let (timestamp, frame) = self.queue.remove(&extended)?;
        self.last_emitted = Some(extended);
        self.stats.emitted = self.stats.emitted.saturating_add(1);
        Some(TimedVideoFrame {
            timestamp,
            extended_timestamp: extended,
            frame,
        })
    }

    pub fn stats(&self) -> VideoQueueStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

fn extend_timestamp(timestamp: u32, reference: u64) -> u64 {
    let mut candidate = (reference & !0xffff_ffff) | timestamp as u64;
    if candidate.saturating_add(0x8000_0000) < reference {
        candidate = candidate.saturating_add(0x1_0000_0000);
    } else if candidate > reference.saturating_add(0x8000_0000) && candidate >= 0x1_0000_0000 {
        candidate -= 0x1_0000_0000;
    }
    candidate
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFrameConfig {
    pub ssrc: u32,
    pub payload_type: u8,
    pub max_packets: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedVideoFrame {
    pub ssrc: u32,
    pub payload_type: u8,
    pub timestamp: u32,
    pub first_sequence: u16,
    pub last_sequence: u16,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoAssembly {
    Pending,
    Frame(EncodedVideoFrame),
    Dropped,
}

struct PendingFrame {
    timestamp: u32,
    first_sequence: u16,
    last_sequence: u16,
    packets: usize,
    payload: Vec<u8>,
}

pub struct VideoFrameAssembler {
    config: VideoFrameConfig,
    pending: Option<PendingFrame>,
    discard_until_marker: bool,
    completed: u64,
    dropped: u64,
}

impl VideoFrameAssembler {
    pub fn new(config: VideoFrameConfig) -> Result<Self, WebRtcError> {
        if config.payload_type > 127 || config.max_packets == 0 || config.max_bytes == 0 {
            return Err(WebRtcError::InvalidPacket(
                "video frame assembler configuration is invalid",
            ));
        }
        Ok(Self {
            config,
            pending: None,
            discard_until_marker: false,
            completed: 0,
            dropped: 0,
        })
    }

    pub fn push(&mut self, output: JitterOutput) -> Result<VideoAssembly, WebRtcError> {
        match output {
            JitterOutput::Gap { .. } => {
                if self.pending.take().is_some() {
                    self.dropped = self.dropped.saturating_add(1);
                }
                self.discard_until_marker = true;
                Ok(VideoAssembly::Dropped)
            }
            JitterOutput::Packet(packet) => self.push_packet(packet),
        }
    }

    pub fn counts(&self) -> (u64, u64) {
        (self.completed, self.dropped)
    }

    fn push_packet(&mut self, packet: RtpPacket) -> Result<VideoAssembly, WebRtcError> {
        if packet.ssrc != self.config.ssrc || packet.payload_type != self.config.payload_type {
            return Err(WebRtcError::InvalidPacket(
                "RTP packet does not match video frame assembler",
            ));
        }
        if self.discard_until_marker {
            if packet.marker {
                self.discard_until_marker = false;
            }
            return Ok(VideoAssembly::Dropped);
        }

        let timestamp_changed = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.timestamp != packet.timestamp);
        let mut result = VideoAssembly::Pending;
        if timestamp_changed {
            self.pending = None;
            self.dropped = self.dropped.saturating_add(1);
            result = VideoAssembly::Dropped;
        }
        let pending = self.pending.get_or_insert_with(|| PendingFrame {
            timestamp: packet.timestamp,
            first_sequence: packet.sequence_number,
            last_sequence: packet.sequence_number,
            packets: 0,
            payload: Vec::new(),
        });
        if pending.packets >= self.config.max_packets
            || pending.payload.len().saturating_add(packet.payload.len()) > self.config.max_bytes
        {
            self.pending = None;
            self.discard_until_marker = !packet.marker;
            self.dropped = self.dropped.saturating_add(1);
            return Ok(VideoAssembly::Dropped);
        }
        pending.packets += 1;
        pending.last_sequence = packet.sequence_number;
        pending.payload.extend_from_slice(&packet.payload);
        if !packet.marker {
            return Ok(result);
        }
        let pending = self.pending.take().expect("video frame exists");
        self.completed = self.completed.saturating_add(1);
        Ok(VideoAssembly::Frame(EncodedVideoFrame {
            ssrc: self.config.ssrc,
            payload_type: self.config.payload_type,
            timestamp: pending.timestamp,
            first_sequence: pending.first_sequence,
            last_sequence: pending.last_sequence,
            payload: pending.payload,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFrameProbeReport {
    pub packets: u8,
    pub frame_bytes: usize,
    pub reordered: bool,
    pub damaged_dropped: bool,
}

pub fn run_video_frame_probe() -> Result<VideoFrameProbeReport, WebRtcError> {
    const SSRC: u32 = 0x5649_4445;
    const PT: u8 = 120;
    let mut jitter = crate::RtpJitterBuffer::new(crate::JitterBufferConfig {
        ssrc: SSRC,
        payload_type: PT,
        clock_rate: 90_000,
        playout_delay_ms: 30,
        capacity: 8,
    })?;
    let packet = |sequence_number, marker, payload: &[u8]| RtpPacket {
        marker,
        payload_type: PT,
        sequence_number,
        timestamp: 90_000,
        ssrc: SSRC,
        payload: payload.to_vec(),
    };
    jitter.push(packet(10, false, b"frame-"), 0)?;
    jitter.push(packet(12, true, b"end"), 2)?;
    jitter.push(packet(11, false, b"middle-"), 4)?;
    let mut assembler = VideoFrameAssembler::new(VideoFrameConfig {
        ssrc: SSRC,
        payload_type: PT,
        max_packets: 8,
        max_bytes: 1_024,
    })?;
    let mut frame = None;
    while let Some(output) = jitter.pop_ready(30) {
        if let VideoAssembly::Frame(completed) = assembler.push(output)? {
            frame = Some(completed);
        }
    }
    let frame = frame.ok_or(WebRtcError::InvalidPacket(
        "video probe did not assemble a frame",
    ))?;
    let dropped = assembler.push(JitterOutput::Gap {
        sequence_number: 13,
        extended_sequence: 13,
        estimated_timestamp: None,
        duration_timestamp_units: None,
    })? == VideoAssembly::Dropped;
    Ok(VideoFrameProbeReport {
        packets: 3,
        frame_bytes: frame.payload.len(),
        reordered: frame.first_sequence == 10
            && frame.last_sequence == 12
            && frame.payload == b"frame-middle-end",
        damaged_dropped: dropped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reorders_frame_and_drops_damage() {
        let report = run_video_frame_probe().unwrap();
        assert_eq!(report.packets, 3);
        assert_eq!(report.frame_bytes, 16);
        assert!(report.reordered);
        assert!(report.damaged_dropped);
    }

    #[test]
    fn assembler_bounds_frames_and_recovers_after_marker() {
        let mut assembler = VideoFrameAssembler::new(VideoFrameConfig {
            ssrc: 1,
            payload_type: 120,
            max_packets: 1,
            max_bytes: 2,
        })
        .unwrap();
        let packet = |sequence_number, marker, payload: &[u8]| {
            JitterOutput::Packet(RtpPacket {
                marker,
                payload_type: 120,
                sequence_number,
                timestamp: 1,
                ssrc: 1,
                payload: payload.to_vec(),
            })
        };
        assert_eq!(
            assembler.push(packet(1, false, b"aa")).unwrap(),
            VideoAssembly::Pending
        );
        assert_eq!(
            assembler.push(packet(2, true, b"b")).unwrap(),
            VideoAssembly::Dropped
        );
        assert!(matches!(
            assembler.push(packet(3, true, b"ok")).unwrap(),
            VideoAssembly::Frame(_)
        ));
        assert_eq!(assembler.counts(), (1, 1));
    }

    #[test]
    fn playout_queue_orders_wraps_and_rejects_stale_frames() {
        let mut queue = VideoPlayoutQueue::new(VideoPlayoutConfig {
            clock_rate: 1_000,
            delay_ms: 40,
            capacity: 3,
        })
        .unwrap();
        assert_eq!(queue.push(u32::MAX - 9, "first", 0), VideoQueuePush::Queued);
        assert_eq!(queue.push(10, "third", 2), VideoQueuePush::Queued);
        assert_eq!(queue.push(0, "second", 3), VideoQueuePush::Queued);
        assert_eq!(queue.push(10, "duplicate", 4), VideoQueuePush::Duplicate);
        assert!(queue.pop_due(39).is_none());
        assert_eq!(queue.pop_due(40).unwrap().frame, "first");
        assert_eq!(queue.pop_due(50).unwrap().frame, "second");
        assert_eq!(queue.pop_due(60).unwrap().frame, "third");
        assert_eq!(queue.push(0, "late", 70), VideoQueuePush::Late);
        assert_eq!(queue.stats().emitted, 3);
        assert_eq!(queue.stats().duplicates, 1);
        assert_eq!(queue.stats().late, 1);
    }
}
