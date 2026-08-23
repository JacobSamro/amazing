//! Bounded RTP video-frame assembly after jitter ordering.

use crate::{JitterOutput, RtpPacket, WebRtcError};

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
}
