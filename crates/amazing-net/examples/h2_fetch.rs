//! Verified public-root TLS + the same HTTP/2 Client used by fetch.
//! Run on win-001: cargo run -p amazing-net --locked --example h2_fetch -- https://nghttp2.org/
use amazing_net::{
    http2::{Client, Limits},
    ParsedUrl,
};
use amazing_net_tls::{connect, Protocol};
use std::{error::Error, time::Duration};
fn main() -> Result<(), Box<dyn Error>> {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://nghttp2.org/".into());
    let url = ParsedUrl::parse(&input)?;
    if url.scheme() != "https" {
        return Err("requires https".into());
    }
    let hostname = url.hostname();
    let stream = connect(
        hostname.trim_start_matches('[').trim_end_matches(']'),
        url.effective_port().ok_or("requires port")?,
        Duration::from_secs(20),
    )?;
    if stream.protocol() != Protocol::H2 {
        return Err("server did not negotiate h2".into());
    }
    let mut client = Client::new(stream, Limits::default())?;
    let headers = vec![
        (b":method".to_vec(), b"GET".to_vec()),
        (b":scheme".to_vec(), b"https".to_vec()),
        (b":authority".to_vec(), url.host_and_port().into_bytes()),
        (b":path".to_vec(), url.http_request_target()?.into_bytes()),
        (b"user-agent".to_vec(), b"Amazing-NET-7".to_vec()),
    ];
    let id = client.start(&headers, &[])?;
    loop {
        if let Some(reply) = client.take_reply(id)? {
            println!(
                "protocol=h2 status={} body_bytes={} headers={} trailers={}",
                reply.status,
                reply.body.len(),
                reply.headers.len(),
                reply.trailers.len()
            );
            if reply.status != 200 {
                return Err("expected HTTP 200".into());
            }
            return Ok(());
        }
        client.poll()?;
    }
}
