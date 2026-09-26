//! Chrome DevTools Protocol message framing over JSON-RPC.
//!
//! CDP exchanges three message shapes on a WebSocket:
//!
//! * a request `{"id":1,"method":"Page.navigate","params":{...}}`,
//! * a response `{"id":1,"result":{...}}` or `{"id":1,"error":{...}}`,
//! * an event `{"method":"Page.loadEventFired","params":{...}}`.
//!
//! When a flattened session is in use (`Target.attachToTarget` with
//! `flatten: true`), request, response and event all carry `sessionId`.
//! [`crate::server`] routes on it; this module only models and (de)serializes
//! the shapes.

use crate::host::HostError;
use crate::json::{Json, JsonError};

/// JSON parse failure (`JSON-RPC` -32700).
pub const PARSE_ERROR: i64 = -32700;
/// A structurally invalid message (-32600).
pub const INVALID_REQUEST: i64 = -32600;
/// Unknown method or domain (-32601).
pub const METHOD_NOT_FOUND: i64 = -32601;
/// A known method with bad parameters (-32602).
pub const INVALID_PARAMS: i64 = -32602;
/// An engine-side failure (-32603).
pub const INTERNAL_ERROR: i64 = -32603;
/// A server-side failure that does not fit the codes above (-32000).
pub const SERVER_ERROR: i64 = -32000;
/// A request carried no `id`, so no response is sent (JSON-RPC notification).
pub const NO_ID: i64 = -1;

/// A parsed client request.
#[derive(Clone, Debug, PartialEq)]
pub struct CdpRequest {
    /// `None` for a JSON-RPC notification. Otherwise a number or string echoed
    /// verbatim on the response.
    pub id: Option<Json>,
    pub method: String,
    /// Always an object; `{}` when the field was absent.
    pub params: Json,
    pub session_id: Option<String>,
}

impl CdpRequest {
    /// Parses and validates one request message.
    pub fn parse(text: &str) -> Result<CdpRequest, CdpError> {
        let value = Json::parse(text).map_err(CdpError::from_json)?;
        let id = match value.get("id") {
            None | Some(Json::Null) => None,
            Some(Json::Number(n)) if n.fract() == 0.0 && n.is_finite() => Some(value.get("id").unwrap().clone()),
            Some(Json::String(_)) => Some(value.get("id").unwrap().clone()),
            Some(_) => {
                return Err(CdpError::invalid_request("id must be a number or string"));
            }
        };
        let method = value
            .get("method")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_request("method must be a string"))?;
        if method.is_empty() {
            return Err(CdpError::invalid_request("method must not be empty"));
        }
        let params = match value.get("params") {
            None | Some(Json::Null) => Json::Object(Vec::new()),
            Some(Json::Object(_)) => value.get("params").unwrap().clone(),
            Some(_) => {
                return Err(CdpError::invalid_request("params must be an object"));
            }
        };
        let session_id = match value.get("sessionId") {
            None | Some(Json::Null) => None,
            Some(Json::String(s)) => Some(s.clone()),
            Some(_) => {
                return Err(CdpError::invalid_request("sessionId must be a string"));
            }
        };
        Ok(CdpRequest {
            id,
            method: method.to_string(),
            params,
            session_id,
        })
    }

    /// Whether a response is expected.
    pub fn expects_response(&self) -> bool {
        self.id.is_some()
    }

    /// A required string parameter.
    pub fn string_param(&self, name: &str) -> Result<&str, CdpError> {
        self.params
            .get(name)
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_params(format!("missing string parameter '{name}'")))
    }

    /// An optional string parameter.
    pub fn optional_string(&self, name: &str) -> Result<Option<&str>, CdpError> {
        match self.params.get(name) {
            None | Some(Json::Null) => Ok(None),
            Some(Json::String(s)) => Ok(Some(s)),
            Some(_) => Err(CdpError::invalid_params(format!(
                "parameter '{name}' must be a string"
            ))),
        }
    }

    /// An optional boolean parameter, defaulting to `default`.
    pub fn optional_bool(&self, name: &str, default: bool) -> Result<bool, CdpError> {
        match self.params.get(name) {
            None | Some(Json::Null) => Ok(default),
            Some(Json::Bool(b)) => Ok(*b),
            Some(_) => Err(CdpError::invalid_params(format!(
                "parameter '{name}' must be a boolean"
            ))),
        }
    }

    /// An optional non-negative integer parameter.
    pub fn optional_u64(&self, name: &str) -> Result<Option<u64>, CdpError> {
        match self.params.get(name) {
            None | Some(Json::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| CdpError::invalid_params(format!("parameter '{name}' must be a u64"))),
        }
    }
}

/// A CDP error payload.
#[derive(Clone, Debug, PartialEq)]
pub struct CdpError {
    pub code: i64,
    pub message: String,
    pub data: Option<Json>,
}

impl CdpError {
    /// Builds an error from a code and message.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// `-32700` for a malformed JSON document.
    pub fn from_json(error: JsonError) -> Self {
        Self::new(PARSE_ERROR, error.to_string())
    }

