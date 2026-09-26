//! The `Tracing` domain: category listing, start/stop and event collection.
//!
//! Chrome streams a trace as many `Tracing.dataCollected` batches followed by
//! `Tracing.tracingComplete`; this crate keeps that shape but collects
//! synchronously on `Tracing.end` (the engine already buffered everything
//! traced since `Tracing.start` in
//! [`crate::host::DevToolsSession::drain_trace_events`]), so one `end` call
//! emits at most one `dataCollected` plus one `tracingComplete`.

use super::object;
use crate::json::Json;
use crate::protocol::{event, CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Tracing.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Tracing.getCategories" => {
            let categories = session.dev.tracing_categories().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "categories",
                Json::Array(categories.into_iter().map(Json::String).collect()),
            )])))
        }
        "Tracing.start" => {
            let categories = comma_separated(request, "categories")?;
            session
                .dev
                .start_tracing(&categories)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Tracing.end" => {
            session.dev.stop_tracing().map_err(CdpError::from)?;
            let collected = session.dev.drain_trace_events();
            let mut events = Vec::new();
            if !collected.is_empty() {
                events.push(event(
                    "Tracing.dataCollected",
                    object(vec![("value", Json::Array(collected))]),
                ));
            }
            events.push(event(
                "Tracing.tracingComplete",
                object(vec![("dataLossOccurred", Json::Bool(false))]),
            ));
            Ok(DispatchOutcome::empty().with_events(events))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Reads a comma-separated string parameter (CDP's `Tracing.start`
/// `categories`), defaulting to every category when absent.
fn comma_separated(request: &CdpRequest, name: &str) -> Result<Vec<String>, CdpError> {
    match request.optional_string(name)? {
        Some(value) if !value.is_empty() => {
            Ok(value.split(',').map(str::to_string).collect())
        }
        _ => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn tracing_categories(&mut self) -> Result<Vec<String>, HostError> {
            Ok(vec!["devtools.timeline".into(), "v8".into()])
        }
        fn start_tracing(&mut self, categories: &[String]) -> Result<(), HostError> {
            assert_eq!(categories, ["devtools.timeline", "v8"]);
            Ok(())
        }
        fn stop_tracing(&mut self) -> Result<(), HostError> {
            Ok(())
        }
        fn drain_trace_events(&mut self) -> Vec<Json> {
            vec![Json::Object(vec![("name".into(), Json::String("paint".into()))])]
        }
    }

    fn session() -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/", "A"),
            Box::new(Scripted),
        )
    }

    #[test]
    fn get_categories_lists_them() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Tracing.getCategories"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let categories = outcome.result.get("categories").unwrap().as_array().unwrap();
        assert_eq!(categories.len(), 2);
    }

    #[test]
    fn start_splits_categories_on_commas() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Tracing.start","params":{"categories":"devtools.timeline,v8"}}"#,
        )
        .unwrap();
        dispatch(&mut session, &request).unwrap();
    }

    #[test]
    fn end_emits_data_collected_then_tracing_complete() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Tracing.end"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.events.len(), 2);
        assert_eq!(
            outcome.events[0].get("method").unwrap().as_str(),
            Some("Tracing.dataCollected")
        );
        assert_eq!(
            outcome.events[1].get("method").unwrap().as_str(),
            Some("Tracing.tracingComplete")
        );
    }
}
