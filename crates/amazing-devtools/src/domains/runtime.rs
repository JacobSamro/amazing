//! The `Runtime` domain: evaluation, remote objects and console events.

use super::{empty_array, exception_details, object, property_descriptor, remote_object};
use crate::host::{
    CallArgument, CallFunctionParams, ConsoleEvent, EvaluateParams, EvaluateResult, RemoteObject,
};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Runtime.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Runtime.enable" => {
            session.enabled.runtime = true;
            let url = session
                .dev
                .frame_tree()
                .map(|tree| tree.frame.url)
                .unwrap_or_else(|_| session.target.url.clone());
            let event = execution_context_created(&session.target.id, &url, 1);
            Ok(DispatchOutcome::empty().with_events(vec![event]))
        }
        "Runtime.disable" => {
            session.enabled.runtime = false;
            Ok(DispatchOutcome::empty())
        }
        "Runtime.evaluate" => {
            let params = evaluate_params(request)?;
            let result = session.dev.evaluate(&params).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(evaluate_result(&result)))
        }
        "Runtime.callFunctionOn" => {
            let params = call_function_params(request)?;
            let result = session
                .dev
                .call_function_on(&params)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(evaluate_result(&result)))
        }
        "Runtime.getProperties" => {
            let object_id = request.string_param("objectId")?;
            let own = request.optional_bool("ownProperties", false)?;
            let properties = session
                .dev
                .get_properties(object_id, own)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![
                (
                    "result",
                    Json::Array(properties.iter().map(property_descriptor).collect()),
                ),
                ("internalProperties", empty_array()),
                ("privateProperties", empty_array()),
            ])))
        }
        "Runtime.releaseObject" => {
            let object_id = request.string_param("objectId")?;
            session.dev.release_object(object_id).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Runtime.releaseObjectGroup" => {
            request.string_param("objectGroup")?;
            Ok(DispatchOutcome::empty())
        }
        "Runtime.runIfWaitingForDebugger" => Ok(DispatchOutcome::empty()),
        "Runtime.compileScript" => {
            let source = request.string_param("expression")?;
            let url = request.optional_string("sourceURL")?;
            let persist = request.optional_bool("persistScript", false)?;
            let script_id = session
                .dev
                .compile_script(source, url, persist)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "scriptId",
                Json::String(script_id),
            )])))
        }
        "Runtime.runScript" => {
            let script_id = request.string_param("scriptId")?;
            let result = session.dev.run_script(script_id).map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(evaluate_result(&result)))
        }
        "Runtime.getIsolateId" => {
            let id = session.dev.get_isolate_id().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "id",
                Json::String(id.unwrap_or_default()),
            )])))
        }
        "Runtime.getHeapUsage" => {
            let usage = session.dev.get_heap_usage().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![
                ("usedSize", Json::number(usage.used_heap_size)),
                ("totalSize", Json::number(usage.total_heap_size)),
            ])))
        }
        "Runtime.globalLexicalScopeNames" => {
            let names = session
                .dev
                .global_lexical_scope_names()
                .map_err(CdpError::from)?;
            let names: Vec<Json> = names.into_iter().map(Json::String).collect();
            Ok(DispatchOutcome::result(object(vec![(
                "names",
                Json::Array(names),
            )])))
        }
        "Runtime.queryObjects" => {
            let prototype = request.string_param("prototypeObjectId")?;
            let object_id = session
                .dev
                .query_objects(prototype)
                .map_err(CdpError::from)?;
            let objects = match object_id {
                Some(id) => remote_object(&RemoteObject {
                    object_type: "object".into(),
                    subtype: Some("array".into()),
                    class_name: Some("Array".into()),
                    object_id: Some(id),
                    ..RemoteObject::undefined()
                }),
                None => Json::Null,
            };
            Ok(DispatchOutcome::result(object(vec![("objects", objects)])))
        }
        "Runtime.setAsyncCallStackDepth" => {
            let depth = request.u64_param("maxDepth")?.min(u32::MAX as u64) as u32;
            session
                .dev
                .set_async_call_stack_depth(depth)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Runtime.discardConsoleEntries" => {
            session
                .dev
                .discard_console_entries()
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

/// Builds the `Runtime.executionContextCreated` event for a page context.
pub fn execution_context_created(target_id: &str, url: &str, context_id: u64) -> Json {
    crate::protocol::event(
        "Runtime.executionContextCreated",
        object(vec![(
            "context",
            object(vec![
                ("id", Json::number(context_id as f64)),
                ("origin", Json::String(origin_of(url))),
                ("name", Json::String(String::new())),
                ("uniqueId", Json::String(target_id.to_string())),
                (
                    "auxData",
                    object(vec![
                        ("isDefault", Json::Bool(true)),
                        ("type", Json::String("default".into())),
                        ("frameId", Json::String(target_id.to_string())),
                    ]),
                ),
            ]),
        )]),
    )
}

/// Builds the `Runtime.consoleAPICalled` event.
pub fn console_event(item: &ConsoleEvent) -> Json {
    crate::protocol::event(
        "Runtime.consoleAPICalled",
        object(vec![
            ("type", Json::String(item.kind.clone())),
            (
                "args",
                Json::Array(item.args.iter().map(remote_object).collect()),
            ),
            (
                "executionContextId",
                Json::number(item.execution_context_id as f64),
            ),
            ("timestamp", Json::number(item.timestamp)),
            (
                "stackTrace",
                object(vec![("callFrames", empty_array())]),
            ),
        ]),
    )
}

/// Extracts `scheme://authority` from a URL, empty when there is none.
pub fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            format!("{scheme}://{authority}")
        }
        None => String::new(),
    }
}

