//! Bounded RTP reordering and audio-codec adapters.
//!
//! The jitter buffer owns sequence-number extension, fixed playout deadlines,
//! duplicate/late rejection, bounded storage, and explicit loss events. Its
//! interface is synchronous: the caller supplies monotonic milliseconds and
//! decides how those ticks are driven by the surrounding event loop.

use std::collections::BTreeMap;

use crate::{RtpPacket, SdpDescription, WebRtcError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitterBufferConfig {
    pub ssrc: u32,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub playout_delay_ms: u64,
    pub capacity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptivePlayoutConfig {
    pub max_delay_ms: u64,
    pub jitter_multiplier: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitterPush {
    Buffered,
    Duplicate,
    Late,
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitterOutput {
    Packet(RtpPacket),
    Gap {
        sequence_number: u16,
        extended_sequence: u64,
        estimated_timestamp: Option<u32>,
        duration_timestamp_units: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JitterStats {
    pub accepted: u64,
    pub emitted: u64,
    pub duplicates: u64,
    pub late: u64,
    pub overflow: u64,
    pub gaps: u64,
}

struct TimedPacket {
    packet: RtpPacket,
    media_offset_ms: u64,
}

struct AdaptivePlayout {
    config: AdaptivePlayoutConfig,
    previous_transit_ms: Option<i64>,
    jitter_q4: u64,
}

pub struct RtpJitterBuffer {
    config: JitterBufferConfig,
    queue: BTreeMap<u64, TimedPacket>,
    next_sequence: Option<u64>,
    highest_sequence: Option<u64>,
    base_timestamp: u32,
    base_arrival_ms: u64,
    last_output_timestamp: Option<u32>,
    target_delay_ms: u64,
    adaptive: Option<AdaptivePlayout>,
    stats: JitterStats,
}

impl RtpJitterBuffer {
    pub fn new(config: JitterBufferConfig) -> Result<Self, WebRtcError> {
        Self::build(config, None)
    }

    pub fn new_adaptive(
        config: JitterBufferConfig,
        adaptive: AdaptivePlayoutConfig,
    ) -> Result<Self, WebRtcError> {
        if adaptive.max_delay_ms < config.playout_delay_ms || adaptive.jitter_multiplier == 0 {
            return Err(WebRtcError::InvalidPacket(
                "adaptive jitter bounds or multiplier are invalid",
            ));
        }
        Self::build(config, Some(adaptive))
    }

    fn build(
        config: JitterBufferConfig,
        adaptive: Option<AdaptivePlayoutConfig>,
    ) -> Result<Self, WebRtcError> {
        if config.payload_type > 127 {
            return Err(WebRtcError::InvalidPacket(
                "jitter payload type exceeds seven bits",
            ));
        }
        if config.clock_rate == 0 {
            return Err(WebRtcError::InvalidPacket(
                "jitter clock rate must be positive",
            ));
        }
        if config.capacity < 2 {
            return Err(WebRtcError::InvalidPacket(
                "jitter capacity must hold at least two packets",
            ));
        }
        let target_delay_ms = config.playout_delay_ms;
        Ok(Self {
            config,
            queue: BTreeMap::new(),
            next_sequence: None,
            highest_sequence: None,
            base_timestamp: 0,
            base_arrival_ms: 0,
            last_output_timestamp: None,
            target_delay_ms,
            adaptive: adaptive.map(|config| AdaptivePlayout {
                config,
                previous_transit_ms: None,
                jitter_q4: 0,
            }),
            stats: JitterStats::default(),
        })
    }

    pub fn push(&mut self, packet: RtpPacket, arrival_ms: u64) -> Result<JitterPush, WebRtcError> {
        if packet.ssrc != self.config.ssrc {
            return Err(WebRtcError::InvalidPacket(
                "RTP packet SSRC does not match jitter buffer",
            ));
        }
        if packet.payload_type != self.config.payload_type {
            return Err(WebRtcError::InvalidPacket(
                "RTP payload type does not match jitter buffer",
            ));
        }

        let extended = match self.highest_sequence {
            Some(reference) => extend_sequence(packet.sequence_number, reference),
            None => {
                let first = packet.sequence_number as u64;
                self.next_sequence = Some(first);
                self.base_timestamp = packet.timestamp;
                self.base_arrival_ms = arrival_ms;
                first
            }
        };
        if self
            .next_sequence
            .is_some_and(|next_sequence| extended < next_sequence)
        {
            self.stats.late = self.stats.late.saturating_add(1);
            return Ok(JitterPush::Late);
        }
        if self.queue.contains_key(&extended) {
            self.stats.duplicates = self.stats.duplicates.saturating_add(1);
            return Ok(JitterPush::Duplicate);
        }
        if self.queue.len() >= self.config.capacity {
            self.stats.overflow = self.stats.overflow.saturating_add(1);
            return Ok(JitterPush::Overflow);
        }

        let timestamp_delta = packet.timestamp.wrapping_sub(self.base_timestamp) as u64;
        let media_delay_ms = timestamp_delta
            .saturating_mul(1_000)
            .checked_div(self.config.clock_rate as u64)
            .unwrap_or(0);
        self.observe_transit(arrival_ms, media_delay_ms);
        self.highest_sequence = Some(
            self.highest_sequence
                .map_or(extended, |highest| highest.max(extended)),
        );
        self.queue.insert(
            extended,
            TimedPacket {
                packet,
                media_offset_ms: media_delay_ms,
            },
        );
        self.stats.accepted = self.stats.accepted.saturating_add(1);
        Ok(JitterPush::Buffered)
    }

    pub fn pop_ready(&mut self, now_ms: u64) -> Option<JitterOutput> {
        let expected = self.next_sequence?;
        if let Some(timed) = self.queue.get(&expected) {
            if self.playout_at(timed) > now_ms {
                return None;
            }
            let packet = self
                .queue
                .remove(&expected)
                .expect("queued jitter packet disappeared")
                .packet;
            self.last_output_timestamp = Some(packet.timestamp);
            self.next_sequence = Some(expected.saturating_add(1));
            self.stats.emitted = self.stats.emitted.saturating_add(1);
            return Some(JitterOutput::Packet(packet));
        }

        let next_queued = self.queue.first_key_value()?;
        if *next_queued.0 <= expected || self.playout_at(next_queued.1) > now_ms {
            return None;
        }
        self.next_sequence = Some(expected.saturating_add(1));
        self.stats.gaps = self.stats.gaps.saturating_add(1);
        let (estimated_timestamp, duration_timestamp_units) = self
            .last_output_timestamp
            .map(|previous| {
                let outputs_until_next = (*next_queued.0 - expected + 1).max(1) as u32;
                let duration = next_queued
                    .1
                    .packet
                    .timestamp
                    .wrapping_sub(previous)
                    .checked_div(outputs_until_next)
                    .filter(|duration| *duration > 0);
                let estimated = duration.map(|duration| previous.wrapping_add(duration));
                (estimated, duration)
            })
            .unwrap_or((None, None));
        if let Some(timestamp) = estimated_timestamp {
            self.last_output_timestamp = Some(timestamp);
        }
        Some(JitterOutput::Gap {
            sequence_number: expected as u16,
            extended_sequence: expected,
            estimated_timestamp,
            duration_timestamp_units,
        })
    }

    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn target_delay_ms(&self) -> u64 {
        self.target_delay_ms
    }

    fn playout_at(&self, timed: &TimedPacket) -> u64 {
        self.base_arrival_ms
            .saturating_add(self.target_delay_ms)
            .saturating_add(timed.media_offset_ms)
    }

    fn observe_transit(&mut self, arrival_ms: u64, media_offset_ms: u64) {
        let Some(adaptive) = self.adaptive.as_mut() else {
            return;
        };
        let transit = (arrival_ms as i128 - self.base_arrival_ms as i128)
            .saturating_sub(media_offset_ms as i128)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        if let Some(previous) = adaptive.previous_transit_ms {
            let variation = transit.abs_diff(previous);
            adaptive.jitter_q4 = adaptive
                .jitter_q4
                .saturating_add(variation)
                .saturating_sub((adaptive.jitter_q4.saturating_add(8)) >> 4);
            let desired = self
                .config
                .playout_delay_ms
                .saturating_add(
                    (adaptive.jitter_q4 >> 4)
                        .saturating_mul(adaptive.config.jitter_multiplier as u64),
                )
                .min(adaptive.config.max_delay_ms);
            if desired >= self.target_delay_ms {
                self.target_delay_ms = desired;
            } else {
                self.target_delay_ms = self
                    .target_delay_ms
                    .saturating_sub(1)
                    .max(self.config.playout_delay_ms);
            }
        }
        adaptive.previous_transit_ms = Some(transit);
    }
}

fn extend_sequence(sequence: u16, reference: u64) -> u64 {
    let mut candidate = (reference & !0xffff) | sequence as u64;
    if candidate.saturating_add(0x8000) < reference {
        candidate = candidate.saturating_add(0x1_0000);
    } else if candidate > reference.saturating_add(0x8000) && candidate >= 0x1_0000 {
        candidate -= 0x1_0000;
    }
    candidate
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAudioFrame {
    pub timestamp: u32,
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

pub trait RtpAudioDecoder {
    fn payload_type(&self) -> u8;
    fn decode(&mut self, packet: &RtpPacket) -> Result<DecodedAudioFrame, WebRtcError>;
    fn conceal(
        &mut self,
        timestamp: u32,
        samples_per_channel: usize,
    ) -> Result<DecodedAudioFrame, WebRtcError>;
}

pub struct L16Decoder {
    payload_type: u8,
    sample_rate: u32,
    channels: u16,
}

impl L16Decoder {
    pub fn new(payload_type: u8, sample_rate: u32, channels: u16) -> Result<Self, WebRtcError> {
        validate_decoder_config(payload_type, sample_rate, channels)?;
        Ok(Self {
            payload_type,
            sample_rate,
            channels,
        })
    }
}

impl RtpAudioDecoder for L16Decoder {
    fn payload_type(&self) -> u8 {
        self.payload_type
    }

    fn decode(&mut self, packet: &RtpPacket) -> Result<DecodedAudioFrame, WebRtcError> {
        validate_packet_payload(packet, self.payload_type)?;
        let frame_bytes = self.channels as usize * 2;
        if packet.payload.len() % frame_bytes != 0 {
            return Err(WebRtcError::InvalidPacket(
                "L16 payload ends inside an audio sample frame",
            ));
        }
        let samples = packet
            .payload
            .chunks_exact(2)
            .map(|sample| i16::from_be_bytes([sample[0], sample[1]]))
            .collect();
        Ok(DecodedAudioFrame {
            timestamp: packet.timestamp,
            sample_rate: self.sample_rate,
            channels: self.channels,
            samples,
        })
    }

    fn conceal(
        &mut self,
        timestamp: u32,
        samples_per_channel: usize,
    ) -> Result<DecodedAudioFrame, WebRtcError> {
        concealed_audio(
            timestamp,
            self.sample_rate,
            self.channels,
            samples_per_channel,
        )
    }
}

pub struct PcmuDecoder {
    payload_type: u8,
    sample_rate: u32,
}

impl PcmuDecoder {
    pub fn new(payload_type: u8, sample_rate: u32) -> Result<Self, WebRtcError> {
        validate_decoder_config(payload_type, sample_rate, 1)?;
        Ok(Self {
            payload_type,
            sample_rate,
        })
    }
}

impl RtpAudioDecoder for PcmuDecoder {
    fn payload_type(&self) -> u8 {
        self.payload_type
    }

    fn decode(&mut self, packet: &RtpPacket) -> Result<DecodedAudioFrame, WebRtcError> {
        validate_packet_payload(packet, self.payload_type)?;
        Ok(DecodedAudioFrame {
            timestamp: packet.timestamp,
            sample_rate: self.sample_rate,
            channels: 1,
            samples: packet.payload.iter().copied().map(decode_mulaw).collect(),
        })
    }

    fn conceal(
        &mut self,
        timestamp: u32,
        samples_per_channel: usize,
    ) -> Result<DecodedAudioFrame, WebRtcError> {
        concealed_audio(timestamp, self.sample_rate, 1, samples_per_channel)
    }
}

fn concealed_audio(
    timestamp: u32,
    sample_rate: u32,
    channels: u16,
    samples_per_channel: usize,
) -> Result<DecodedAudioFrame, WebRtcError> {
    let sample_count =
        samples_per_channel
            .checked_mul(channels as usize)
            .ok_or(WebRtcError::InvalidPacket(
                "concealed audio length overflow",
            ))?;
    Ok(DecodedAudioFrame {
        timestamp,
        sample_rate,
        channels,
        samples: vec![0; sample_count],
    })
}

fn validate_decoder_config(
    payload_type: u8,
    sample_rate: u32,
    channels: u16,
) -> Result<(), WebRtcError> {
    if payload_type > 127 {
        return Err(WebRtcError::InvalidPacket(
            "decoder payload type exceeds seven bits",
        ));
    }
    if sample_rate == 0 || channels == 0 {
        return Err(WebRtcError::InvalidPacket(
            "decoder rate and channels must be positive",
        ));
    }
    Ok(())
}

fn validate_packet_payload(packet: &RtpPacket, payload_type: u8) -> Result<(), WebRtcError> {
    if packet.payload_type != payload_type {
        return Err(WebRtcError::InvalidPacket(
            "RTP packet does not match decoder payload type",
        ));
    }
    Ok(())
}

fn decode_mulaw(encoded: u8) -> i16 {
    let value = !encoded;
    let sign = value & 0x80 != 0;
    let exponent = (value >> 4) & 0x07;
    let mantissa = value & 0x0f;
    let magnitude = (((mantissa as i32) << 3) + 0x84) << exponent;
    let sample = magnitude - 0x84;
    if sign {
        -(sample as i16)
    } else {
        sample as i16
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioCodecKind {
    L16,
    Pcmu,
    Opus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedAudioCodec {
    pub payload_type: u8,
    pub encoding: String,
    pub clock_rate: u32,
    pub channels: u16,
    kind: AudioCodecKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedAudioCodecs {
    codecs: Vec<NegotiatedAudioCodec>,
}

impl NegotiatedAudioCodecs {
    pub fn codecs(&self) -> &[NegotiatedAudioCodec] {
        &self.codecs
    }

    pub fn codec(&self, payload_type: u8) -> Result<&NegotiatedAudioCodec, WebRtcError> {
        self.codecs
            .iter()
            .find(|codec| codec.payload_type == payload_type)
            .ok_or(WebRtcError::Unsupported(
                "RTP payload type was not negotiated",
            ))
    }

    pub fn jitter_config(
        &self,
        payload_type: u8,
        ssrc: u32,
        playout_delay_ms: u64,
        capacity: usize,
    ) -> Result<JitterBufferConfig, WebRtcError> {
        let codec = self.codec(payload_type)?;
        Ok(JitterBufferConfig {
            ssrc,
            payload_type,
            clock_rate: codec.clock_rate,
            playout_delay_ms,
            capacity,
        })
    }

    pub fn decoder(&self, payload_type: u8) -> Result<Box<dyn RtpAudioDecoder>, WebRtcError> {
        let codec = self.codec(payload_type)?;
        match codec.kind {
            AudioCodecKind::L16 => Ok(Box::new(L16Decoder::new(
                codec.payload_type,
                codec.clock_rate,
                codec.channels,
            )?)),
            AudioCodecKind::Pcmu => Ok(Box::new(PcmuDecoder::new(
                codec.payload_type,
                codec.clock_rate,
            )?)),
            AudioCodecKind::Opus => Err(WebRtcError::Unsupported(
                "Opus decoder requires the isolated codec adapter",
            )),
        }
    }
}

pub fn negotiate_audio_codecs(
    offer: &SdpDescription,
    answer: &SdpDescription,
) -> Result<NegotiatedAudioCodecs, WebRtcError> {
    if offer.media != "audio" || answer.media != "audio" {
        return Err(WebRtcError::Unsupported(
            "audio codec negotiation requires audio media",
        ));
    }
    let mut codecs = Vec::new();
    for offered in &offer.rtp_maps {
        let Some(accepted) = answer
            .rtp_maps
            .iter()
            .find(|mapping| mapping.payload_type == offered.payload_type)
        else {
            continue;
        };
        if !offered.encoding.eq_ignore_ascii_case(&accepted.encoding)
            || offered.clock_rate != accepted.clock_rate
            || offered.channels != accepted.channels
        {
            return Err(WebRtcError::InvalidPacket(
                "answer changed offered RTP codec parameters",
            ));
        }
        let kind = if offered.encoding.eq_ignore_ascii_case("L16") {
            AudioCodecKind::L16
        } else if offered.encoding.eq_ignore_ascii_case("PCMU") && offered.channels == 1 {
            AudioCodecKind::Pcmu
        } else if offered.encoding.eq_ignore_ascii_case("opus")
            && offered.clock_rate == 48_000
            && matches!(offered.channels, 1 | 2)
        {
            AudioCodecKind::Opus
        } else {
            continue;
        };
        codecs.push(NegotiatedAudioCodec {
            payload_type: offered.payload_type,
            encoding: offered.encoding.to_ascii_uppercase(),
            clock_rate: offered.clock_rate,
            channels: offered.channels,
            kind,
        });
    }
    if codecs.is_empty() {
        return Err(WebRtcError::Unsupported(
            "offer and answer have no supported common audio codec",
        ));
    }
    Ok(NegotiatedAudioCodecs { codecs })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitterCodecReport {
    pub emitted_packets: u8,
    pub reordered: bool,
    pub duplicate_rejected: bool,
    pub l16_samples: u8,
    pub pcmu_samples: u8,
}

pub fn run_jitter_codec_probe() -> Result<JitterCodecReport, WebRtcError> {
    const SSRC: u32 = 0x4a49_5452;
    const PAYLOAD_TYPE: u8 = 96;
    let mut jitter = RtpJitterBuffer::new(JitterBufferConfig {
        ssrc: SSRC,
        payload_type: PAYLOAD_TYPE,
        clock_rate: 8_000,
        playout_delay_ms: 60,
        capacity: 8,
    })?;
    let packet = |sequence_number, timestamp, sample: i16| RtpPacket {
        marker: false,
        payload_type: PAYLOAD_TYPE,
        sequence_number,
        timestamp,
        ssrc: SSRC,
        payload: sample.to_be_bytes().to_vec(),
    };
    let first = packet(u16::MAX, 0, -1_000);
    let wrapped = packet(0, 160, 1_000);
    let later = packet(1, 320, 3_000);
    jitter.push(first, 0)?;
    jitter.push(later, 5)?;
    jitter.push(wrapped.clone(), 10)?;
    let duplicate_rejected = jitter.push(wrapped, 11)? == JitterPush::Duplicate;

    let mut decoder = L16Decoder::new(PAYLOAD_TYPE, 8_000, 1)?;
    let mut sequences = Vec::new();
    let mut samples = Vec::new();
    for now_ms in [60, 80, 100] {
        let Some(JitterOutput::Packet(packet)) = jitter.pop_ready(now_ms) else {
            return Err(WebRtcError::InvalidPacket(
                "jitter probe did not emit a scheduled packet",
            ));
        };
        sequences.push(packet.sequence_number);
        samples.extend(decoder.decode(&packet)?.samples);
    }
    let mut pcmu = PcmuDecoder::new(0, 8_000)?;
    let pcmu_frame = pcmu.decode(&RtpPacket {
        marker: false,
        payload_type: 0,
        sequence_number: 1,
        timestamp: 0,
        ssrc: SSRC,
        payload: vec![0xff, 0x80],
    })?;
    Ok(JitterCodecReport {
        emitted_packets: sequences.len() as u8,
        reordered: sequences == [u16::MAX, 0, 1] && samples == [-1_000, 1_000, 3_000],
        duplicate_rejected,
        l16_samples: samples.len() as u8,
        pcmu_samples: pcmu_frame.samples.len() as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(sequence_number: u16, timestamp: u32) -> RtpPacket {
        RtpPacket {
            marker: false,
            payload_type: 96,
            sequence_number,
            timestamp,
            ssrc: 7,
            payload: vec![0, sequence_number as u8],
        }
    }

    fn buffer(capacity: usize) -> RtpJitterBuffer {
        RtpJitterBuffer::new(JitterBufferConfig {
            ssrc: 7,
            payload_type: 96,
            clock_rate: 8_000,
            playout_delay_ms: 60,
            capacity,
        })
        .unwrap()
    }

    #[test]
    fn reorders_packets_and_extends_sequence_across_wrap() {
        let mut buffer = buffer(8);
        assert_eq!(
            buffer.push(packet(u16::MAX, 0), 0).unwrap(),
            JitterPush::Buffered
        );
        assert_eq!(
            buffer.push(packet(1, 320), 5).unwrap(),
            JitterPush::Buffered
        );
        assert_eq!(
            buffer.push(packet(0, 160), 10).unwrap(),
            JitterPush::Buffered
        );
        let mut emitted = Vec::new();
        for now in [60, 80, 100] {
            let Some(JitterOutput::Packet(packet)) = buffer.pop_ready(now) else {
                panic!("expected packet at {now}");
            };
            emitted.push(packet.sequence_number);
        }
        assert_eq!(emitted, [u16::MAX, 0, 1]);
        assert_eq!(buffer.stats().emitted, 3);
    }

    #[test]
    fn emits_explicit_gap_when_missing_packet_reaches_playout() {
        let mut buffer = buffer(8);
        buffer.push(packet(10, 0), 0).unwrap();
        buffer.push(packet(12, 320), 5).unwrap();
        assert!(matches!(
            buffer.pop_ready(60),
            Some(JitterOutput::Packet(_))
        ));
        assert_eq!(
            buffer.pop_ready(100),
            Some(JitterOutput::Gap {
                sequence_number: 11,
                extended_sequence: 11,
                estimated_timestamp: Some(160),
                duration_timestamp_units: Some(160),
            })
        );
        assert!(matches!(
            buffer.pop_ready(100),
            Some(JitterOutput::Packet(_))
        ));
        assert_eq!(buffer.stats().gaps, 1);
    }

    #[test]
    fn bounds_storage_and_classifies_duplicate_and_late_packets() {
        let mut buffer = buffer(2);
        let first = packet(20, 0);
        assert_eq!(buffer.push(first.clone(), 0).unwrap(), JitterPush::Buffered);
        assert_eq!(buffer.push(first, 1).unwrap(), JitterPush::Duplicate);
        assert_eq!(
            buffer.push(packet(21, 160), 2).unwrap(),
            JitterPush::Buffered
        );
        assert_eq!(
            buffer.push(packet(22, 320), 3).unwrap(),
            JitterPush::Overflow
        );
        assert!(matches!(
            buffer.pop_ready(60),
            Some(JitterOutput::Packet(_))
        ));
        assert_eq!(buffer.push(packet(20, 0), 70).unwrap(), JitterPush::Late);
        assert_eq!(
            buffer.stats(),
            JitterStats {
                accepted: 2,
                emitted: 1,
                duplicates: 1,
                late: 1,
                overflow: 1,
                gaps: 0,
            }
        );
    }

    #[test]
    fn l16_and_pcmu_adapters_decode_real_wire_samples() {
        let mut l16 = L16Decoder::new(96, 44_100, 1).unwrap();
        let mut l16_packet = packet(1, 123);
        l16_packet.payload = vec![0x7f, 0xff, 0x80, 0x00];
        let decoded = l16.decode(&l16_packet).unwrap();
        assert_eq!(decoded.samples, [i16::MAX, i16::MIN]);
        assert_eq!((decoded.sample_rate, decoded.channels), (44_100, 1));

        let mut pcmu = PcmuDecoder::new(0, 8_000).unwrap();
        let pcmu_packet = RtpPacket {
            payload_type: 0,
            payload: vec![0xff, 0x7f, 0x80, 0x00],
            ..packet(2, 456)
        };
        let decoded = pcmu.decode(&pcmu_packet).unwrap();
        assert_eq!(decoded.samples, [0, 0, 32_124, -32_124]);
        let concealed = pcmu.conceal(616, 160).unwrap();
        assert_eq!(concealed.timestamp, 616);
        assert_eq!(concealed.samples, vec![0; 160]);
    }

    #[test]
    fn public_probe_crosses_jitter_and_both_codec_adapters() {
        let report = run_jitter_codec_probe().unwrap();
        assert_eq!(report.emitted_packets, 3);
        assert!(report.reordered);
        assert!(report.duplicate_rejected);
        assert_eq!((report.l16_samples, report.pcmu_samples), (3, 2));
    }

    #[test]
    fn adaptive_playout_raises_delay_and_reschedules_queued_packets() {
        let mut buffer = RtpJitterBuffer::new_adaptive(
            JitterBufferConfig {
                ssrc: 7,
                payload_type: 96,
                clock_rate: 1_000,
                playout_delay_ms: 40,
                capacity: 8,
            },
            AdaptivePlayoutConfig {
                max_delay_ms: 120,
                jitter_multiplier: 4,
            },
        )
        .unwrap();
        buffer.push(packet(1, 0), 0).unwrap();
        buffer.push(packet(2, 20), 80).unwrap();
        assert!(buffer.target_delay_ms() > 40);
        assert!(
            buffer.pop_ready(40).is_none(),
            "queued packet was not rescheduled"
        );
        assert!(matches!(
            buffer.pop_ready(buffer.target_delay_ms()),
            Some(JitterOutput::Packet(_))
        ));
    }

    fn description(maps: Vec<crate::RtpMap>) -> SdpDescription {
        SdpDescription {
            origin: "- 1 1 IN IP4 127.0.0.1".into(),
            session_name: "codec test".into(),
            timing: "0 0".into(),
            connection_address: "127.0.0.1".parse().unwrap(),
            media: "audio".into(),
            port: 5000,
            protocol: "UDP/TLS/RTP/SAVPF".into(),
            payload_types: maps.iter().map(|mapping| mapping.payload_type).collect(),
            ice_ufrag: "test".into(),
            ice_pwd: "TestPassword123456789012".into(),
            fingerprint_algorithm: "sha-256".into(),
            fingerprint: "AA".into(),
            setup: "actpass".into(),
            candidates: Vec::new(),
            rtp_maps: maps,
        }
    }

    #[test]
    fn sdp_negotiation_constructs_jitter_and_both_decoders() {
        let maps = vec![
            crate::RtpMap {
                payload_type: 96,
                encoding: "L16".into(),
                clock_rate: 48_000,
                channels: 1,
            },
            crate::RtpMap {
                payload_type: 0,
                encoding: "PCMU".into(),
                clock_rate: 8_000,
                channels: 1,
            },
        ];
        let negotiated = negotiate_audio_codecs(&description(maps.clone()), &description(maps))
            .expect("common codecs");
        assert_eq!(negotiated.codecs().len(), 2);
        assert_eq!(
            negotiated.jitter_config(96, 7, 60, 8).unwrap().clock_rate,
            48_000
        );
        assert_eq!(negotiated.decoder(0).unwrap().payload_type(), 0);
        assert!(negotiated.decoder(111).is_err());
    }

    #[test]
    fn sdp_negotiation_rejects_changed_or_unsupported_codec_sets() {
        let unsupported = crate::RtpMap {
            payload_type: 111,
            encoding: "speex".into(),
            clock_rate: 48_000,
            channels: 2,
        };
        assert!(
            negotiate_audio_codecs(
                &description(vec![unsupported.clone()]),
                &description(vec![unsupported])
            )
            .is_err()
        );

        let offer = crate::RtpMap {
            payload_type: 96,
            encoding: "L16".into(),
            clock_rate: 48_000,
            channels: 1,
        };
        let mut answer = offer.clone();
        answer.clock_rate = 44_100;
        assert!(
            negotiate_audio_codecs(&description(vec![offer]), &description(vec![answer])).is_err()
        );
    }
}
