//! The `Log` domain: engine log entries (JS console, network, security).

use super::object;
use crate::host::LogEntry;
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Log.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Log.enable" => {
            session.enabled.log = true;
            Ok(DispatchOutcome::empty())
        }
        "Log.disable" => {
            session.enabled.log = false;
            Ok(DispatchOutcome::empty())
        }
        "Log.clear" => Ok(DispatchOutcome::empty()),
        "Log.startViolationsReport" => Ok(DispatchOutcome::empty()),
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Log.entryAdded` event.
pub fn entry_event(item: &LogEntry) -> Json {
    let mut entry = vec![
        ("source", Json::String(item.source.clone())),
        ("level", Json::String(item.level.clone())),
        ("text", Json::String(item.text.clone())),
        ("timestamp", Json::number(item.timestamp)),
    ];
    if let Some(url) = &item.url {
        entry.push(("url", Json::String(url.clone())));
    }
    if let Some(line) = item.line_number {
        entry.push(("lineNumber", Json::number(line as f64)));
    }
    if let Some(stack) = &item.stack_trace {
        entry.push(("stackTrace", Json::String(stack.clone())));
    }
    crate::protocol::event("Log.entryAdded", object(vec![("entry", object(entry))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_event_serializes_optional_fields() {
        let item = LogEntry {
            source: "javascript".into(),
            level: "error".into(),
            text: "boom".into(),
            timestamp: 3.0,
            url: Some("https://a/x.js".into()),
            line_number: Some(7),
            stack_trace: None,
        };
        let event = entry_event(&item);
        let entry = event.get("params").unwrap().get("entry").unwrap();
        assert_eq!(entry.get("source").unwrap().as_str(), Some("javascript"));
        assert_eq!(entry.get("lineNumber").unwrap().as_u64(), Some(7));
        assert!(entry.get("stackTrace").is_none());
    }
}
