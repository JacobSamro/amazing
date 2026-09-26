//! Per-connection session state, CDP domain dispatch and engine-event polling.
//!
//! A [`SessionState`] owns one attached target: its [`DevToolsSession`], the
//! CDP node-id map for that session, and which domains the client has enabled.
//! Events are only emitted for enabled domains, mirroring Chrome: a client that
//! never called `Network.enable` never sees `Network.*` events.

use std::collections::HashMap;

use crate::domains;
use crate::host::{DevToolsSession, EngineNodeId, TargetInfo};
use crate::json::Json;
use crate::protocol::{CdpError, CdpRequest};

/// The result of a dispatched command: the `result` object plus any events that
/// must be emitted immediately after the response.
#[derive(Clone, Debug, PartialEq)]
pub struct DispatchOutcome {
    pub result: Json,
    pub events: Vec<Json>,
}

impl DispatchOutcome {
    /// A result with no immediate events.
    pub fn result(result: Json) -> Self {
        Self {
            result,
            events: Vec::new(),
        }
    }

    /// An empty object result.
    pub fn empty() -> Self {
        Self::result(Json::Object(Vec::new()))
    }

    /// Attaches immediate events.
    pub fn with_events(mut self, events: Vec<Json>) -> Self {
        self.events = events;
        self
    }
}

/// Which domains the client has enabled on this connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnabledDomains {
    pub page: bool,
    pub runtime: bool,
    pub dom: bool,
    pub css: bool,
    pub network: bool,
    pub log: bool,
    pub fetch: bool,
    pub security: bool,
    pub accessibility: bool,
    pub debugger: bool,
    pub dom_storage: bool,
    pub indexed_db: bool,
}

/// Maps engine node identity to per-session CDP node ids.
///
/// CDP node ids are dense integers assigned by the agent; `backendNodeId` is the
/// engine's own id, already carried by [`EngineNodeId`].
#[derive(Debug, Default)]
pub struct NodeIdMap {
    forward: HashMap<EngineNodeId, u64>,
    reverse: HashMap<u64, EngineNodeId>,
    next: u64,
}

impl NodeIdMap {
    /// Returns the CDP node id for `engine`, assigning one on first sight.
    pub fn id(&mut self, engine: EngineNodeId) -> u64 {
        if let Some(&id) = self.forward.get(&engine) {
            return id;
        }
        self.next += 1;
        let id = self.next;
        self.forward.insert(engine, id);
        self.reverse.insert(id, engine);
        id
    }

    /// Resolves a CDP node id back to engine identity.
    pub fn engine(&self, node_id: u64) -> Option<EngineNodeId> {
        self.reverse.get(&node_id).copied()
    }

    /// The `backendNodeId` for a CDP node id (the engine id's raw value).
    pub fn backend_id(&self, node_id: u64) -> Option<u64> {
        self.reverse.get(&node_id).map(|engine| engine.0)
    }
}

/// One attached target on one WebSocket connection.
pub struct SessionState {
    pub session_id: String,
    pub target: TargetInfo,
    pub dev: Box<dyn DevToolsSession>,
    pub nodes: NodeIdMap,
    pub enabled: EnabledDomains,
    /// `DOM.performSearch` results, keyed by the search id handed to the client.
    pub searches: HashMap<String, Vec<EngineNodeId>>,
    next_search: u64,
}

impl SessionState {
    /// Creates a session with no domains enabled.
    pub fn new(session_id: impl Into<String>, target: TargetInfo, dev: Box<dyn DevToolsSession>) -> Self {
        Self {
            session_id: session_id.into(),
            target,
            dev,
            nodes: NodeIdMap::default(),
            enabled: EnabledDomains::default(),
            searches: HashMap::new(),
            next_search: 0,
        }
    }

    /// Stores a search result set and returns its id.
    pub fn store_search(&mut self, results: Vec<EngineNodeId>) -> String {
        self.next_search += 1;
        let id = format!("search:{}", self.next_search);
        self.searches.insert(id.clone(), results);
        id
    }