    /// `-32600` for a structurally invalid message.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    /// `-32601` for an unknown method.
    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("'{method}' wasn't found"))
    }

    /// `-32602` for a known method with bad parameters.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    /// `-32603` for an engine-side failure.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(INTERNAL_ERROR, message)
    }

    /// `-32000` for a generic server failure.
    pub fn server(message: impl Into<String>) -> Self {
        Self::new(SERVER_ERROR, message)
    }

    /// Serializes the error object (`{"code":...,"message":...}`).
    pub fn to_json(&self) -> Json {
        let mut object = vec![
            ("code".to_string(), Json::Number(self.code as f64)),
            ("message".to_string(), Json::String(self.message.clone())),
        ];
        if let Some(data) = &self.data {
            object.push(("data".to_string(), data.clone()));
        }
        Json::Object(object)
    }
}

impl From<HostError> for CdpError {
    fn from(error: HostError) -> Self {
        match error {
            HostError::Unsupported => Self::method_not_found("engine support"),
            HostError::NotFound => Self::server("the referenced target, node or object was not found"),
            HostError::InvalidParams(message) => Self::invalid_params(message),
            HostError::Internal(message) => Self::internal(message),
        }
    }
}

/// A `{id, result}` response.
pub fn response(id: &Json, result: Json) -> Json {
    Json::Object(vec![
        ("id".to_string(), id.clone()),
        ("result".to_string(), result),
    ])
}

/// A `{id, error}` response.
pub fn error_response(id: &Json, error: &CdpError) -> Json {
    Json::Object(vec![
        ("id".to_string(), id.clone()),
        ("error".to_string(), error.to_json()),
    ])
}

/// An event with no session tag.
pub fn event(method: &str, params: Json) -> Json {
    Json::Object(vec![
        ("method".to_string(), Json::String(method.to_string())),
        ("params".to_string(), params),
    ])
}

/// Adds a `sessionId` field to a request/response/event object.
pub fn with_session(mut message: Json, session_id: &str) -> Json {
    message.insert("sessionId", Json::String(session_id.to_string()));
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_request() {
        let request = CdpRequest::parse(r#"{"id":7,"method":"Page.navigate","params":{"url":"https://a"}}"#)
            .unwrap();
        assert_eq!(request.id, Some(Json::Number(7.0)));
        assert_eq!(request.method, "Page.navigate");
        assert_eq!(request.string_param("url").unwrap(), "https://a");
        assert!(request.expects_response());
    }

    #[test]
    fn parses_defaults_and_notifications() {
        let request = CdpRequest::parse(r#"{"method":"Page.enable"}"#).unwrap();
        assert_eq!(request.id, None);
        assert_eq!(request.params, Json::Object(Vec::new()));
        assert!(!request.expects_response());
        assert_eq!(request.optional_bool("x", true).unwrap(), true);
        assert_eq!(request.optional_u64("y").unwrap(), None);
    }

    #[test]
    fn parses_string_ids_and_sessions() {
        let request =
            CdpRequest::parse(r#"{"id":"a","method":"Runtime.evaluate","sessionId":"s1"}"#).unwrap();
        assert_eq!(request.id, Some(Json::String("a".into())));
        assert_eq!(request.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn rejects_invalid_requests() {
        assert_eq!(CdpRequest::parse("{").unwrap_err().code, PARSE_ERROR);
        for bad in [
            r#"[]"#,
            r#"{"id":1}"#,
            r#"{"method":7}"#,
            r#"{"method":""}"#,
            r#"{"method":"a","params":[]}"#,
            r#"{"method":"a","sessionId":4}"#,
            r#"{"method":"a","id":1.5}"#,
        ] {
            let error = CdpRequest::parse(bad).unwrap_err();
            assert_eq!(error.code, INVALID_REQUEST, "unexpected code for {bad}");
        }
    }

    #[test]
    fn serializes_messages() {
        let id = Json::Number(1.0);
        assert_eq!(
            response(&id, Json::Bool(true)).to_string(),
            r#"{"id":1,"result":true}"#
        );
        assert_eq!(
            error_response(&id, &CdpError::method_not_found("X.y")).to_string(),
            r#"{"id":1,"error":{"code":-32601,"message":"'X.y' wasn't found"}}"#
        );
        assert_eq!(
            event("Page.loadEventFired", Json::Object(Vec::new())).to_string(),
            r#"{"method":"Page.loadEventFired","params":{}}"#
        );
        assert_eq!(
            with_session(event("E", Json::Object(Vec::new())), "s").to_string(),
            r#"{"method":"E","params":{},"sessionId":"s"}"#
        );
    }

    #[test]
    fn host_errors_map_to_codes() {
        assert_eq!(CdpError::from(HostError::Unsupported).code, METHOD_NOT_FOUND);
        assert_eq!(CdpError::from(HostError::NotFound).code, SERVER_ERROR);
        assert_eq!(
            CdpError::from(HostError::InvalidParams("x".into())).code,
            INVALID_PARAMS
        );
        assert_eq!(CdpError::from(HostError::Internal("x".into())).code, INTERNAL_ERROR);
    }
}
