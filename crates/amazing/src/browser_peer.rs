use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 256 * 1024;
const FRAME_COUNT: u32 = 45;
const FRAME_INTERVAL_MS: u64 = 100;
const VIDEO_SSRC: u32 = 0x4252_5752;
const MAX_RTP_PAYLOAD: usize = 1_200;

pub struct BrowserPeerReport {
    pub http_address: SocketAddrV4,
    pub browser_address: SocketAddrV4,
    pub local_media_address: SocketAddrV4,
    pub payload_type: u8,
    pub frames_sent: u32,
    pub rtp_packets_sent: usize,
    pub browser_result: String,
}

struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

pub fn run_browser_peer(http_port: u16) -> Result<BrowserPeerReport, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, http_port))
        .map_err(|error| format!("bind browser signaling HTTP: {error}"))?;
    let http_address = match listener.local_addr().map_err(|error| error.to_string())? {
        SocketAddr::V4(address) => address,
        SocketAddr::V6(_) => return Err("browser signaling requires IPv4".into()),
    };
    println!("BROWSER_URL http://{http_address}/");
    std::io::stdout()
        .flush()
        .map_err(|error| error.to_string())?;

    let media_socket =
        UdpSocket::bind("0.0.0.0:0").map_err(|error| format!("bind browser media UDP: {error}"))?;
    let identity = amazing_webrtc_crypto::SecureIdentity::generate("amazing-browser-peer")
        .map_err(|error| error.to_string())?;

    for _ in 0..12 {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("accept browser HTTP: {error}"))?;
        configure_http_stream(&stream)?;
        let request = read_http_request(&mut stream)?;
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/") => write_http_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                BROWSER_PAGE.as_bytes(),
            )?,
            ("POST", "/offer") => {
                let offer_source = String::from_utf8(request.body)
                    .map_err(|_| "browser offer is not UTF-8".to_string())?;
                let offer = amazing_webrtc::parse_sdp(&offer_source)
                    .map_err(|error| format!("parse live browser offer: {error}"))?;
                let browser_candidate = offer
                    .primary_media()
                    .map_err(|error| error.to_string())?
                    .candidates
                    .first()
                    .ok_or_else(|| "browser offer has no usable IPv4 ICE candidate".to_string())?
                    .address;
                media_socket
                    .connect(browser_candidate)
                    .map_err(|error| format!("connect browser media UDP: {error}"))?;
                let local_media_address = crate::std_socket_v4(&media_socket)?;
                let answer = amazing_webrtc_crypto::build_local_answer(
                    &offer,
                    local_media_address,
                    identity.fingerprint(),
                    "browserpeer",
                    "amaz",
                    "AmazingBrowserPassword1234",
                    901,
                )
                .map_err(|error| error.to_string())?;
                let answer_source =
                    amazing_webrtc::encode_sdp(&answer).map_err(|error| error.to_string())?;
                write_http_response(
                    &mut stream,
                    "200 OK",
                    "application/sdp",
                    answer_source.as_bytes(),
                )?;
                drop(stream);
                return establish_and_stream(
                    listener,
                    http_address,
                    media_socket,
                    identity,
                    offer,
                    answer,
                    browser_candidate,
                    local_media_address,
                );
            }
            _ => write_http_response(&mut stream, "404 Not Found", "text/plain", b"not found")?,
        }
    }
    Err("browser did not submit an SDP offer".into())
}

