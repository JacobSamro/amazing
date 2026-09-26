//! The `Debugger` domain: breakpoints, stepping, call frames and sources.
//!
//! Chrome's `CallFrame`/`Location`/`Scope` schema carries a lot of optional
//! detail (a scope chain, `this`, generator state, async stack traces, …);
//! this crate keeps the seam to what a front end needs to show a stack and
//! evaluate in a frame ([`crate::host::CallFrame`],
//! [`crate::host::ScriptLocation`]), matching every other domain here. The
//! engine that implements [`crate::host::DevToolsSession`] for this seam is
//! the JS engine itself (see `.agents/js-runtime/questions.md`).

use super::{empty_array, object};
use crate::domains::runtime::evaluate_result;
use crate::host::{CallFrame, PausedEvent, ScriptInfo, ScriptLocation};
use crate::json::Json;
use crate::protocol::{event, CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Debugger.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Debugger.enable" => {
            session.enabled.debugger = true;
            session.dev.debugger_enabled(true).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "debuggerId",
                Json::String(session.session_id.clone()),
            )])))
        }
        "Debugger.disable" => {
            session.enabled.debugger = false;
            session.dev.debugger_enabled(false).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.setBreakpointByUrl" => {
            let line_number = request.u64_param("lineNumber")? as u32;
            let url = request.string_param("url")?;
            let column_number = request.optional_u64("columnNumber")?.map(|n| n as u32);
            let condition = request.optional_string("condition")?;
            let (breakpoint_id, locations) = session
                .dev
                .set_breakpoint_by_url(url, line_number, column_number, condition)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![
                ("breakpointId", Json::String(breakpoint_id)),
                (
                    "locations",
                    Json::Array(locations.iter().map(location_json).collect()),
                ),
            ])))
        }
        "Debugger.removeBreakpoint" => {
            let breakpoint_id = request.string_param("breakpointId")?;
            session
                .dev
                .remove_breakpoint(breakpoint_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.pause" => {
            session.dev.debugger_pause().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.resume" => {
            session.dev.debugger_resume().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty()
                .with_events(vec![event("Debugger.resumed", object(Vec::new()))]))
        }
        "Debugger.stepInto" => {
            session.dev.debugger_step_into().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.stepOver" => {
            session.dev.debugger_step_over().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.stepOut" => {
            session.dev.debugger_step_out().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Debugger.evaluateOnCallFrame" => {
            let call_frame_id = request.string_param("callFrameId")?;
            let expression = request.string_param("expression")?;
            let result = session
                .dev
                .evaluate_on_call_frame(call_frame_id, expression)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(evaluate_result(&result)))
        }
        "Debugger.getScriptSource" => {
            let script_id = request.string_param("scriptId")?;
            let source = session
                .dev
                .get_script_source(script_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "scriptSource",
                Json::String(source),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Serializes a `Debugger.Location`.
fn location_json(location: &ScriptLocation) -> Json {
    let mut fields = vec![
        ("scriptId", Json::String(location.script_id.clone())),
        ("lineNumber", Json::number(location.line_number as f64)),
    ];
    if let Some(column) = location.column_number {
        fields.push(("columnNumber", Json::number(column as f64)));
    }
    object(fields)
}

/// Serializes a `Debugger.CallFrame`.
fn call_frame_json(frame: &CallFrame) -> Json {
    object(vec![
        ("callFrameId", Json::String(frame.call_frame_id.clone())),
        ("functionName", Json::String(frame.function_name.clone())),
        ("location", location_json(&frame.location)),
        ("url", Json::String(frame.url.clone())),
        ("scopeChain", empty_array()),
    ])
}

/// Builds the `Debugger.scriptParsed` event.
pub fn script_parsed(script: &ScriptInfo) -> Json {
    event(
        "Debugger.scriptParsed",
        object(vec![
            ("scriptId", Json::String(script.script_id.clone())),
            ("url", Json::String(script.url.clone())),
            ("startLine", Json::number(script.start_line as f64)),
            ("startColumn", Json::number(script.start_column as f64)),
            ("endLine", Json::number(script.end_line as f64)),
            ("endColumn", Json::number(script.end_column as f64)),
            ("executionContextId", Json::number(1.0)),
            ("hash", Json::String(script.hash.clone())),
        ]),
    )
}

/// Builds the `Debugger.paused` event.
pub fn paused(paused_event: &PausedEvent) -> Json {
    event(
        "Debugger.paused",
        object(vec![
            (
                "callFrames",
                Json::Array(paused_event.call_frames.iter().map(call_frame_json).collect()),
            ),
            ("reason", Json::String(paused_event.reason.clone())),
            (
                "hitBreakpoints",
                Json::Array(
                    paused_event
                        .hit_breakpoints
                        .iter()
                        .cloned()
                        .map(Json::String)
                        .collect(),
                ),
            ),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, EvaluateResult, HostError, RemoteObject};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn set_breakpoint_by_url(
            &mut self,
            _url: &str,
            line_number: u32,
            _column_number: Option<u32>,
            _condition: Option<&str>,
        ) -> Result<(String, Vec<ScriptLocation>), HostError> {
            Ok((
                "bp-1".into(),
                vec![ScriptLocation {
                    script_id: "script-1".into(),
                    line_number,
                    column_number: None,
                }],
            ))
        }
        fn evaluate_on_call_frame(
            &mut self,
            _call_frame_id: &str,
            _expression: &str,
        ) -> Result<EvaluateResult, HostError> {
            Ok(EvaluateResult::value(RemoteObject::number(7.0)))
        }
        fn get_script_source(&mut self, _script_id: &str) -> Result<String, HostError> {
            Ok("function f() {}".into())
        }
        fn debugger_resume(&mut self) -> Result<(), HostError> {
            Ok(())
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
    fn enable_sets_the_flag() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Debugger.enable"}"#).unwrap();
        dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.debugger);
    }

    #[test]
    fn set_breakpoint_by_url_returns_id_and_locations() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Debugger.setBreakpointByUrl","params":{"lineNumber":10,"url":"https://a/x.js"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome.result.get("breakpointId").unwrap().as_str(),
            Some("bp-1")
        );
        let locations = outcome.result.get("locations").unwrap().as_array().unwrap();
        assert_eq!(locations[0].get("lineNumber").unwrap().as_f64(), Some(10.0));
    }

    #[test]
    fn resume_emits_resumed_event() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Debugger.resume"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(
            outcome.events[0].get("method").unwrap().as_str(),
            Some("Debugger.resumed")
        );
    }

    #[test]
    fn evaluate_on_call_frame_serializes_the_result() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Debugger.evaluateOnCallFrame","params":{"callFrameId":"cf-1","expression":"x"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome.result.get("result").unwrap().get("value").unwrap().as_f64(),
            Some(7.0)
        );
    }

    #[test]
    fn get_script_source_returns_the_source_text() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Debugger.getScriptSource","params":{"scriptId":"script-1"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome.result.get("scriptSource").unwrap().as_str(),
            Some("function f() {}")
        );
    }
}
