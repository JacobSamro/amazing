//! The `HeapProfiler` domain: heap snapshots and heap object identity.
//!
//! Chrome streams a snapshot as many `addHeapSnapshotChunk` events followed
//! by `reportHeapSnapshotProgress(done: true)`; this crate keeps that framing
//! but leaves the chunking policy to the engine
//! ([`crate::host::DevToolsSession::take_heap_snapshot`] returns one or more
//! chunks already split). The engine that implements this seam is the JS
//! engine (see `.agents/js-runtime/questions.md`).

use super::{object, remote_object};
use crate::json::Json;
use crate::protocol::{event, CdpError, CdpRequest};
use crate::session::{DispatchOutcome, SessionState};

/// Handles a `HeapProfiler.*` command.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    match request.method.as_str() {
        "HeapProfiler.enable" => {
            session
                .dev
                .heap_profiler_enabled(true)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "HeapProfiler.disable" => {
            session
                .dev
                .heap_profiler_enabled(false)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "HeapProfiler.takeHeapSnapshot" => {
            let chunks = session.dev.take_heap_snapshot().map_err(CdpError::from)?;
            let mut events: Vec<Json> = chunks
                .into_iter()
                .map(|chunk| {
                    event(
                        "HeapProfiler.addHeapSnapshotChunk",
                        object(vec![("chunk", Json::String(chunk))]),
                    )
                })
                .collect();
            events.push(event(
                "HeapProfiler.reportHeapSnapshotProgress",
                object(vec![
                    ("done", Json::number(0.0)),
                    ("total", Json::number(0.0)),
                    ("finished", Json::Bool(true)),
                ]),
            ));
            Ok(DispatchOutcome::empty().with_events(events))
        }
        "HeapProfiler.collectGarbage" => {
            session.dev.collect_garbage().map_err(CdpError::from)?;
            Ok(DispatchOutcome::empty())
        }
        "HeapProfiler.getHeapObjectId" => {
            let object_id = request.string_param("objectId")?;
            let heap_object_id = session
                .dev
                .get_heap_object_id(object_id)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "heapSnapshotObjectId",
                Json::String(heap_object_id),
            )])))
        }
        "HeapProfiler.getObjectByHeapObjectId" => {
            let heap_object_id = request.string_param("objectId")?;
            let object_group = request.optional_string("objectGroup")?;
            let remote = session
                .dev
                .get_object_by_heap_object_id(heap_object_id, object_group)
                .map_err(CdpError::from)?;
            Ok(DispatchOutcome::result(object(vec![(
                "result",
                remote_object(&remote),
            )])))
        }
        other => Err(CdpError::method_not_found(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{DevToolsSession, HostError, RemoteObject};

    struct Scripted;

    impl DevToolsSession for Scripted {
        fn take_heap_snapshot(&mut self) -> Result<Vec<String>, HostError> {
            Ok(vec!["{\"a\":1}".into(), "{\"b\":2}".into()])
        }
        fn get_heap_object_id(&mut self, _object_id: &str) -> Result<String, HostError> {
            Ok("heap-1".into())
        }
        fn get_object_by_heap_object_id(
            &mut self,
            _heap_object_id: &str,
            _object_group: Option<&str>,
        ) -> Result<RemoteObject, HostError> {
            Ok(RemoteObject::number(9.0))
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
    fn take_heap_snapshot_emits_chunks_then_progress() {
        let mut session = session();
        let request =
            CdpRequest::parse(r#"{"id":1,"method":"HeapProfiler.takeHeapSnapshot"}"#).unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(outcome.events.len(), 3);
        assert_eq!(
            outcome.events[0].get("method").unwrap().as_str(),
            Some("HeapProfiler.addHeapSnapshotChunk")
        );
        assert_eq!(
            outcome.events[2].get("method").unwrap().as_str(),
            Some("HeapProfiler.reportHeapSnapshotProgress")
        );
    }

    #[test]
    fn get_object_by_heap_object_id_returns_the_remote_object() {
        let mut session = session();
        let request = CdpRequest::parse(
            r#"{"id":1,"method":"HeapProfiler.getObjectByHeapObjectId","params":{"objectId":"heap-1"}}"#,
        )
        .unwrap();
        let outcome = dispatch(&mut session, &request).unwrap();
        assert_eq!(
            outcome.result.get("result").unwrap().get("value").unwrap().as_f64(),
            Some(9.0)
        );
    }
}
