use super::{parse_json, usv_string, ApiError, Blob, FormData, JsonValue};
use amazing_net::url::{parse_form_urlencoded, serialize_form_urlencoded};
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
    FormData(FormData),
    Blob(Blob),
    Document { bytes: Vec<u8>, content_type: String },
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
                let encoded = serialize_form_urlencoded(
                    &items
                        .iter()
                        .map(|(k, v)| (usv_string(k), usv_string(v)))
                        .collect::<Vec<_>>(),
                )
                .into_bytes();
                (
                    encoded,
                    None,
                    true,
                    Some("application/x-www-form-urlencoded;charset=UTF-8".into()),
                )
            }
            BodyInput::FormData(form) => {
                let (bytes, mime) = form.encode_multipart();
                (bytes, None, true, Some(mime))
            }
            BodyInput::Blob(blob) => {
                let mime = if blob.content_type.is_empty() {
                    None
                } else {
                    Some(blob.content_type)
                };
                (blob.bytes, None, true, mime)
            }
            BodyInput::Document { bytes, content_type } =>
                (bytes, None, true, Some(content_type)),
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
    pub fn set_mime_type(&mut self, mime: Option<String>) {
        self.mime = mime;
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
    pub fn blob(&mut self) -> Result<Blob, ApiError> {
        let mime = self.mime.clone().unwrap_or_default();
        Ok(Blob::from_bytes(self.consume()?, &mime))
    }
    pub fn form_data(&mut self) -> Result<FormData, ApiError> {
        let mime = self.mime.clone().unwrap_or_default();
        let bytes = self.consume()?;
        let mut fields = mime.split(';');
        let kind = fields.next().unwrap_or("").trim();
        if kind.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
            let mut form = FormData::default();
            for (name, value) in parse_form_urlencoded(&String::from_utf8_lossy(&bytes)) {
                form.append_text(&name, &value);
            }
            return Ok(form);
        }
        if !kind.eq_ignore_ascii_case("multipart/form-data") {
            return Err(ApiError::InvalidFormData);
        }
        let boundary = fields
            .filter_map(|part| part.trim().split_once('='))
            .find(|(name, _)| name.trim().eq_ignore_ascii_case("boundary"))
            .map(|(_, value)| value.trim().trim_matches('"'))
            .ok_or(ApiError::InvalidFormData)?;
        FormData::decode_multipart(&bytes, boundary)
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
