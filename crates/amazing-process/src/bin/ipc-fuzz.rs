//! Deterministic mutation driver for the production IPC wire surface.
//! Run on a build host: ipc-fuzz --target decoders|dispatchers --fuzz-seconds 600
use amazing_ipc::codec::{self, Wire};
use amazing_ipc::version::VersionRange;
use amazing_ipc::{interface, Dispatcher, Packet};
use amazing_process::protocol::*;
use amazing_process::{Capabilities, Handshake, Ping};
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::time::{Duration, Instant};

type Target = (&'static str, fn(&[u8]) -> u64);
interface! {interface DemoEcho(17) => DemoEchoProxy, DemoEchoReceiver {fn echo(message:String) -> String = 1;}}
interface! {interface SandboxProbe(18) => SandboxProbeProxy, SandboxProbeReceiver {fn probe(message:String) -> String = 1;}}
fn decode<T: Wire>(bytes: &[u8]) -> u64 {
    match codec::decode::<T>(bytes) {
        Ok(_) => 0,
        Err(codec::Error::Eof) => 1,
        Err(codec::Error::Version) => 2,
        Err(codec::Error::Limit) => 3,
        Err(codec::Error::Invalid) => 4,
        Err(codec::Error::Utf8) => 5,
        Err(codec::Error::Trailing) => 6,
    }
}
macro_rules! decoder_targets {
    ($($ty:ty),+ $(,)?) => { &[$((stringify!($ty), decode::<$ty> as fn(&[u8]) -> u64)),+] };
}
const DECODERS: &[Target] = decoder_targets![
    Packet,
    Handshake,
    Capabilities,
    VersionRange,
    Viewport,
    Navigate,
    Frame,
    FrameReady,
    SharedFrameReady,
    ReleaseFrame,
    AcquireFrame,
    AcquireResult,
    SadTab,
    Point,
    ScrollDelta,
    InputEvent,
    InputAck,
    StartRequest,
    Started,
    RequestId,
    ResponseHead,
    BodyChunk,
    Completion,
    NetworkEvent,
    HeaderField,
    WireOrigin,
    WireCookieSite,
    NetworkContext,
    NetworkStart,
    NetworkStarted,
    NetworkRead,
    NetworkCancel,
    NetworkEpoch,
    NetworkHead,
    NetworkBody,
    NetworkComplete,
    NetworkReply,
    GpuFrame,
    DecodeRequest,
    DecodedReady,
    StoragePartition,
    StorageEntry,
    StoragePut,
    HistoryActivate,
    HistoryFreeze,
    HistoryRestore,
    HistoryDiscard,
    HistoryLifecycleEvent,
    HistoryAck,
    ChannelRegistration,
    BroadcastPost,
    MessageDelivery,
    PortPair,
    PortPost,
    PortStart,
    PortClose,
];
fn snapshot_decode(bytes: &[u8]) -> u64 {
    match amazing_process::gpu_snapshot_wire::decode_snapshot(bytes) {
        Ok(snapshot) => {
            u64::from(amazing_compositor::CompositorFrame::from_snapshot(snapshot).is_err()) * 7
        }
        Err(error) => decode_error(error),
    }
}
fn utility_payloads(bytes: &[u8]) -> u64 {
    if bytes.len() > codec::MAX_FRAME - 4096 {
        return 3;
    }
    let image = if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'<') {
        amazing_svg::decode(bytes, amazing_svg::DecodeLimits::default()).is_ok()
    } else {
        amazing_media::decode_image(bytes).is_ok()
    };
    let audio = amazing_media::decode_wav(bytes).is_ok();
    let font = match bytes.get(..4) {
        Some(b"wOFF") => Some(amazing_text_provider::FontFormat::Woff),
        Some(b"wOF2") => Some(amazing_text_provider::FontFormat::Woff2),
        Some(b"OTTO") | Some(b"true") | Some([0, 1, 0, 0]) => {
            Some(amazing_text_provider::FontFormat::OpenType)
        }
        _ => None,
    };
    let font = font.is_some_and(|format| {
        amazing_text_provider::decode_web_font_to_sfnt(format, bytes).is_ok()
    });
    u64::from(image) | (u64::from(audio) << 1) | (u64::from(font) << 2)
}
fn requests(
    bytes: &[u8],
    validate: fn(u16, &[u8]) -> Result<(), amazing_ipc::Error>,
    max: u16,
) -> u64 {
    let mut signature = 0u64;
    for method in 0..=max + 1 {
        let result = validate(method, bytes);
        let code = match result {
            Ok(()) => 0,
            Err(amazing_ipc::Error::Codec(error)) => decode_error(error),
            Err(amazing_ipc::Error::UnknownMethod) => 8,
            Err(_) => 9,
        };
        signature = signature.wrapping_mul(17).wrapping_add(code + 1);
    }
    signature
}
fn decode_error(error: codec::Error) -> u64 {
    match error {
        codec::Error::Eof => 1,
        codec::Error::Version => 2,
        codec::Error::Limit => 3,
        codec::Error::Invalid => 4,
        codec::Error::Utf8 => 5,
        codec::Error::Trailing => 6,
    }
}
fn renderer(bytes: &[u8]) -> u64 {
    requests(bytes, RendererProxy::validate_request, 6)
}
fn network(bytes: &[u8]) -> u64 {
    requests(bytes, NetworkProxy::validate_request, 3)
}
fn network_v2(bytes: &[u8]) -> u64 {
    requests(bytes, NetworkProxyV2::validate_request, 5)
}
fn events(bytes: &[u8]) -> u64 {
    requests(bytes, BrowserEventsProxy::validate_request, 1)
}
fn gpu(bytes: &[u8]) -> u64 {
    requests(bytes, GpuProxy::validate_request, 1)
}
fn utility(bytes: &[u8]) -> u64 {
    requests(bytes, UtilityProxy::validate_request, 4)
}
fn storage(bytes: &[u8]) -> u64 {
    requests(bytes, StorageProxy::validate_request, 4)
}
fn history(bytes: &[u8]) -> u64 {
    requests(bytes, HistoryProxy::validate_request, 4)
}
fn messaging(bytes: &[u8]) -> u64 {
    requests(bytes, MessagingProxy::validate_request, 8)
}
fn demo_echo(bytes: &[u8]) -> u64 {
    requests(bytes, DemoEchoProxy::validate_request, 1)
}
fn sandbox_probe(bytes: &[u8]) -> u64 {
    requests(bytes, SandboxProbeProxy::validate_request, 1)
}
fn ping(bytes: &[u8]) -> u64 {
    let mut signature = 0;
    for method in 0..=1 {
        signature = signature * 2 + u64::from(Ping.dispatch(method, bytes).is_ok());
    }
    signature
}
const DISPATCHERS: &[Target] = &[
    ("RendererControls", renderer),
    ("NetworkRequests", network),
    ("NetworkRequestsV2", network_v2),
    ("BrowserEvents", events),
    ("GpuPresentation", gpu),
    ("UtilityDecoder", utility),
    ("StorageService", storage),
    ("HistoryLifecycle", history),
    ("MessagingRequests", messaging),
    ("DemoEcho", demo_echo),
    ("SandboxProbe", sandbox_probe),
    ("Ping", ping),
];

