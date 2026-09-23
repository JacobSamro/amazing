//! Native Fetch API objects. WebIDL and promise scheduling belong to bindings.
//! Algorithms follow WHATWG Fetch 357bd98924d94b81fbe8608192a2ee1f123b82f4 §5.
mod body;
mod forms;
mod headers;
mod json;
mod objects;

pub use body::*;
pub use forms::*;
pub use headers::*;
pub use json::*;
pub use objects::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    ByteString,
    InvalidHeaderName,
    InvalidHeaderTuple,
    InvalidHeaderValue,
    ImmutableHeaders,
    InvalidUrl,
    UrlCredentials,
    InvalidWindow,
    NavigateMode,
    InvalidMode,
    InvalidCredentials,
    InvalidCache,
    InvalidRedirect,
    InvalidReferrerPolicy,
    InvalidDuplex,
    InvalidPriority,
    InvalidMethod,
    ForbiddenMethod,
    NoCorsMethod,
    OnlyIfCachedMode,
    InvalidReferrer,
    GetHeadBody,
    StreamNeedsDuplex,
    StreamMode,
    KeepaliveStream,
    BodyUsed,
    BodyTooLarge,
    BodyRead,
    InvalidJson,
    UnsupportedFormData,
    UnsupportedBlob,
    InvalidFormData,
    InvalidStatus,
    InvalidStatusText,
    NullBodyStatus,
    InvalidRedirectStatus,
    Aborted,
    Network(crate::FetchError),
}
impl ApiError {
    /// WebIDL maps only the status variants to RangeError; the rest are TypeError
    /// except Abort, which the embedding maps to its AbortError DOMException.
    pub fn is_range_error(&self) -> bool {
        matches!(self, Self::InvalidStatus | Self::InvalidRedirectStatus)
    }
}

pub fn usv_string(input: &[u16]) -> String {
    String::from_utf16_lossy(input)
}
pub fn byte_string(input: &[u16]) -> Result<String, ApiError> {
    if input.iter().any(|c| *c > 255) {
        return Err(ApiError::ByteString);
    }
    Ok(input.iter().map(|c| char::from(*c as u8)).collect())
}
