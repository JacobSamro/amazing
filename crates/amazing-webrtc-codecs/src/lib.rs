//! Isolated compressed-codec adapters for the std-only WebRTC core.

#![forbid(unsafe_code)]

use amazing_webrtc::{
    DecodedAudioFrame, NegotiatedAudioCodec, RtpAudioDecoder, RtpPacket, WebRtcError,
};
use opus2::{Application, Channels, Decoder, Encoder};

const OPUS_RATE: u32 = 48_000;
const MAX_FRAME_SAMPLES: usize = 5_760;
const MAX_PACKET_BYTES: usize = 4_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(String);

impl std::fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}

pub struct OpusDecoderAdapter {
    payload_type: u8,
    channels: u16,
    decoder: Decoder,
}

impl OpusDecoderAdapter {
    pub fn new(payload_type: u8, channels: u16) -> Result<Self, CodecError> {
        let channels_value = opus_channels(channels)?;
        let decoder = Decoder::new(OPUS_RATE, channels_value)
            .map_err(|error| CodecError(format!("create Opus decoder: {error}")))?;
        Ok(Self {
            payload_type,
            channels,
            decoder,
        })
    }

    pub fn from_negotiated(codec: &NegotiatedAudioCodec) -> Result<Self, CodecError> {
        if !codec.encoding.eq_ignore_ascii_case("OPUS") || codec.clock_rate != OPUS_RATE {
            return Err(CodecError(
                "negotiated mapping is not supported Opus".into(),
            ));
        }
        Self::new(codec.payload_type, codec.channels)
    }

    fn decode_payload(
        &mut self,
        timestamp: u32,
        payload: &[u8],
        samples_per_channel: usize,
    ) -> Result<DecodedAudioFrame, CodecError> {
        let mut samples = vec![0i16; samples_per_channel.saturating_mul(self.channels as usize)];
        let decoded = self
            .decoder
            .decode(payload, &mut samples, false)
            .map_err(|error| CodecError(format!("decode Opus packet: {error}")))?;
        samples.truncate(decoded.saturating_mul(self.channels as usize));
        Ok(DecodedAudioFrame {
            timestamp,
            sample_rate: OPUS_RATE,
            channels: self.channels,
            samples,
        })
    }
}

impl RtpAudioDecoder for OpusDecoderAdapter {
    fn payload_type(&self) -> u8 {
        self.payload_type
    }

    fn decode(&mut self, packet: &RtpPacket) -> Result<DecodedAudioFrame, WebRtcError> {
        if packet.payload_type != self.payload_type {
            return Err(WebRtcError::InvalidPacket(
                "RTP packet does not match Opus payload type",
            ));
        }
        self.decode_payload(packet.timestamp, &packet.payload, MAX_FRAME_SAMPLES)
            .map_err(|_| WebRtcError::InvalidPacket("Opus packet decode failed"))
    }

    fn conceal(
        &mut self,
        timestamp: u32,
        samples_per_channel: usize,
    ) -> Result<DecodedAudioFrame, WebRtcError> {
        if samples_per_channel == 0 || samples_per_channel > MAX_FRAME_SAMPLES {
            return Err(WebRtcError::InvalidPacket(
                "Opus concealment duration is invalid",
            ));
        }
        self.decode_payload(timestamp, &[], samples_per_channel)
            .map_err(|_| WebRtcError::InvalidPacket("Opus packet concealment failed"))
    }
}

pub struct OpusEncoderAdapter {
    payload_type: u8,
    channels: u16,
    encoder: Encoder,
}

impl OpusEncoderAdapter {
    pub fn new(payload_type: u8, channels: u16) -> Result<Self, CodecError> {
        let channels_value = opus_channels(channels)?;
        let encoder = Encoder::new(OPUS_RATE, channels_value, Application::Voip)
            .map_err(|error| CodecError(format!("create Opus encoder: {error}")))?;
        Ok(Self {
            payload_type,
            channels,
            encoder,
        })
    }

