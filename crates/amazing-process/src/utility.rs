//! Sandboxed untrusted byte decoding with immutable shared output.
use crate::protocol::{
    AcquireFrame, AcquireResult, DecodeRequest, DecodedReady, ReleaseFrame, SharedFrameReady,
    UtilityDecoder, UtilityReceiver,
};
use amazing_ipc::codec;
use amazing_ipc::{Connection, Error};
use amazing_ipc_platform::{FrameHandle, HandleChannel, MAX_DIMENSION};
use std::sync::{Arc, Mutex};

pub const IMAGE: u8 = 1;
pub const FONT: u8 = 2;
pub const AUDIO: u8 = 3;
const MAX_INPUT: usize = codec::MAX_FRAME - 4096;
const MAX_OUTPUT: usize = 64 * 1024 * 1024;

struct Decoded {
    kind: u8,
    width: u32,
    height: u32,
    sample_rate: u32,
    channels: u16,
    bytes: Vec<u8>,
}
fn decode(request: DecodeRequest) -> Result<Decoded, Error> {
    if request.bytes.0.is_empty() || request.bytes.0.len() > MAX_INPUT {
        return Err(Error::Codec(codec::Error::Limit));
    }
    let bytes = &request.bytes.0;
    let result = match request.kind {
        IMAGE => {
            let image = if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'<') {
                amazing_svg::decode(bytes, amazing_svg::DecodeLimits::default())
                    .map_err(|e| Error::Remote(format!("image: {e}")))?
            } else {
                amazing_media::decode_image(bytes)
                    .map_err(|e| Error::Remote(format!("image: {e}")))?
            };
            Decoded {
                kind: IMAGE,
                width: image.width,
                height: image.height,
                sample_rate: 0,
                channels: 0,
                bytes: image.pixels,
            }
        }
        AUDIO => {
            let audio = amazing_media::decode_wav(bytes)
                .map_err(|e| Error::Remote(format!("audio: {e}")))?;
            let byte_len = audio
                .samples
                .len()
                .checked_mul(2)
                .filter(|n| *n <= MAX_OUTPUT)
                .ok_or(Error::Codec(codec::Error::Limit))?;
            let mut samples = Vec::with_capacity(byte_len);
            for sample in audio.samples {
                samples.extend_from_slice(&sample.to_le_bytes());
            }
            Decoded {
                kind: AUDIO,
                width: 0,
                height: 0,
                sample_rate: audio.sample_rate,
                channels: audio.channels,
                bytes: samples,
            }
        }
        FONT => {
            let format = match bytes.get(..4) {
                Some(b"wOFF") => amazing_text_provider::FontFormat::Woff,
                Some(b"wOF2") => amazing_text_provider::FontFormat::Woff2,
                Some(b"OTTO") | Some(b"true") | Some([0, 1, 0, 0]) => {
                    amazing_text_provider::FontFormat::OpenType
                }
                _ => return Err(Error::Codec(codec::Error::Invalid)),
            };
            let sfnt = amazing_text_provider::decode_web_font_to_sfnt(format, bytes)
                .map_err(|e| Error::Remote(format!("font: {e:?}")))?;
            Decoded {
                kind: FONT,
                width: 0,
                height: 0,
                sample_rate: 0,
                channels: 0,
                bytes: sfnt,
            }
        }
        _ => return Err(Error::Codec(codec::Error::Invalid)),
    };
    if result.bytes.is_empty() || result.bytes.len() > MAX_OUTPUT {
        return Err(Error::Codec(codec::Error::Limit));
    }
    Ok(result)
}

struct Pending {
    handle: FrameHandle,
    generation: u64,
    in_flight: bool,
}
#[derive(Default)]
struct State {
    sequence: u64,
    pending: Option<Pending>,
}
struct Endpoint {
    channel: HandleChannel,
    state: Mutex<State>,
}
impl UtilityDecoder for Endpoint {
    fn decode(&self, request: DecodeRequest) -> Result<DecodedReady, Error> {
        let result = decode(request)?;
        let mut state = self.state.lock().unwrap();
        if state.pending.as_ref().is_some_and(|p| p.in_flight) {
            return Err(Error::Remote("output transfer busy".into()));
        }
        let generation = state
            .sequence
            .checked_add(1)
            .ok_or(Error::Codec(codec::Error::Limit))?;
        let payload_len =
            u32::try_from(result.bytes.len()).map_err(|_| Error::Codec(codec::Error::Limit))?;
        let width = ((result.bytes.len().div_ceil(4)).min(MAX_DIMENSION as usize)) as u32;
        let row = width as usize * 4;
        let height = result.bytes.len().div_ceil(row) as u32;
        if height == 0 || height > MAX_DIMENSION {
            return Err(Error::Codec(codec::Error::Limit));
        }
        let mut packed = vec![0; row * height as usize];
        packed[..result.bytes.len()].copy_from_slice(&result.bytes);
        let handle = FrameHandle::new(generation, width, height, &packed)
            .map_err(|e| Error::Remote(format!("shared output: {e}")))?;
        let shared = SharedFrameReady::from(handle.metadata());
        state.sequence = generation;
        state.pending = Some(Pending {
            handle,
            generation,
            in_flight: false,
        });
        Ok(DecodedReady {
            kind: result.kind,
            width: result.width,
            height: result.height,
            sample_rate: result.sample_rate,
            channels: result.channels,
            payload_len,
            shared,
        })
    }
    fn acquire(&self, request: AcquireFrame) -> Result<AcquireResult, Error> {
        let mut state = self.state.lock().unwrap();
        let latest = state.sequence;
        let Some(pending) = state.pending.as_mut() else {
            return Ok(AcquireResult::Superseded(latest));
        };
        if pending.generation != request.generation {
            return Ok(AcquireResult::Superseded(latest));
        }
        if pending.in_flight {
            return Ok(AcquireResult::Busy(()));
        }
        pending
            .handle
            .publish(&self.channel)
            .map_err(|e| Error::Remote(format!("shared output: {e}")))?;
        pending.in_flight = true;
        Ok(AcquireResult::Transferred(()))
    }
    fn received(&self, request: AcquireFrame) -> Result<bool, Error> {
        let mut state = self.state.lock().unwrap();
        let Some(pending) = state.pending.as_mut() else {
            return Ok(false);
        };
        if pending.generation != request.generation || !pending.in_flight {
            return Ok(false);
        }
        pending.in_flight = false;
        Ok(true)
    }
    fn release(&self, request: ReleaseFrame) -> Result<bool, Error> {
        let mut state = self.state.lock().unwrap();
        if state
            .pending
            .as_ref()
            .is_some_and(|p| p.generation == request.generation && !p.in_flight)
        {
            state.pending = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
pub fn serve_utility(connection: &Arc<Connection>, channel: Option<HandleChannel>) {
    if let Some(channel) = channel {
        connection.bind(
            12,
            UtilityReceiver(Endpoint {
                channel,
                state: Mutex::new(State::default()),
            }),
        );
        connection.bind(0, crate::Ping);
    }
}
