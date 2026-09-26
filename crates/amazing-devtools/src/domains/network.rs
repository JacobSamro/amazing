//! The `Network` domain: request/response/loading events from the engine.

use super::{encode_body, headers, object};
use crate::host::{NetworkFailedEvent, NetworkFinishedEvent, NetworkRequestEvent, NetworkResponseEvent};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Network.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Network.enable" => {
            session.enabled.network = true;
            Ok(DispatchOutcome::empty())
        }
        "Network.disable" => {
            session.enabled.network = false;
            Ok(DispatchOutcome::empty())
        }
        "Network.setCacheDisabled" => {
            let disabled = request.bool_param("cacheDisabled")?;
            session.dev.set_cache_disabled(disabled).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Network.setExtraHTTPHeaders" => {
            let headers_param = request
                .params
                .get("headers")
                .ok_or_else(|| CdpError::invalid_params("headers is required"))?;
            let entries = headers_param
                .as_object()
                .ok_or_else(|| CdpError::invalid_params("headers must be an object"))?;
            let mut headers = Vec::with_capacity(entries.len());
            for (name, value) in entries {
                let value = value
                    .as_str()
                    .ok_or_else(|| CdpError::invalid_params("header values must be strings"))?;
                headers.push((name.clone(), value.to_owned()));
            }
            session
                .dev
                .set_extra_http_headers(&headers)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Network.clearBrowserCache" => Ok(DispatchOutcome::empty()),
        "Network.setBypassServiceWorker" => {
            request.optional_bool("bypass", false)?;
            Ok(DispatchOutcome::empty())
        }
        "Network.getResponseBody" => {
            let request_id = request.string_param("requestId")?;
            let body = session
                .dev
                .get_response_body(request_id)
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
        "Network.getRequestPostData" => {
            let request_id = request.string_param("requestId")?;
            let post = session
                .dev
                .get_request_post_data(request_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "postData",
                Json::String(post.unwrap_or_default()),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Network.requestWillBeSent` event.
pub fn request_event(item: &NetworkRequestEvent) -> Json {
    let mut request = vec![
        ("url", Json::String(item.url.clone())),
        ("method", Json::String(item.method.clone())),
        ("headers", headers(&item.headers)),
        ("hasPostData", Json::Bool(item.post_data.is_some())),
        ("initialPriority", Json::String("Medium".into())),
        (
            "referrerPolicy",
            Json::String("no-referrer-when-downgrade".into()),
        ),
    ];
    if let Some(post_data) = &item.post_data {
        request.push(("postData", Json::String(post_data.clone())));
    }
    crate::protocol::event(
        "Network.requestWillBeSent",
        object(vec![
            ("requestId", Json::String(item.request_id.clone())),
            ("loaderId", Json::String(item.loader_id.clone())),
            ("documentURL", Json::String(item.url.clone())),
            ("request", object(request)),
            ("timestamp", Json::number(item.timestamp)),
            ("wallTime", Json::number(item.wall_time)),
            (
                "initiator",
                object(vec![("type", Json::String("other".into()))]),
            ),
            ("type", Json::String(item.resource_type.clone())),
            ("frameId", Json::String(item.frame_id.clone())),
        ]),
    )
}

/// Builds the `Network.responseReceived` event.
pub fn response_event(item: &NetworkResponseEvent) -> Json {
    let response = object(vec![
        ("url", Json::String(item.url.clone())),
        ("status", Json::number(item.status as f64)),
        ("statusText", Json::String(item.status_text.clone())),
        ("headers", headers(&item.headers)),
        ("mimeType", Json::String(item.mime_type.clone())),
        ("connectionReused", Json::Bool(false)),
        ("connectionId", Json::number(0.0)),
        ("encodedDataLength", Json::number(item.encoded_data_length as f64)),
        ("securityState", Json::String("unknown".into())),
        ("fromDiskCache", Json::Bool(item.from_disk_cache)),
    ]);
    crate::protocol::event(
        "Network.responseReceived",
        object(vec![
            ("requestId", Json::String(item.request_id.clone())),
            ("loaderId", Json::String(String::new())),
            ("timestamp", Json::number(item.timestamp)),
            ("type", Json::String(item.resource_type.clone())),
            ("response", response),
        ]),
    )
}

/// Builds the `Network.loadingFinished` event.
pub fn finished_event(item: &NetworkFinishedEvent) -> Json {
    crate::protocol::event(
        "Network.loadingFinished",
        object(vec![
            ("requestId", Json::String(item.request_id.clone())),
            ("timestamp", Json::number(item.timestamp)),
            ("encodedDataLength", Json::number(item.encoded_data_length as f64)),
        ]),
    )
}

/// Builds the `Network.loadingFailed` event.
pub fn failed_event(item: &NetworkFailedEvent) -> Json {
    crate::protocol::event(
        "Network.loadingFailed",
        object(vec![
            ("requestId", Json::String(item.request_id.clone())),
            ("timestamp", Json::number(item.timestamp)),
            ("type", Json::String(item.resource_type.clone())),
            ("errorText", Json::String(item.error_text.clone())),
            ("canceled", Json::Bool(item.canceled)),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_event_carries_headers_and_type() {
        let item = NetworkRequestEvent {
            request_id: "1".into(),
            url: "https://a/x".into(),
            method: "GET".into(),
            headers: vec![("Accept".into(), "*/*".into())],
            post_data: None,
            resource_type: "Document".into(),
            frame_id: "F1".into(),
            loader_id: String::new(),
            timestamp: 1.5,
            wall_time: 2.0,
        };
        let event = request_event(&item);
        assert_eq!(event.get("method").unwrap().as_str(), Some("Network.requestWillBeSent"));
        let request = event.get("params").unwrap().get("request").unwrap();
        assert_eq!(request.get("headers").unwrap().get("Accept").unwrap().as_str(), Some("*/*"));
        assert_eq!(request.get("hasPostData").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn failed_event_carries_error_text_and_type() {
        let item = NetworkFailedEvent {
            request_id: "1".into(),
            resource_type: "Script".into(),
            error_text: "net::ERR_CONNECTION_REFUSED".into(),
            canceled: false,
            timestamp: 4.0,
        };
        let event = failed_event(&item);
        let params = event.get("params").unwrap();
        assert_eq!(params.get("errorText").unwrap().as_str(), Some("net::ERR_CONNECTION_REFUSED"));
        assert_eq!(params.get("type").unwrap().as_str(), Some("Script"));
        assert_eq!(params.get("canceled").unwrap().as_bool(), Some(false));
    }

    #[test]
    fn response_event_has_status_and_type() {
        let item = NetworkResponseEvent {
            request_id: "1".into(),
            url: "https://a/x".into(),
            status: 200,
            status_text: "OK".into(),
            mime_type: "text/html".into(),
            headers: Vec::new(),
            encoded_data_length: 12,
            from_disk_cache: false,
            resource_type: "Document".into(),
            timestamp: 1.5,
        };
        let event = response_event(&item);
        let response = event.get("params").unwrap().get("response").unwrap();
        assert_eq!(response.get("status").unwrap().as_u64(), Some(200));
        assert_eq!(event.get("params").unwrap().get("type").unwrap().as_str(), Some("Document"));
    }
}
