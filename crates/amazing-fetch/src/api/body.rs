use super::{parse_json, usv_string, ApiError, JsonValue};
use amazing_net::StreamingResponse;
use std::{cell::RefCell, rc::Rc};

pub const MAX_API_BODY_BYTES: usize = 32 * 1024 * 1024;

pub trait PullBody {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError>;
}
impl PullBody for StreamingResponse {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, ApiError> {
        StreamingResponse::next_chunk(self).map_err(|_| ApiError::BodyRead)
    }
}
pub enum BodyInput {
    Bytes(Vec<u8>),
    String(Vec<u16>),
    UrlSearchParams(Vec<(Vec<u16>, Vec<u16>)>),
    FormData,
    Stream(Box<dyn PullBody>),
}
impl BodyInput {
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Stream(_))
    }
}

struct SharedBody {
    bytes: Vec<u8>,
    source: Option<Box<dyn PullBody>>,
    complete: bool,
}
pub struct ApiBody {
    shared: Option<Rc<RefCell<SharedBody>>>,
    used: bool,
    disturbed: bool,
    locked: bool,
    mime: Option<String>,
}
impl std::fmt::Debug for ApiBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiBody")
            .field("present", &self.shared.is_some())
            .field("used", &self.used)
            .field("disturbed", &self.disturbed)
            .field("locked", &self.locked)
            .finish()
    }
}
impl ApiBody {
    pub fn empty() -> Self {
        Self {
            shared: None,
            used: false,
            disturbed: false,
            locked: false,
            mime: None,
        }
    }
    pub fn from_input(input: BodyInput) -> Result<Self, ApiError> {
        let (bytes, source, complete, mime) = match input {
            BodyInput::Bytes(bytes) => (bytes, None, true, None),
            BodyInput::String(units) => (
                usv_string(&units).into_bytes(),
                None,
                true,
                Some("text/plain;charset=UTF-8".into()),
            ),
            BodyInput::UrlSearchParams(items) => {
                let encoded = items
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "{}={}",
                            form_encode(&usv_string(k)),
                            form_encode(&usv_string(v))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("&")
                    .into_bytes();
                (
                    encoded,
                    None,
                    true,
                    Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
                )
            }
            BodyInput::FormData => return Err(ApiError::UnsupportedFormData),
            BodyInput::Stream(stream) => (Vec::new(), Some(stream), false, None),
        };
        if bytes.len() > MAX_API_BODY_BYTES {
            return Err(ApiError::BodyTooLarge);
        }
        Ok(Self {
            shared: Some(Rc::new(RefCell::new(SharedBody {
                bytes,
                source,
                complete,
            }))),
            used: false,
            disturbed: false,
            locked: false,
            mime,
        })
    }
    pub fn mime_type(&self) -> Option<&str> {
        self.mime.as_deref()
    }
    pub fn shared_present(&self) -> bool {
        self.shared.is_some()
    }
    pub fn body_used(&self) -> bool {
        self.shared.is_some() && self.disturbed
    }
    pub fn lock(&mut self) {
        if self.shared.is_some() {
            self.locked = true;
        }
    }
    pub fn unlock(&mut self) {
        self.locked = false;
    }
    pub fn mark_disturbed(&mut self) {
        if self.shared.is_some() {
            self.disturbed = true;
        }
    }
    pub fn is_unusable(&self) -> bool {
        self.shared.is_some() && (self.used || self.disturbed || self.locked)
    }
    pub fn clone_body(&self) -> Result<Self, ApiError> {
        if self.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        Ok(Self {
            shared: self.shared.clone(),
            used: false,
            disturbed: false,
            locked: false,
            mime: self.mime.clone(),
        })
    }
    pub fn bytes(&mut self) -> Result<Vec<u8>, ApiError> {
        self.consume()
    }
    pub fn array_buffer(&mut self) -> Result<Vec<u8>, ApiError> {
        self.consume()
    }
    pub fn text(&mut self) -> Result<String, ApiError> {
        let bytes = self.consume()?;
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
    pub fn json(&mut self) -> Result<JsonValue, ApiError> {
        parse_json(&self.consume()?)
    }
    pub fn blob(&mut self) -> Result<(), ApiError> {
        Err(ApiError::UnsupportedBlob)
    }
    pub fn form_data(&mut self) -> Result<(), ApiError> {
        Err(ApiError::UnsupportedFormData)
    }
    fn consume(&mut self) -> Result<Vec<u8>, ApiError> {
        let Some(shared) = &self.shared else {
            return Ok(Vec::new());
        };
        if self.is_unusable() {
            return Err(ApiError::BodyUsed);
        }
        self.used = true;
        self.disturbed = true;
        let mut shared = shared.borrow_mut();
        while !shared.complete {
            let next = shared
                .source
                .as_mut()
                .ok_or(ApiError::BodyRead)?
                .next_chunk()?;
            match next {
                Some(chunk) => {
                    if shared
                        .bytes
                        .len()
                        .checked_add(chunk.len())
                        .is_none_or(|n| n > MAX_API_BODY_BYTES)
                    {
                        return Err(ApiError::BodyTooLarge);
                    }
                    shared.bytes.extend_from_slice(&chunk);
                }
                None => {
                    shared.complete = true;
                    shared.source = None;
                }
            }
        }
        Ok(shared.bytes.clone())
    }
}
fn form_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"*-._".contains(b) {
            out.push(*b as char);
        } else if *b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