fn seed_bytes(hex: &str) -> Vec<u8> {
    let digits: String = hex.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    digits
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
const SEEDS: &[&str] = &[
    include_str!("../../fuzz/corpus/empty.hex"),
    include_str!("../../fuzz/corpus/version.hex"),
    include_str!("../../fuzz/corpus/length-overflow.hex"),
    include_str!("../../fuzz/corpus/packet.hex"),
];
fn valid_seeds() -> Vec<Vec<u8>> {
    let mut seeds = vec![
        codec::encode(&Packet {
            interface: 11,
            id: 1,
            kind: amazing_ipc::REQUEST,
            payload: vec![1, 2, 3],
        })
        .unwrap(),
        codec::encode(&Handshake {
            version: amazing_process::BOOTSTRAP_VERSION,
            role: "gpu".into(),
            capabilities: Capabilities(BTreeMap::new()),
            interfaces: vec![VersionRange {
                interface: 11,
                min: 1,
                max: 1,
            }],
        })
        .unwrap(),
        codec::encode(&Navigate {
            url: "http://example.test/".into(),
            html_bytes: codec::ByteBuf(b"<p>seed</p>".to_vec()),
            viewport: Viewport {
                width: 16,
                height: 16,
            },
        })
        .unwrap(),
        codec::encode(&GpuFrame {
            sequence: 1,
            snapshot: codec::ByteBuf(vec![1, 0]),
        })
        .unwrap(),
        codec::encode(&DecodeRequest {
            kind: amazing_process::utility::IMAGE,
            bytes: codec::ByteBuf(b"P6\n1 1\n255\n\x10\x20\x30".to_vec()),
        })
        .unwrap(),
        codec::encode(&StoragePut {
            entry: StorageEntry {
                partition: StoragePartition {
                    origin: "https://a.test".into(),
                    top_level_site: "https://a.test".into(),
                    bucket: "default".into(),
                },
                namespace: "local".into(),
                key: codec::ByteBuf(b"key".to_vec()),
            },
            value: codec::ByteBuf(b"value".to_vec()),
        })
        .unwrap(),
        amazing_process::gpu_snapshot_wire::encode_snapshot(
            &amazing_compositor::CompositorFrameSnapshot {
                version: amazing_compositor::SNAPSHOT_VERSION,
                width: 16,
                height: 16,
                passes: vec![amazing_compositor::RenderPass {
                    id: 0,
                    effect_node: None,
                    clip_node: None,
                    quads: Vec::new(),
                }],
                resources: amazing_compositor::ResourceDelta::default().to_snapshot(),
                damage: Vec::new(),
                trees: amazing_compositor::PropertyTrees::new().to_snapshot(),
                tile_lookup: Vec::new(),
            },
        )
        .unwrap(),
    ];
    seeds.push(include_bytes!("../../tests/fixtures/synthetic-a.ttf").to_vec());
    seeds.push(b"P6\n1 1\n255\n\x10\x20\x30".to_vec());
    seeds.push(b"RIFF\x26\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x11\x2b\x00\x00\x11\x2b\x00\x00\x01\x00\x08\x00data\x02\x00\x00\x00\x00\xff".to_vec());
    seeds
}
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
fn mutate(input: &[u8], state: &mut u64) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + next(state) % 4 {
        let position = (next(state) as usize) % (out.len() + 1);
        let existing = position.min(out.len().saturating_sub(1));
        match next(state) % 5 {
            0 if !out.is_empty() => {
                out.remove(existing);
            }
            1 if out.len() < 4096 => out.insert(position, next(state) as u8),
            2 if !out.is_empty() => out[existing] ^= 1 << (next(state) % 8),
            3 if !out.is_empty() => out[existing] = next(state) as u8,
            _ if out.len() < 4092 => out
                .splice(position..position, (next(state) as u32).to_le_bytes())
                .for_each(drop),
            _ => (),
        }
    }
    out
}
fn signature(targets: &[Target], bytes: &[u8]) -> Vec<u64> {
    targets
        .iter()
        .map(
            |(_, check)| match std::panic::catch_unwind(|| check(bytes)) {
                Ok(value) => value,
                Err(_) => {
                    let dir = std::path::Path::new("target/ipc-fuzz-crashes");
                    std::fs::create_dir_all(dir).unwrap();
                    let name = format!(
                        "repro-{:016x}.hex",
                        bytes
                            .iter()
                            .fold(0u64, |h, b| h.wrapping_mul(1099511628211) ^ u64::from(*b))
                    );
                    std::fs::write(
                        dir.join(&name),
                        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                    )
                    .unwrap();
                    panic!("IPC decoder panic; saved {}", dir.join(name).display());
                }
            },
        )
        .collect()
}
fn minimize(mut bytes: Vec<u8>, target: &[u64], targets: &[Target]) -> Vec<u8> {
    let mut chunk = bytes.len() / 2;
    while chunk > 0 {
        let mut offset = 0;
        while offset + chunk <= bytes.len() {
            let mut candidate = bytes.clone();
            candidate.drain(offset..offset + chunk);
            if signature(targets, &candidate) == target {
                bytes = candidate;
            } else {
                offset += chunk;
            }
        }
        chunk /= 2;
    }
    bytes
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let target = args
        .windows(2)
        .find(|pair| pair[0] == "--target")
        .map(|pair| pair[1].as_str())
        .unwrap_or("decoders");
    let seconds: u64 = args
        .windows(2)
        .find(|pair| pair[0] == "--fuzz-seconds")
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(1);
    let targets: Vec<Target> = match target {
        "decoders" => DECODERS
            .iter()
            .copied()
            .chain(std::iter::once((
                "CompositorFrameSnapshot",
                snapshot_decode as fn(&[u8]) -> u64,
            )))
            .chain(std::iter::once((
                "UtilityPayloads",
                utility_payloads as fn(&[u8]) -> u64,
            )))
            .collect(),
        "dispatchers" => DISPATCHERS.to_vec(),
        _ => panic!("unknown target {target}"),
    };
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut corpus: Vec<Vec<u8>> = SEEDS.iter().map(|s| seed_bytes(s)).collect();
    corpus.extend(valid_seeds());
    let mut seen = HashSet::new();
    let mut state = 0x1a2b_3c4d_5e6f_7890u64;
    let mut cases = 0u64;
    for seed in &corpus {
        seen.insert(signature(&targets, seed));
    }
    let oversized = vec![0; codec::MAX_FRAME + 1];
    let _ = signature(&targets, &oversized);
    while Instant::now() < deadline {
        let index = next(&mut state) as usize % corpus.len();
        let bytes = mutate(&corpus[index], &mut state);
        let result = signature(&targets, &bytes);
        if seen.insert(result.clone()) && corpus.len() < 256 {
            corpus.push(minimize(bytes, &result, &targets));
        }
        cases += 1;
    }
    println!("target={target} seconds={seconds} cases={cases} decoders={} signals={} corpus={} panics=0 max_input={}", targets.len(), seen.len(), corpus.len(), codec::MAX_FRAME + 1);
}
