//! The `Profiler` domain: sampling CPU profiles.
//!
//! `Profiler.stop`'s result (a call tree plus a flat sample list) is carried
//! as [`crate::host::CpuProfile`]/[`crate::host::CpuProfileNode`], matching
//! Chrome's schema but leaving out fields (`positionTicks`, per-node
//! `deoptReason`) no engine here populates yet. The engine that implements
//! this is the JS engine (see `.agents/js-runtime/questions.md`).

use super::object;
use crate::host::{CpuProfile, CpuProfileNode};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `Profiler.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "Profiler.enable" => {
            session.dev.profiler_enabled(true).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Profiler.disable" => {
            session.dev.profiler_enabled(false).map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Profiler.setSamplingInterval" => {
            let interval = request.u64_param("interval")?.min(u32::MAX as u64) as u32;
            session
                .dev
                .set_sampling_interval(interval)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Profiler.start" => {
            session.dev.start_profiling().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "Profiler.stop" => {
            let profile = session.dev.stop_profiling().map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "profile",
                profile_json(&profile),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

fn node_json(node: &CpuProfileNode) -> Json {
    object(vec![
        ("id", Json::number(node.id as f64)),
        (
            "callFrame",
            object(vec![
                ("functionName", Json::String(node.function_name.clone())),
                ("scriptId", Json::String(node.script_id.clone())),
                ("url", Json::String(node.url.clone())),
                ("lineNumber", Json::number(node.line_number as f64)),
                ("columnNumber", Json::number(0.0)),
            ]),
        ),
        ("hitCount", Json::number(node.hit_count as f64)),
        (
            "children",
            Json::Array(node.children.iter().map(|id| Json::number(*id as f64)).collect()),
        ),
    ])
}

/// Serializes a `Profiler.Profile`.
fn profile_json(profile: &CpuProfile) -> Json {
    object(vec![
        (
            "nodes",
            Json::Array(profile.nodes.iter().map(node_json).collect()),
        ),
        ("startTime", Json::number(profile.start_time)),
        ("endTime", Json::number(profile.end_time)),
        (
            "samples",
            Json::Array(profile.samples.iter().map(|id| Json::number(*id as f64)).collect()),
        ),
        (
            "timeDeltas",
            Json::Array(
                profile
                    .time_deltas
                    .iter()
                    .map(|delta| Json::number(*delta as f64))
                    .collect(),
            ),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn stop_profiling(&mut self) -> Result<CpuProfile, HostError> {
            Ok(CpuProfile {
                nodes: vec![CpuProfileNode {
                    id: 1,
                    function_name: "f".into(),
                    script_id: "script-1".into(),
                    url: "https://a/x.js".into(),
                    line_number: 3,
                    hit_count: 5,
                    children: Vec::new(),
                }],
                start_time: 0.0,
                end_time: 100.0,
                samples: vec![1, 1],
                time_deltas: vec![0, 50],
            })
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
    fn stop_serializes_the_profile_tree() {
        let mut session = session();
        let request = CdpRequest::parse(r#"{"id":1,"method":"Profiler.stop"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        let profile = outcome.result.get("profile").unwrap();
        let nodes = profile.get("nodes").unwrap().as_array().unwrap();
        assert_eq!(
            nodes[0]
                .get("callFrame")
                .unwrap()
                .get("functionName")
                .unwrap()
                .as_str(),
            Some("f")
        );
        assert_eq!(nodes[0].get("hitCount").unwrap().as_f64(), Some(5.0));
        assert_eq!(
            profile.get("samples").unwrap().as_array().unwrap().len(),
            2
        );
    }
}