    /// Emits events that have accumulated in the engine since the last poll,
    /// filtered by the enabled domains.
    pub fn poll_events(&mut self) -> Vec<Json> {
        let enabled = self.enabled;
        let mut events = Vec::new();
        if enabled.page {
            for item in self.dev.drain_lifecycle() {
                events.push(domains::page::lifecycle_event(&item));
            }
        }
        if enabled.runtime {
            for item in self.dev.drain_console() {
                events.push(domains::runtime::console_event(&item));
            }
            for item in self.dev.drain_exceptions() {
                events.push(domains::runtime::exception_thrown_event(&item));
            }
        }
        if enabled.network {
            for item in self.dev.drain_network_requests() {
                events.push(domains::network::request_event(&item));
            }
            for item in self.dev.drain_network_responses() {
                events.push(domains::network::response_event(&item));
            }
            for item in self.dev.drain_network_finished() {
                events.push(domains::network::finished_event(&item));
            }
            for item in self.dev.drain_network_failed() {
                events.push(domains::network::failed_event(&item));
            }
        }
        if enabled.log {
            for item in self.dev.drain_log() {
                events.push(domains::log::entry_event(&item));
            }
        }
        if enabled.fetch {
            for item in self.dev.drain_intercepted_requests() {
                events.push(domains::fetch::request_paused(&item));
            }
        }
        if enabled.security {
            for state in self.dev.drain_security_states() {
                events.push(domains::security::state_changed(&state));
            }
        }
        if enabled.accessibility {
            if let Some(root) = self.dev.drain_ax_load_complete() {
                events.push(domains::accessibility::load_complete(&root));
            }
            let updated = self.dev.drain_ax_nodes_updated();
            if !updated.is_empty() {
                events.push(domains::accessibility::nodes_updated(&updated));
            }
        }
        if enabled.debugger {
            for script in self.dev.drain_scripts_parsed() {
                events.push(domains::debugger::script_parsed(&script));
            }
            for paused in self.dev.drain_debugger_paused() {
                events.push(domains::debugger::paused(&paused));
            }
        }
        events
    }
}

/// Routes a command to its domain. `Target`/`Browser` are handled by the
/// connection because they act on the host rather than on one page session.
pub fn dispatch(session: &mut SessionState, request: &CdpRequest) -> Result<DispatchOutcome, CdpError> {
    let (domain, _) = request
        .method
        .split_once('.')
        .ok_or_else(|| CdpError::invalid_request("method must be 'Domain.command'"))?;
    match domain {
        "Page" => domains::page::dispatch(session, request),
        "Runtime" => domains::runtime::dispatch(session, request),
        "DOM" => domains::dom::dispatch(session, request),
        "CSS" => domains::css::dispatch(session, request),
        "Input" => domains::input::dispatch(session, request),
        "Emulation" => domains::emulation::dispatch(session, request),
        "Overlay" => domains::overlay::dispatch(session, request),
        "Performance" => domains::performance::dispatch(session, request),
        "Security" => domains::security::dispatch(session, request),
        "Network" => domains::network::dispatch(session, request),
        "Log" => domains::log::dispatch(session, request),
        "Fetch" => domains::fetch::dispatch(session, request),
        "Accessibility" => domains::accessibility::dispatch(session, request),
        "Debugger" => domains::debugger::dispatch(session, request),
        "Profiler" => domains::profiler::dispatch(session, request),
        "HeapProfiler" => domains::heap_profiler::dispatch(session, request),
        "DOMSnapshot" => domains::dom_snapshot::dispatch(session, request),
        "Storage" => domains::storage::dispatch(session, request),
        "DOMStorage" => domains::dom_storage::dispatch(session, request),
        "IndexedDB" => domains::indexed_db::dispatch(session, request),
        "Tracing" => domains::tracing::dispatch(session, request),
        _ => Err(CdpError::method_not_found(&request.method)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::CdpRequest;

    #[test]
    fn node_ids_are_assigned_once_and_reversible() {
        let mut map = NodeIdMap::default();
        assert_eq!(map.id(EngineNodeId(42)), 1);
        assert_eq!(map.id(EngineNodeId(42)), 1);
        assert_eq!(map.id(EngineNodeId(7)), 2);
        assert_eq!(map.engine(1), Some(EngineNodeId(42)));
        assert_eq!(map.backend_id(2), Some(7));
        assert_eq!(map.engine(99), None);
    }

    #[test]
    fn unknown_domain_is_method_not_found() {
        struct Empty;
        impl DevToolsSession for Empty {}
        let mut session = SessionState::new("s", TargetInfo::page("t", "about:blank", ""), Box::new(Empty));
        let request = CdpRequest::parse(r#"{"id":1,"method":"Wat.nope"}"#).unwrap();
        assert_eq!(dispatch(&mut session, &request).unwrap_err().code, crate::protocol::METHOD_NOT_FOUND);
    }
}