    pub fn from_negotiated(codec: &NegotiatedAudioCodec) -> Result<Self, CodecError> {
        if !codec.encoding.eq_ignore_ascii_case("OPUS") || codec.clock_rate != OPUS_RATE {
            return Err(CodecError(
                "negotiated mapping is not supported Opus".into(),
            ));
        }
        Self::new(codec.payload_type, codec.channels)
    }

    pub fn encode(
        &mut self,
        sequence_number: u16,
        timestamp: u32,
        ssrc: u32,
        samples: &[i16],
    ) -> Result<RtpPacket, CodecError> {
        if samples.is_empty() || samples.len() % self.channels as usize != 0 {
            return Err(CodecError(
                "Opus PCM does not contain whole channel frames".into(),
            ));
        }
        let samples_per_channel = samples.len() / self.channels as usize;
        if !matches!(samples_per_channel, 120 | 240 | 480 | 960 | 1_920 | 2_880) {
            return Err(CodecError("Opus frame duration is unsupported".into()));
        }
        let mut payload = vec![0u8; MAX_PACKET_BYTES];
        let encoded = self
            .encoder
            .encode(samples, &mut payload)
            .map_err(|error| CodecError(format!("encode Opus frame: {error}")))?;
        payload.truncate(encoded);
        Ok(RtpPacket {
            marker: false,
            payload_type: self.payload_type,
            sequence_number,
            timestamp,
            ssrc,
            payload,
        })
    }
}

