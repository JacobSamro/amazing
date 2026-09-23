//! Fetch §6 data: URL processor and Infra forgiving-base64 decoder.
use crate::{api::MAX_API_BODY_BYTES, mime::MimeType, FetchError, HeaderList, InternalResponse};
use amazing_net::ParsedUrl;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataUrl {
    pub mime_type: MimeType,
    pub body: Vec<u8>,
}

pub fn process_data_url(url: &ParsedUrl) -> Option<DataUrl> {
    if url.scheme() != "data" {
        return None;
    }
    let serialized = url.serialize_without_fragment();
    let input = serialized.strip_prefix("data:")?;
    let (raw_mime, encoded_body) = input.split_once(',')?;
    if encoded_body.len() > MAX_API_BODY_BYTES.saturating_mul(3) {
        return None;
    }
    let mut mime = raw_mime
        .trim_matches(|c: char| c.is_ascii_whitespace())
        .to_owned();
    let mut body = percent_decode(encoded_body);
    let lower = mime.to_ascii_lowercase();
    if lower.ends_with("base64") {
        let prefix = &mime[..mime.len() - 6];
        if let Some(semicolon) = prefix.rfind(';') {
            if prefix[semicolon + 1..].chars().all(|c| c == ' ') {
                body = forgiving_base64_decode(&body)?;
                mime.truncate(semicolon);
            }
        }
    }
    if mime.starts_with(';') {
        mime.insert_str(0, "text/plain");
    }
    let mime_type = MimeType::parse(&mime)
        .unwrap_or_else(|| MimeType::parse("text/plain;charset=US-ASCII").unwrap());
    if body.len() > MAX_API_BODY_BYTES {
        return None;
    }
    Some(DataUrl { mime_type, body })
}

pub(crate) fn data_response(url: &ParsedUrl) -> Result<InternalResponse, FetchError> {
    let data = process_data_url(url).ok_or(FetchError::Transport)?;
    let mut headers = HeaderList::default();
    headers.append("Content-Type", &data.mime_type.serialize())?;
    Ok(InternalResponse {
        status: 200,
        status_text: "OK".into(),
        headers,
        body: Some(data.body),
    })
}

pub fn forgiving_base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut data: Vec<u8> = input
        .iter()
        .copied()
        .filter(|b| !matches!(b, b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        .collect();
    if data.len() % 4 == 0 {
        for _ in 0..2 {
            if data.last() == Some(&b'=') {
                data.pop();
            }
        }
    }
    if data.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(data.len().saturating_mul(3) / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for b in data {
        let n = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(n);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn percent_decode(input: &str) -> Vec<u8> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(a), Some(b)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}
fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        api::{fetch_api, ApiRequest, ApiRequestInit, ApiRequestInput, ApiResponse},
        FetchPlan, HttpUrl, NetworkOrCache, Request, Site,
    };

    struct NoNetwork;
    impl NetworkOrCache for NoNetwork {
        fn exchange(
            &mut self,
            _: &Request,
            _: &FetchPlan,
            _: &Site,
        ) -> Result<InternalResponse, FetchError> {
            panic!("data: must not use transport")
        }
    }

    #[test]
    fn fetch_api_serves_data_in_basic_and_cors_modes() {
        let base = HttpUrl::parse("https://example.test/").unwrap();
        for mode in [None, Some("cors".to_owned())] {
            let input: Vec<u16> = "data:text/plain;base64,YQ==#fragment"
                .encode_utf16()
                .collect();
            let mut request = ApiRequest::new(
                ApiRequestInput::Url(&input),
                ApiRequestInit {
                    mode,
                    ..ApiRequestInit::default()
                },
                &base,
                base.origin().clone(),
            )
            .unwrap();
            let mut response: ApiResponse =
                fetch_api(&mut request, &mut NoNetwork, &Site::Opaque(0), &false).unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.body.bytes().unwrap(), b"a");
            assert_eq!(
                response
                    .headers
                    .get_string("content-type")
                    .unwrap()
                    .as_deref(),
                Some("text/plain")
            );
        }
    }
}