#[allow(clippy::too_many_arguments)]
fn establish_and_stream(
    listener: TcpListener,
    http_address: SocketAddrV4,
    media_socket: UdpSocket,
    identity: amazing_webrtc_crypto::SecureIdentity,
    offer: amazing_webrtc::SdpDescription,
    answer: amazing_webrtc::SdpDescription,
    browser_address: SocketAddrV4,
    local_media_address: SocketAddrV4,
) -> Result<BrowserPeerReport, String> {
    let video = answer
        .media("video")
        .ok_or_else(|| "browser answer has no video media".to_string())?;
    let payload_type = video
        .rtp_maps
        .iter()
        .find(|mapping| mapping.encoding.eq_ignore_ascii_case("H264"))
        .ok_or_else(|| "browser answer has no H.264 mapping".to_string())?
        .payload_type;
    let mut endpoint = amazing_webrtc_crypto::SecureUdpEndpoint::establish(
        amazing_webrtc_crypto::SecureEndpointConfig {
            socket: media_socket,
            identity,
            local_description: answer,
            remote_description: offer,
            local_is_offerer: false,
            ice_role: amazing_webrtc_crypto::IceRole::Controlled,
        },
    )
    .map_err(|error| error.to_string())?;

    let mut encoder =
        amazing_webrtc_codecs::H264EncoderAdapter::new().map_err(|error| error.to_string())?;
    let mut sequence = 20_000u16;
    let mut timestamp = 90_000u32;
    let mut packet_count = 0usize;
    for phase in 0..FRAME_COUNT {
        let rgb = browser_frame_rgb(phase);
        let encoded = encoder
            .encode_rgb(&rgb, (320, 180))
            .map_err(|error| error.to_string())?;
        let packets = amazing_webrtc::packetize_h264_access_unit(
            &encoded,
            amazing_webrtc::H264PacketizationConfig {
                payload_type,
                first_sequence_number: sequence,
                timestamp,
                ssrc: VIDEO_SSRC,
                max_payload_bytes: MAX_RTP_PAYLOAD,
            },
        )
        .map_err(|error| error.to_string())?;
        for packet in &packets {
            endpoint
                .send_rtp(&amazing_webrtc::encode_rtp(packet).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        }
        packet_count = packet_count.saturating_add(packets.len());
        sequence = sequence.wrapping_add(packets.len() as u16);
        timestamp = timestamp.wrapping_add(9_000);
        std::thread::sleep(Duration::from_millis(FRAME_INTERVAL_MS));
    }

    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure browser result listener: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(12);
    let browser_result = loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                configure_http_stream(&stream)?;
                let request = read_http_request(&mut stream)?;
                if request.method == "POST" && request.path == "/result" {
                    write_http_response(&mut stream, "204 No Content", "text/plain", b"")?;
                    break String::from_utf8(request.body)
                        .map_err(|_| "browser result is not UTF-8".to_string())?;
                }
                write_http_response(&mut stream, "404 Not Found", "text/plain", b"not found")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("browser did not report decoded media".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(format!("accept browser result: {error}")),
        }
    };
    endpoint.close().map_err(|error| error.to_string())?;
    Ok(BrowserPeerReport {
        http_address,
        browser_address,
        local_media_address,
        payload_type,
        frames_sent: FRAME_COUNT,
        rtp_packets_sent: packet_count,
        browser_result,
    })
}

fn browser_frame_rgb(phase: u32) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(320 * 180 * 3);
    for y in 0..180u32 {
        for x in 0..320u32 {
            rgb.extend_from_slice(&[
                (x.wrapping_mul(3).wrapping_add(phase * 11)) as u8,
                (y.wrapping_mul(5).wrapping_add(phase * 17)) as u8,
                ((x + y).wrapping_mul(2).wrapping_add(phase * 23)) as u8,
            ]);
        }
    }
    rgb
}

