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
}