fn evaluate_result(result: &EvaluateResult) -> Json {
    let mut fields = vec![("result", remote_object(&result.result))];
    if let Some(exception) = &result.exception {
        fields.push(("exceptionDetails", exception_details(exception)));
    }
    object(fields)
}

fn evaluate_params(request: &CdpRequest) -> Result<EvaluateParams, CdpError> {
    Ok(EvaluateParams {
        expression: request.string_param("expression")?.to_string(),
        object_group: request.optional_string("objectGroup")?.map(str::to_string),
        include_command_line_api: request.optional_bool("includeCommandLineAPI", false)?,
        silent: request.optional_bool("silent", false)?,
        return_by_value: request.optional_bool("returnByValue", false)?,
        generate_preview: request.optional_bool("generatePreview", false)?,
        await_promise: request.optional_bool("awaitPromise", false)?,
        context_id: request.optional_u64("contextId")?,
        user_gesture: request.optional_bool("userGesture", false)?,
    })
}

fn call_function_params(request: &CdpRequest) -> Result<CallFunctionParams, CdpError> {
    let mut arguments = Vec::new();
    if let Some(items) = request.params.get("arguments") {
        let items = items
            .as_array()
            .ok_or_else(|| CdpError::invalid_params("arguments must be an array"))?;
        for item in items {
            arguments.push(CallArgument {
                value: item.get("value").cloned(),
                object_id: item
                    .get("objectId")
                    .and_then(Json::as_str)
                    .map(str::to_string),
            });
        }
    }
    Ok(CallFunctionParams {
        function_declaration: request.string_param("functionDeclaration")?.to_string(),
        object_id: request.optional_string("objectId")?.map(str::to_string),
        arguments,
        return_by_value: request.optional_bool("returnByValue", false)?,
        await_promise: request.optional_bool("awaitPromise", false)?,
        object_group: request.optional_string("objectGroup")?.map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError, RemoteObject};
    use crate::session::SessionState;

    struct Echo;

    impl DevToolsSession for Echo {
        fn evaluate(&mut self, params: &EvaluateParams) -> Result<EvaluateResult, HostError> {
            if params.expression == "boom" {
                return Ok(EvaluateResult {
                    result: RemoteObject::undefined(),
                    exception: Some(crate::host::ExceptionDetails {
                        text: "Uncaught".into(),
                        line_number: 1,
                        column_number: 2,
                        url: None,
                        exception: None,
                        stack_trace: None,
                    }),
                });
            }
            Ok(EvaluateResult::value(RemoteObject::number(42.0)))
        }
        fn get_properties(
            &mut self,
            object_id: &str,
            _own: bool,
        ) -> Result<Vec<crate::host::PropertyDescriptor>, HostError> {
            if object_id == "missing" {
                return Err(HostError::NotFound);
            }
            Ok(vec![crate::host::PropertyDescriptor::own(
                "a",
                RemoteObject::number(1.0),
            )])
        }
    }

    fn session() -> SessionState {
        SessionState::new(
            "s",
            crate::host::TargetInfo::page("T1", "https://a/x", "A"),
            Box::new(Echo),
        )
    }

    #[test]
    fn enable_emits_execution_context() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Runtime.enable"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert!(session.enabled.runtime);
        let context = outcome.events[0].get("params").unwrap().get("context").unwrap();
        assert_eq!(context.get("origin").unwrap().as_str(), Some("https://a"));
    }

    #[test]
    fn evaluate_serializes_result_and_exception() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Runtime.evaluate","params":{"expression":"1+1","returnByValue":true}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome.result.get("result").unwrap().get("value").unwrap().as_f64(),
            Some(42.0)
        );

        let request = CdpRequest::parse(
            r#"{"id":2,"method":"Runtime.evaluate","params":{"expression":"boom"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome
                .result
                .get("exceptionDetails")
                .unwrap()
                .get("text")
                .unwrap()
                .as_str(),
            Some("Uncaught")
        );
    }

    #[test]
    fn get_properties_maps_not_found_to_server_error() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"Runtime.getProperties","params":{"objectId":"missing"}}"#,
        )
        .unwrap();
        assert_eq!(
            dispatch(&mut session, &request).unwrap_err().code,
            crate::protocol::SERVER_ERROR
        );
    }
}
