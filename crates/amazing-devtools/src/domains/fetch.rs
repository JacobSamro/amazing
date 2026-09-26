//! The `Fetch` domain: request interception.
//!
//! The engine owns matching and pausing; this crate normalizes the wire shapes
//! and hands decisions back through [`crate::host::DevToolsSession`]. With
//! `Fetch.enable` active, the connection polls `drain_intercepted_requests`
//! and emits `Fetch.requestPaused`; `continueRequest`/`failRequest`/
//! `fulfillRequest` resolve the paused request.

use super::{encode_body, headers, object};
use crate::base64;
use crate::host::{FetchContinue, FetchDecision, FetchFulfill, FetchPattern, InterceptedRequest};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Fetch.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Fetch.enable" => {
            let patterns = patterns(request)?;
            let handle_auth = request.optional_bool("handleAuthRequests", false)?;
            session
                .dev
                .set_fetch_enabled(true, &patterns, handle_auth)
                .map_err(CdpError::from)?;
            session.enabled.fetch = true;
            Ok(DispatchOutcome::empty())
        }
        "Fetch.disable" => {
            session
                .dev
                .set_fetch_enabled(false, &[], false)
                .map_err(CdpError::from)?;
            session.enabled.fetch = false;
            Ok(DispatchOutcome::empty())
        }
        "Fetch.continueRequest" => {
            let request_id = request.string_param("requestId")?.to_string();
            let decision = FetchDecision::Continue(FetchContinue {
                url: request.optional_string("url")?.map(str::to_string),
                method: request.optional_string("method")?.map(str::to_string),
                headers: optional_header_list(request, "headers")?,
                post_data: request.optional_string("postData")?.map(str::to_string),
            });
            session
                .dev
                .resolve_intercepted_request(&request_id, &decision)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Fetch.failRequest" => {
            let request_id = request.string_param("requestId")?.to_string();
            let reason = request.string_param("errorReason")?.to_string();
            session
                .dev
                .resolve_intercepted_request(&request_id, &FetchDecision::Fail(reason))
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Fetch.fulfillRequest" => {
            let request_id = request.string_param("requestId")?.to_string();
            let response_code = request
                .optional_u64("responseCode")?
                .ok_or_else(|| CdpError::invalid_params("missing responseCode"))?;
            let follow = optional_header_list(request, "responseHeaders")?;
            let body = match request.optional_string("body")? {
                Some(encoded) => base64::decode(encoded).map_err(|error| {
                    CdpError::invalid_params(format!("body is not valid base64: {error}"))
                })?,
                None => Vec::new(),
            };
            let decision = FetchDecision::Fulfill(FetchFulfill {
                response_code: response_code.min(u16::MAX as u64) as u16,
                response_phrase: request.optional_string("responsePhrase")?.map(str::to_string),
                headers: follow.unwrap_or_default(),
                body,
            });
            session
                .dev
                .resolve_intercepted_request(&request_id, &decision)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Fetch.getResponseBody" => {
            let request_id = request.string_param("requestId")?.to_string();
            let body = session
                .dev
                .get_response_body(&request_id)
                .map_err(CdpError::from)?;
            let (body, base64_encoded) = match body {
                Some(content) => encode_body(content),
                None => (String::new(), false),
            };
            Ok(DispatchOutcome::result(object(vec![
                ("body", Json::String(body)),
                ("base64Encoded", Json::Bool(base64_encoded)),
            ])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Fetch.requestPaused` event.
pub fn request_paused(item: &InterceptedRequest) -> Json {
    let mut body = vec![
        ("url", Json::String(item.url.clone())),
        ("method", Json::String(item.method.clone())),
        ("headers", headers(&item.headers)),
        ("hasPostData", Json::Bool(item.post_data.is_some())),
    ];
    if let Some(post_data) = &item.post_data {
        body.push(("postData", Json::String(post_data.clone())));
    }
    let mut params = vec![
        ("requestId", Json::String(item.request_id.clone())),
        ("request", object(body)),
        ("frameId", Json::String(item.frame_id.clone())),
        ("resourceType", Json::String(item.resource_type.clone())),
    ];
    if let Some(network) = &item.network_id {
        params.push(("networkId", Json::String(network.clone())));
    }
    crate::protocol::event("Fetch.requestPaused", object(params))
}

fn patterns(request: &CdpRequest) -> Result<Vec<FetchPattern>, CdpError> {
    let Some(raw) = request.params.get("patterns") else {
        return Ok(Vec::new());
    };
    if raw == &Json::Null {
        return Ok(Vec::new());
    }
    let raw = raw
        .as_array()
        .ok_or_else(|| CdpError::invalid_params("patterns must be an array"))?;
    let mut patterns = Vec::with_capacity(raw.len());
    for item in raw {
        patterns.push(FetchPattern {
            url_pattern: optional_str(item, "urlPattern")?,
            resource_type: optional_str(item, "resourceType")?,
            request_stage: optional_str(item, "requestStage")?,
        });
    }
    Ok(patterns)
}

fn optional_str(value: &Json, field: &str) -> Result<Option<String>, CdpError> {
    match value.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(CdpError::invalid_params(format!(
            "patterns[].{field} must be a string"
        ))),
    }
}

fn optional_header_list(
    request: &CdpRequest,
    param: &str,
) -> Result<Option<Vec<(String, String)>>, CdpError> {
    let Some(raw) = request.params.get(param) else {
        return Ok(None);
    };
    if raw == &Json::Null {
        return Ok(None);
    }
    let raw = raw
        .as_array()
        .ok_or_else(|| CdpError::invalid_params(format!("{param} must be an array")))?;
    let mut out = Vec::with_capacity(raw.len());
    for item in raw {
        let name = item
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_params(format!("{param}[].name must be a string")))?;
        let value = item
            .get("value")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::invalid_params(format!("{param}[].value must be a string")))?;
        out.push((name.to_string(), value.to_string()));
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};
    use crate::session::SessionState;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Recording {
        enabled: Option<bool>,
        patterns: Vec<FetchPattern>,
        decisions: Vec<(String, FetchDecision)>,
    }

    struct Recorder(Arc<Mutex<Recording>>);

    impl DevToolsSession for Recorder {
        fn set_fetch_enabled(
            &mut self,
            enabled: bool,
            patterns: &[FetchPattern],
            _handle_auth: bool,
        ) -> Result<(), HostError> {
            let mut recording = self.0.lock().unwrap();
            recording.enabled = Some(enabled);
            recording.patterns = patterns.to_vec();
            Ok(())
        }
        fn resolve_intercepted_request(
            &mut self,
            request_id: &str,
            decision: &FetchDecision,
        ) -> Result<(), HostError> {
            self.0
                .lock()
                .unwrap()
                .decisions
                .push((request_id.to_string(), decision.clone()));
            Ok(())
        }
    }

    fn session() -> (SessionState, Arc<Mutex<Recording>>) {
        let recording = Arc::new(Mutex::new(Recording::default()));
        let session = SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "about:blank", ""),
            Box::new(Recorder(Arc::clone(&recording))),
        );
        (session, recording)
    }

    #[test]
    fn enable_parses_patterns_and_decisions_are_recorded() {
        let (mut session, recording) = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Fetch.enable","params":{"patterns":[{"urlPattern":"*x*","resourceType":"Fetch"}],"handleAuthRequests":true}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.fetch);
        {
            let recording = recording.lock().unwrap();
            assert_eq!(recording.enabled, Some(true));
            assert_eq!(recording.patterns.len(), 1);
            assert_eq!(recording.patterns[0].url_pattern.as_deref(), Some("*x*"));
        }

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"Fetch.fulfillRequest","params":{"requestId":"r1","responseCode":201,"responsePhrase":"Created","responseHeaders":[{"name":"x-a","value":"b"}],"body":"aGk="}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();

        let recording = recording.lock().unwrap();
        match &recording.decisions[0].1 {
            FetchDecision::Fulfill(fulfill) => {
                assert_eq!(fulfill.response_code, 201);
                assert_eq!(fulfill.headers, [("x-a".to_string(), "b".to_string())]);
                assert_eq!(fulfill.body, b"hi");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
}