fn configure_http_stream(stream: &TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() >= MAX_HTTP_HEADER_BYTES {
            return Err("browser HTTP headers exceed the cap".into());
        }
        let mut chunk = [0; 1_024];
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("browser HTTP request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| "browser HTTP headers are not UTF-8".to_string())?;
    let mut lines = headers[..headers.len() - 4].split("\r\n");
    let request = lines
        .next()
        .ok_or_else(|| "browser HTTP request line is missing".to_string())?;
    let fields: Vec<&str> = request.split_ascii_whitespace().collect();
    if fields.len() != 3 || fields[2] != "HTTP/1.1" {
        return Err("browser HTTP request line is invalid".into());
    }
    let mut content_length = 0usize;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "browser HTTP header is malformed".to_string())?;
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse()
                .map_err(|_| "browser Content-Length is invalid".to_string())?;
        }
    }
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err("browser HTTP body exceeds the cap".into());
    }
    let mut body = bytes[header_end..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut chunk = vec![0; remaining.min(8 * 1024)];
        stream
            .read_exact(&mut chunk)
            .map_err(|error| format!("read browser HTTP body: {error}"))?;
        body.extend_from_slice(&chunk);
    }
    Ok(HttpRequest {
        method: fields[0].to_owned(),
        path: fields[1].to_owned(),
        body,
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(body).map_err(|error| error.to_string())
}

const BROWSER_PAGE: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Amazing Chromium WebRTC Interop</title>
<style>
body { margin: 0; background: #08111b; color: #d8ffe6; font: 18px system-ui; }
main { max-width: 960px; margin: 32px auto; }
h1 { color: #6dffa3; }
#status { padding: 14px; border: 1px solid #2d8f57; background: #10251a; }
video { display: block; width: 640px; height: 360px; margin-top: 20px; background: #000; }
</style>
<main><h1>Amazing ↔ Chromium WebRTC</h1><div id="status">creating offer…</div>
<video id="video" autoplay muted playsinline></video><canvas id="canvas" width="320" height="180" hidden></canvas></main>
<script>
const status = document.querySelector('#status');
const video = document.querySelector('#video');
const canvas = document.querySelector('#canvas');
let reported = false;
async function report(text) {
  if (reported) return;
  reported = true;
  status.textContent = text;
  await fetch('/result', { method: 'POST', body: text });
}
(async () => {
  const peer = new RTCPeerConnection({ iceServers: [] });
  const stream = new MediaStream();
  video.srcObject = stream;
  peer.ontrack = event => stream.addTrack(event.track);
  peer.onconnectionstatechange = () => {
    if (!reported) status.textContent = `connection ${peer.connectionState}`;
  };
  peer.addTransceiver('audio', { direction: 'recvonly' });
  peer.addTransceiver('video', { direction: 'recvonly' });
  await peer.setLocalDescription(await peer.createOffer());
  if (peer.iceGatheringState !== 'complete') {
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('ICE gathering timeout')), 8000);
      peer.addEventListener('icegatheringstatechange', () => {
        if (peer.iceGatheringState === 'complete') { clearTimeout(timeout); resolve(); }
      });
    });
  }
  status.textContent = 'posting offer…';
  const response = await fetch('/offer', { method: 'POST', headers: { 'Content-Type': 'application/sdp' }, body: peer.localDescription.sdp });
  if (!response.ok) throw new Error(`answer HTTP ${response.status}`);
  await peer.setRemoteDescription({ type: 'answer', sdp: await response.text() });
  let frames = 0;
  const onFrame = () => {
    frames += 1;
    if (frames >= 5 && video.videoWidth > 0) {
      const context = canvas.getContext('2d', { willReadFrequently: true });
      context.drawImage(video, 0, 0, 320, 180);
      const pixels = context.getImageData(0, 0, 320, 180).data;
      let checksum = 0;
      for (let index = 0; index < pixels.length; index += 97) checksum = (checksum * 33 + pixels[index]) >>> 0;
      report(`decoded frames=${frames} size=${video.videoWidth}x${video.videoHeight} checksum=${checksum} state=${peer.connectionState}`);
      return;
    }
    video.requestVideoFrameCallback(onFrame);
  };
  video.requestVideoFrameCallback(onFrame);
  setTimeout(() => report(`ERROR decode timeout state=${peer.connectionState} frames=${frames}`), 15000);
})().catch(error => report(`ERROR ${error.stack || error}`));
</script>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_http_parser_reads_exact_body_and_rejects_oversize() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .write_all(
                    b"POST /offer HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\n\r\ntest",
                )
                .unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream).unwrap();
        writer.join().unwrap();
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("POST", "/offer")
        );
        assert_eq!(request.body, b"test");

        let oversized = format!(
            "POST /offer HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_HTTP_BODY_BYTES + 1
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            TcpStream::connect(address)
                .unwrap()
                .write_all(oversized.as_bytes())
                .unwrap();
        });
        let (mut stream, _) = listener.accept().unwrap();
        assert!(read_http_request(&mut stream).is_err());
        writer.join().unwrap();
    }
}