fn opus_channels(channels: u16) -> Result<Channels, CodecError> {
    match channels {
        1 => Ok(Channels::Mono),
        2 => Ok(Channels::Stereo),
        _ => Err(CodecError("Opus supports one or two channels".into())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpusProbeReport {
    pub encoded_bytes: usize,
    pub decoded_samples: usize,
    pub concealed_samples: usize,
}

pub fn run_opus_probe() -> Result<OpusProbeReport, CodecError> {
    let pcm: Vec<i16> = (0..960)
        .map(|index| (((index % 96) - 48) * 400) as i16)
        .collect();
    let mut encoder = OpusEncoderAdapter::new(111, 1)?;
    let packet = encoder.encode(1, 0, 0x4f50_5553, &pcm)?;
    let encoded_bytes = packet.payload.len();
    let mut decoder = OpusDecoderAdapter::new(111, 1)?;
    let decoded = decoder
        .decode(&packet)
        .map_err(|error| CodecError(error.to_string()))?;
    let concealed = decoder
        .conceal(960, 960)
        .map_err(|error| CodecError(error.to_string()))?;
    Ok(OpusProbeReport {
        encoded_bytes,
        decoded_samples: decoded.samples.len(),
        concealed_samples: concealed.samples.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVideoFrame {
    pub width: u32,
    pub height: u32,
    pub planes: usize,
    pub checksum: u64,
}

pub struct Vp8DecoderAdapter {
    decoder: media_codec::decoder::DecoderContext<media_codec::decoder::VideoDecoder>,
}

impl Vp8DecoderAdapter {
    pub fn new() -> Result<Self, CodecError> {
        let params = media_codec::CodecParameters::new(
            media_codec::VideoParameters::default(),
            media_codec::decoder::DecoderParameters::default(),
        );
        let decoder = media_codec_vpx::decoder::VpxDecoder::new(
            media_codec::CodecID::VP8,
            &media_codec::decoder::VideoDecoderParameters::default(),
            None,
        )
        .map_err(|error| CodecError(format!("create VP8 decoder: {error}")))?;
        let decoder =
            media_codec::decoder::DecoderContext::new_with_decoder(Box::new(decoder), &params)
                .map_err(|error| CodecError(format!("configure VP8 decoder: {error}")))?;
        Ok(Self { decoder })
    }

    pub fn decode(&mut self, payload: &[u8], pts: i64) -> Result<DecodedVideoFrame, CodecError> {
        let mut packet = media_codec::packet::Packet::from_slice(payload);
        packet.pts = Some(pts);
        self.decoder
            .send_packet(&packet)
            .map_err(|error| CodecError(format!("send VP8 packet: {error}")))?;
        let shared = self
            .decoder
            .receive_frame()
            .map_err(|error| CodecError(format!("receive VP8 frame: {error}")))?;
        let frame = shared.read();
        let descriptor = frame.descriptor();
        let mapped = frame
            .map()
            .map_err(|error| CodecError(format!("map VP8 frame: {error}")))?;
        let planes = mapped
            .planes()
            .ok_or_else(|| CodecError("VP8 frame has no mapped planes".into()))?;
        let mut checksum = 0u64;
        for plane in planes.iter() {
            for byte in plane.data().unwrap_or_default() {
                checksum = checksum.rotate_left(5) ^ *byte as u64;
            }
        }
        Ok(DecodedVideoFrame {
            width: descriptor.width().get(),
            height: descriptor.height().get(),
            planes: planes.len(),
            checksum,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8ProbeReport {
    pub frames: usize,
    pub interframes: usize,
    pub width: u32,
    pub height: u32,
    pub checksum_changed: bool,
}

pub fn run_vp8_probe(ivf: &[u8]) -> Result<Vp8ProbeReport, CodecError> {
    let frames = parse_ivf(ivf)?;
    let mut decoder = Vp8DecoderAdapter::new()?;
    let mut decoded = 0usize;
    let mut interframes = 0usize;
    let mut first_checksum = None;
    let mut checksum_changed = false;
    let mut dimensions = None;
    for (pts, payload) in frames.into_iter().take(12) {
        if payload.first().is_some_and(|byte| byte & 1 != 0) {
            interframes += 1;
        }
        let frame = decoder.decode(payload, pts as i64)?;
        dimensions.get_or_insert((frame.width, frame.height));
        if dimensions != Some((frame.width, frame.height)) || frame.planes < 3 {
            return Err(CodecError("VP8 decoded inconsistent frame geometry".into()));
        }
        match first_checksum {
            Some(first) if first != frame.checksum => checksum_changed = true,
            None => first_checksum = Some(frame.checksum),
            _ => {}
        }
        decoded += 1;
    }
    let (width, height) = dimensions.ok_or_else(|| CodecError("IVF had no frames".into()))?;
    Ok(Vp8ProbeReport {
        frames: decoded,
        interframes,
        width,
        height,
        checksum_changed,
    })
}

fn parse_ivf(bytes: &[u8]) -> Result<Vec<(u64, &[u8])>, CodecError> {
    if bytes.len() < 32 || &bytes[..4] != b"DKIF" || &bytes[8..12] != b"VP80" {
        return Err(CodecError("unsupported or truncated VP8 IVF header".into()));
    }
    let mut cursor = 32usize;
    let mut frames = Vec::new();
    while cursor < bytes.len() {
        let header = bytes
            .get(cursor..cursor + 12)
            .ok_or_else(|| CodecError("truncated IVF frame header".into()))?;
        let size = u32::from_le_bytes(header[..4].try_into().expect("IVF size")) as usize;
        let pts = u64::from_le_bytes(header[4..12].try_into().expect("IVF timestamp"));
        cursor += 12;
        let payload = bytes
            .get(cursor..cursor.saturating_add(size))
            .ok_or_else(|| CodecError("truncated IVF frame payload".into()))?;
        frames.push((pts, payload));
        cursor += size;
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_roundtrip_and_plc_cross_the_core_codec_seam() {
        let report = run_opus_probe().expect("Opus probe");
        assert!(report.encoded_bytes > 0 && report.encoded_bytes < 1_920);
        assert_eq!(report.decoded_samples, 960);
        assert_eq!(report.concealed_samples, 960);
    }

    #[test]
    fn vp8_keyframe_and_interframes_decode_to_pixels() {
        let report = run_vp8_probe(include_bytes!("../../../examples/vp80-02-inter-1402.ivf"))
            .expect("VP8 probe");
        assert!(report.frames >= 2);
        assert!(report.interframes >= 1);
        assert!(report.width > 0 && report.height > 0);
        assert!(report.checksum_changed);
    }
}
