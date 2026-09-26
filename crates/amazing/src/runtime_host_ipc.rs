//! Optional browser-owned process navigation state for the fallback driver.
//! The default driver path stays active until the pinned history parity gate is measured.
use amazing_fetch::HttpUrl;
use amazing_process::history::{
    CommitKind, DocumentStatus, Eligibility, HistorySnapshot, NavigationTicket, SessionHistory,
    SessionHistoryEntry,
};
use amazing_process::network_v2::NetworkSession;
use amazing_process::protocol::{
    ByteBuf, HeaderField, HistoryActivate, HistoryProxy, NETWORK_CHUNK, NETWORK_MAX_RESPONSE_BODY,
    NETWORK_VERSION, NetworkContext, NetworkHead, NetworkReply, NetworkStart, NetworkStarted,
    WireCookieSite, WireOrigin,
};
use amazing_process::{Capabilities, ProcessHost, RestartPolicy, Role};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ROOT_NAVIGABLE: u64 = 1;
const RENDERER_ID: u64 = 1;
const MAX_REDIRECTS: usize = 20;
const TIMEOUT: Duration = Duration::from_secs(15);

struct ChildContext {
    top_level_origin: WireOrigin,
    ancestor_origins: Vec<WireOrigin>,
    referrer: String,
}

/// Explicit opt-in for the process-backed navigation experiment.
/// `AMAZING_IPC_CHILD` may point at an `ipc-demo-child` built from this workspace.
pub fn enabled() -> bool {
    std::env::var_os("AMAZING_IPC_HISTORY").as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub(crate) fn child_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("AMAZING_IPC_CHILD") {
        return Ok(PathBuf::from(path));
    }
    let mut path = std::env::current_exe().map_err(|error| error.to_string())?;
    path.set_file_name(if cfg!(windows) {
        "ipc-demo-child.exe"
    } else {
        "ipc-demo-child"
    });
    Ok(path)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

struct Tab {
    history: SessionHistory,
    network: NetworkSession,
    renderer: ProcessHost,
    next_navigable: u64,
}
impl Tab {
    fn launch(id: u64, child: &Path) -> Result<Self, String> {
        let network = NetworkSession::launch(child, id, TIMEOUT)
            .map_err(|error| format!("network process: {error:?}"))?;
        let renderer = ProcessHost::spawn(
            child,
            Role::Renderer,
            Capabilities(BTreeMap::new()),
            RestartPolicy::Never,
        )
        .map_err(|error| format!("renderer process: {error:?}"))?;
        let history = SessionHistory::new(id, ROOT_NAVIGABLE)
            .map_err(|error| format!("session history: {error:?}"))?;
        Ok(Self {
            history,
            network,
            renderer,
            next_navigable: ROOT_NAVIGABLE + 1,
        })
    }
    fn history_proxy(&self) -> HistoryProxy {
        HistoryProxy::new(self.renderer.connection.clone(), TIMEOUT)
    }
    fn child_context(&self, navigable: u64) -> Result<(WireOrigin, ChildContext), String> {
        let mut cursor = self
            .history
            .parent_navigable(navigable)
            .ok_or_else(|| "child has no parent navigable".to_owned())?;
        let mut ancestors = Vec::new();
        let mut referrer = String::new();
        loop {
            let entry = self
                .history
                .active_entry(cursor)
                .ok_or_else(|| "child parent is not active".to_owned())?;
            if referrer.is_empty() {
                referrer = entry.url.clone();
            }
            let origin = HttpUrl::parse(&entry.url)
                .map_err(|error| format!("child parent URL: {error:?}"))?
                .origin()
                .serialize();
            ancestors.push(WireOrigin::Tuple(origin));
            if cursor == ROOT_NAVIGABLE {
                break;
            }
            cursor = self
                .history
                .parent_navigable(cursor)
                .ok_or_else(|| "child ancestry does not reach tab root".to_owned())?;
            if ancestors.len() >= 32 {
                return Err("child ancestry limit".into());
            }
        }
        let initiator = ancestors[0].clone();
        ancestors.reverse();
        Ok((
            initiator,
            ChildContext {
                top_level_origin: ancestors[0].clone(),
                ancestor_origins: ancestors,
                referrer,
            },
        ))
    }
    fn freeze(
        &mut self,
        entry: &SessionHistoryEntry,
        eligibility: Eligibility,
    ) -> Result<(), String> {
        let state = self
            .history
            .document(entry.document)
            .ok_or_else(|| "missing outgoing document".to_owned())?;
        if state.status != DocumentStatus::Active {
            return Ok(());
        }
        let epoch = state.epoch;
        match self.history.freeze_request(
            entry.document,
            RENDERER_ID,
            epoch,
            eligibility,
            4096,
            now_ms(),
        ) {
            Ok(request) => {
                let ack = self
                    .history_proxy()
                    .freeze(request)
                    .map_err(|error| format!("freeze: {error:?}"))?;
                let evicted = self
                    .history
                    .freeze_ack(ack, now_ms())
                    .map_err(|error| format!("freeze acknowledgement: {error:?}"))?;
                for document in evicted {
                    self.discard(document)?;
                }
            }
            Err(_) => self.discard(entry.document)?,
        }
        Ok(())
    }
    fn freeze_current_tree(&mut self, eligibility: Eligibility) -> Result<(), String> {
        let current = self.history.current_step();
        let mut entries: Vec<_> = self
            .history
            .steps()
            .iter()
            .find(|step| Some(step.id) == current)
            .into_iter()
            .flat_map(|step| step.entries.values())
            .filter_map(|id| self.history.entry(*id).cloned())
            .collect();
        // The browser mints each child ID after its parent, so descending IDs
        // quiesce nested descendants before their ancestors.
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.navigable));
        for entry in entries {
            self.freeze(&entry, eligibility)?;
        }
        Ok(())
    }
    fn discard(&mut self, document: u64) -> Result<(), String> {
        let request = self
            .history
            .discard(document)
            .map_err(|error| format!("discard: {error:?}"))?;
        self.history_proxy()
            .discard(request)
            .map_err(|error| format!("renderer discard: {error:?}"))?;
        Ok(())
    }
    fn drain_orphans(&mut self) -> Result<(), String> {
        for request in self.history.take_orphaned_discards() {
            let _ = self.history_proxy().discard(request);
        }
        Ok(())
    }
    fn activate(&mut self, entry: &SessionHistoryEntry) -> Result<(), String> {
        let epoch = self
            .history
            .document(entry.document)
            .ok_or_else(|| "missing new document".to_owned())?
            .epoch;
        let acknowledged = self
            .history_proxy()
            .activate(HistoryActivate {
                document: entry.document,
                epoch,
                history_state: entry.snapshot.history_state.clone().map(ByteBuf),
            })
            .map_err(|error| format!("renderer activation: {error:?}"))?;
        if !acknowledged {
            return Err("renderer declined document activation".into());
        }
        Ok(())
    }
    fn fetch_navigation(
        &mut self,
        url: &str,
        ticket_id: u64,
        reload_or_history: bool,
        initiator: WireOrigin,
        child: Option<ChildContext>,
    ) -> Result<(NetworkHead, Vec<u8>), String> {
        if child.is_none() {
            self.network
                .begin_navigation()
                .map_err(|error| format!("navigation epoch: {error:?}"))?;
        }
        let mut request = navigation_request(url, ticket_id, reload_or_history, initiator, child)?;
        let mut started = self
            .network
            .start(request.clone())
            .map_err(|error| format!("start navigation: {error:?}"))?;
        let mut seen = BTreeSet::from([request.url.clone()]);
        for _ in 0..=MAX_REDIRECTS {
            let (head, body) = collect_navigation(&self.network, &started)?;
            let location = head
                .headers
                .iter()
                .find(|field| field.name.eq_ignore_ascii_case("location"))
                .map(|field| field.value.as_str());
            if matches!(head.status, 301 | 302 | 303 | 307 | 308) {
                let location = location.ok_or_else(|| "redirect without Location".to_owned())?;
                let base = amazing_net::Url::parse(&head.url)
                    .map_err(|error| format!("redirect URL: {error:?}"))?;
                let next = amazing_net::resolve(location, &base)
                    .map_err(|error| format!("redirect Location: {error:?}"))?
                    .to_string();
                if !seen.insert(next.clone()) {
                    return Err("navigation redirect loop".into());
                }
                request.url = next;
                request.base_url = head.url.clone();
                if head.status == 303 {
                    request.method = "GET".into();
                    request.body_present = false;
                    request.body = ByteBuf(Vec::new());
                }
                started = self
                    .network
                    .restart_navigation(&started, &head, request.clone())
                    .map_err(|error| format!("restart navigation: {error:?}"))?;
                continue;
            }
            return Ok((head, body));
        }
        Err("navigation redirect limit".into())
    }
}
impl Drop for Tab {
    fn drop(&mut self) {
        let _ = self.network.shutdown();
        let _ = self.renderer.kill();
    }
}

fn collect_navigation(
    session: &NetworkSession,
    started: &NetworkStarted,
) -> Result<(NetworkHead, Vec<u8>), String> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut head = None;
    let mut body = Vec::new();
    loop {
        if std::time::Instant::now() >= deadline {
            let _ = session.cancel(started);
            return Err("navigation timed out".into());
        }
        match session
            .read(started, NETWORK_CHUNK)
            .map_err(|error| format!("read navigation: {error:?}"))?
        {
            NetworkReply::Pending(()) => std::thread::yield_now(),
            NetworkReply::Head(value) => {
                if (value.id, value.client_id, value.epoch)
                    != (started.id, started.client_id, started.epoch)
                    || head.replace(value).is_some()
                {
                    return Err("duplicate response head".into());
                }
            }
            NetworkReply::Body(chunk) => {
                if (chunk.id, chunk.epoch) != (started.id, started.epoch)
                    || head.is_none()
                    || body
                        .len()
                        .checked_add(chunk.bytes.0.len())
                        .is_none_or(|len| len > NETWORK_MAX_RESPONSE_BODY as usize)
                {
                    let _ = session.cancel(started);
                    return Err("navigation body limit".into());
                }
                body.extend_from_slice(&chunk.bytes.0);
            }
            NetworkReply::Complete(done) => {
                if (done.id, done.epoch) != (started.id, started.epoch) {
                    return Err("foreign navigation completion".into());
                }
                if let Some(error) = done.error {
                    return Err(format!("network completion: {error}"));
                }
                return head
                    .map(|head| (head, body))
                    .ok_or_else(|| "response without head".into());
            }
        }
    }
}

fn navigation_request(
    url: &str,
    ticket_id: u64,
    reload_or_history: bool,
    initiator: WireOrigin,
    child: Option<ChildContext>,
) -> Result<NetworkStart, String> {
    HttpUrl::parse(url).map_err(|error| format!("navigation URL: {error:?}"))?;
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX);
    let child_navigation = child.is_some();
    let top_level_origin = child
        .as_ref()
        .map(|context| context.top_level_origin.clone());
    let ancestor_origins = child
        .as_ref()
        .map_or_else(Vec::new, |context| context.ancestor_origins.clone());
    let referrer = child
        .as_ref()
        .map_or_else(|| "about:client".into(), |context| context.referrer.clone());
    Ok(NetworkStart {
        version: NETWORK_VERSION,
        client_id: 0,
        epoch: 0,
        url: url.into(),
        base_url: url.into(),
        method: "GET".into(),
        headers: vec![HeaderField {
            name: "Accept".into(),
            value: "text/html".into(),
        }],
        body_present: false,
        body: ByteBuf(Vec::new()),
        destination: if child_navigation {
            "frame"
        } else {
            "document"
        }
        .into(),
        mode: "navigate".into(),
        credentials: "include".into(),
        redirect: "manual".into(),
        cache: "default".into(),
        referrer,
        referrer_policy: "strict-origin-when-cross-origin".into(),
        response_limit: NETWORK_MAX_RESPONSE_BODY,
        navigation_reload_or_history: reload_or_history && !child_navigation,
        navigation_client_reserved_id: (!child_navigation).then_some(ticket_id),
        context: NetworkContext {
            cookie_site: WireCookieSite::Origin(initiator.clone()),
            origin: initiator,
            top_level_origin,
            ancestor_origins,
            sandbox_flags: 0,
            now_unix,
            restricts_mixed_content: false,
            image_set_initiator: false,
            top_level_navigation: !child_navigation,
            embedder_policy: "unsafe-none".into(),
            no_cors_media_state: "none".into(),
            csp_headers: Vec::new(),
        },
    })
}

pub struct Driver {
    child: PathBuf,
    tabs: BTreeMap<u64, Tab>,
}
impl Driver {
    pub fn new(child: PathBuf) -> Self {
        Self {
            child,
            tabs: BTreeMap::new(),
        }
    }
    fn tab(&mut self, tab_id: u64) -> Result<&mut Tab, String> {
        if tab_id == 0 {
            return Err("zero tab ID".into());
        }
        if !self.tabs.contains_key(&tab_id) {
            let tab = Tab::launch(tab_id, &self.child)?;
            self.tabs.insert(tab_id, tab);
        }
        Ok(self.tabs.get_mut(&tab_id).unwrap())
    }
    pub fn navigate<T>(
        &mut self,
        tab_id: u64,
        url: &str,
        record: bool,
        history_pos: usize,
        outgoing: HistorySnapshot,
        eligibility: Eligibility,
        render: impl FnOnce(&str, &[u8]) -> Result<T, String>,
    ) -> Result<T, String> {
        let child = self.child.clone();
        let tab = self.tab(tab_id)?;
        if tab
            .renderer
            .crashed()
            .map_err(|error| format!("renderer status: {error:?}"))?
        {
            tab.history.renderer_crashed(RENDERER_ID);
            // A replacement renderer receives a fresh document token below.
            tab.renderer = ProcessHost::spawn(
                &child,
                Role::Renderer,
                Capabilities(BTreeMap::new()),
                RestartPolicy::Never,
            )
            .map_err(|error| format!("restart renderer: {error:?}"))?;
        }
        if let Some(active) = tab.history.active_entry(ROOT_NAVIGABLE).cloned()
            && tab
                .history
                .document(active.document)
                .is_some_and(|state| state.status == DocumentStatus::Active)
        {
            tab.history
                .save_snapshot(RENDERER_ID, active.document, outgoing)
                .map_err(|error| format!("snapshot: {error:?}"))?;
        }
        if !record
            && tab.history.steps().get(history_pos).map(|step| step.id)
                != tab.history.current_step()
        {
            if let Some(step) = tab.history.steps().get(history_pos).map(|step| step.id) {
                tab.freeze_current_tree(eligibility)?;
                let traversal = tab
                    .history
                    .traverse(step)
                    .map_err(|error| format!("traversal: {error:?}"))?;
                let entry = traversal
                    .entries
                    .get(&ROOT_NAVIGABLE)
                    .ok_or_else(|| "traversal lacks root entry".to_owned())?;
                let root_frozen = tab
                    .history
                    .document(entry.document)
                    .is_some_and(|state| state.status == DocumentStatus::Frozen);
                for target in traversal.entries.values() {
                    if tab
                        .history
                        .document(target.document)
                        .is_some_and(|state| state.status == DocumentStatus::Frozen)
                    {
                        let request = tab
                            .history
                            .restore_request(target.document, RENDERER_ID, now_ms())
                            .map_err(|error| format!("restore: {error:?}"))?;
                        let ack = tab
                            .history_proxy()
                            .restore(request)
                            .map_err(|error| format!("renderer restore: {error:?}"))?;
                        tab.history
                            .restore_ack(ack)
                            .map_err(|error| format!("restore acknowledgement: {error:?}"))?;
                    }
                }
                if root_frozen {
                    return render(&entry.url, &[]);
                }
            }
        }
        let current = tab.history.active_entry(ROOT_NAVIGABLE).cloned();
        let source = current.as_ref().map(|entry| entry.document);
        let kind = if current.is_none() || record {
            CommitKind::Navigate
        } else {
            CommitKind::Reload
        };
        let committed_snapshot = if kind == CommitKind::Reload {
            current.as_ref().unwrap().snapshot.clone()
        } else {
            HistorySnapshot::default()
        };
        let ticket = tab
            .history
            .begin(RENDERER_ID, ROOT_NAVIGABLE, source, kind)
            .map_err(|error| format!("navigation ticket: {error:?}"))?;
        let initiator = current
            .as_ref()
            .and_then(|entry| HttpUrl::parse(&entry.url).ok())
            .map(|url| WireOrigin::Tuple(url.origin().serialize()))
            .unwrap_or(WireOrigin::Opaque(u128::from(tab_id)));
        let (head, body) = match tab.fetch_navigation(url, ticket.id, !record, initiator, None) {
            Ok(response) => response,
            Err(error) => {
                let _ = tab.history.cancel(&ticket);
                return Err(error);
            }
        };
        let output = match render(&head.url, &body) {
            Ok(output) => output,
            Err(error) => {
                let _ = tab.history.cancel(&ticket);
                return Err(error);
            }
        };
        if current.is_some() {
            tab.freeze_current_tree(eligibility)?;
        }
        let entry = tab
            .history
            .commit(ticket, head.url, committed_snapshot)
            .map_err(|error| format!("history commit: {error:?}"))?;
        tab.activate(&entry)?;
        tab.drain_orphans()?;
        Ok(output)
    }
    pub fn create_child(&mut self, tab_id: u64, parent: u64) -> Result<u64, String> {
        let tab = self.tab(tab_id)?;
        let child = tab.next_navigable;
        let next = child.checked_add(1).ok_or("navigable IDs exhausted")?;
        tab.history
            .add_child(parent, child)
            .map_err(|error| format!("child navigable: {error:?}"))?;
        tab.next_navigable = next;
        Ok(child)
    }
    /// Mint the child ticket before asking the network process for its response.
    fn begin_child_navigation(
        &mut self,
        tab_id: u64,
        navigable: u64,
    ) -> Result<NavigationTicket, String> {
        let tab = self.tab(tab_id)?;
        let source = tab
            .history
            .active_entry(navigable)
            .map(|entry| entry.document);
        tab.history
            .begin(RENDERER_ID, navigable, source, CommitKind::Navigate)
            .map_err(|error| format!("child navigation ticket: {error:?}"))
    }
    pub fn navigate_child<T>(
        &mut self,
        tab_id: u64,
        navigable: u64,
        url: &str,
        outgoing: HistorySnapshot,
        eligibility: Eligibility,
        render: impl FnOnce(&str, &[u8]) -> Result<T, String>,
    ) -> Result<T, String> {
        let (initiator, context) = self.tab(tab_id)?.child_context(navigable)?;
        let ticket = self.begin_child_navigation(tab_id, navigable)?;
        let response =
            self.tab(tab_id)?
                .fetch_navigation(url, ticket.id, false, initiator, Some(context));
        let (head, body) = match response {
            Ok(response) => response,
            Err(error) => {
                let _ = self.tab(tab_id)?.history.cancel(&ticket);
                return Err(error);
            }
        };
        self.finish_child_navigation(ticket, head, body, outgoing, eligibility, render)
    }
    /// Commit only the terminal V2 response received by `navigate_child`.
    fn finish_child_navigation<T>(
        &mut self,
        ticket: NavigationTicket,
        head: NetworkHead,
        body: Vec<u8>,
        outgoing: HistorySnapshot,
        eligibility: Eligibility,
        render: impl FnOnce(&str, &[u8]) -> Result<T, String>,
    ) -> Result<T, String> {
        let tab = self.tab(ticket.tab)?;
        if head.client_id != ticket.tab
            || head.epoch != tab.network.epoch()
            || matches!(head.status, 301 | 302 | 303 | 307 | 308)
            || body.len() > NETWORK_MAX_RESPONSE_BODY as usize
            || HttpUrl::parse(&head.url).is_err()
        {
            let _ = tab.history.cancel(&ticket);
            return Err("invalid child navigation response".into());
        }
        let prior = tab.history.active_entry(ticket.navigable).cloned();
        if let Some(entry) = &prior {
            tab.history
                .save_snapshot(RENDERER_ID, entry.document, outgoing)
                .map_err(|error| format!("child snapshot: {error:?}"))?;
        }
        let output = match render(&head.url, &body) {
            Ok(output) => output,
            Err(error) => {
                let _ = tab.history.cancel(&ticket);
                return Err(error);
            }
        };
        if let Some(entry) = prior {
            tab.freeze(&entry, eligibility)?;
        }
        let entry = tab
            .history
            .commit(ticket, head.url, HistorySnapshot::default())
            .map_err(|error| format!("child history commit: {error:?}"))?;
        tab.activate(&entry)?;
        tab.drain_orphans()?;
        Ok(output)
    }
    pub fn save_snapshot(
        &mut self,
        tab_id: u64,
        navigable: u64,
        snapshot: HistorySnapshot,
    ) -> Result<(), String> {
        let tab = self.tab(tab_id)?;
        let document = tab
            .history
            .active_entry(navigable)
            .ok_or_else(|| "snapshot has no active entry".to_owned())?
            .document;
        tab.history
            .save_snapshot(RENDERER_ID, document, snapshot)
            .map_err(|error| format!("save snapshot: {error:?}"))
    }
    pub fn same_document(
        &mut self,
        tab_id: u64,
        navigable: u64,
        url: String,
        replace: bool,
        snapshot: HistorySnapshot,
    ) -> Result<SessionHistoryEntry, String> {
        let tab = self.tab(tab_id)?;
        let source = tab
            .history
            .active_entry(navigable)
            .ok_or_else(|| "same-document navigation has no source".to_owned())?
            .document;
        let kind = if replace {
            CommitKind::Replace
        } else {
            CommitKind::Push
        };
        let ticket = tab
            .history
            .begin(RENDERER_ID, navigable, Some(source), kind)
            .map_err(|error| format!("same-document ticket: {error:?}"))?;
        let entry = tab
            .history
            .commit(ticket, url, snapshot)
            .map_err(|error| format!("same-document commit: {error:?}"))?;
        tab.drain_orphans()?;
        Ok(entry)
    }
    pub fn traverse_same_document(
        &mut self,
        tab_id: u64,
        step: u64,
        outgoing: HistorySnapshot,
    ) -> Result<SessionHistoryEntry, String> {
        let tab = self.tab(tab_id)?;
        let active = tab
            .history
            .active_entry(ROOT_NAVIGABLE)
            .cloned()
            .ok_or("same-document traversal has no active entry")?;
        let target = tab
            .history
            .steps()
            .iter()
            .find(|candidate| candidate.id == step)
            .and_then(|candidate| candidate.entries.get(&ROOT_NAVIGABLE))
            .and_then(|id| tab.history.entry(*id))
            .cloned()
            .ok_or("same-document traversal target missing")?;
        if target.document != active.document {
            return Err("traversal crosses documents".into());
        }
        tab.history
            .save_snapshot(RENDERER_ID, active.document, outgoing)
            .map_err(|error| format!("outgoing snapshot: {error:?}"))?;
        tab.history
            .traverse(step)
            .map_err(|error| format!("same-document traversal: {error:?}"))?;
        tab.history
            .active_entry(ROOT_NAVIGABLE)
            .cloned()
            .ok_or("same-document traversal lost root".into())
    }
    pub fn renderer_crashed(&mut self, tab_id: u64) -> Result<Vec<u64>, String> {
        Ok(self.tab(tab_id)?.history.renderer_crashed(RENDERER_ID))
    }
    pub fn history(&self, tab_id: u64) -> Option<&SessionHistory> {
        self.tabs.get(&tab_id).map(|tab| &tab.history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn navigation_request_mints_browser_owned_manual_redirect_fields() {
        let request = navigation_request(
            "http://example.test/page",
            42,
            false,
            WireOrigin::Opaque(42),
            None,
        )
        .unwrap();
        assert_eq!(request.mode, "navigate");
        assert_eq!(request.redirect, "manual");
        assert_eq!(request.navigation_client_reserved_id, Some(42));
        assert!(request.context.top_level_navigation);
        assert_eq!(request.response_limit, NETWORK_MAX_RESPONSE_BODY);
        let parent = WireOrigin::Tuple("http://example.test".into());
        let child = navigation_request(
            "http://example.test/frame",
            43,
            false,
            parent.clone(),
            Some(ChildContext {
                top_level_origin: parent.clone(),
                ancestor_origins: vec![parent],
                referrer: "http://example.test/page".into(),
            }),
        )
        .unwrap();
        assert_eq!(child.destination, "frame");
        assert!(!child.context.top_level_navigation);
        assert_eq!(child.navigation_client_reserved_id, None);
    }

    /// Build `ipc-demo-child` and set AMAZING_IPC_CHILD to its absolute path.
    #[test]
    #[ignore = "requires the amazing-process child binary"]
    fn redirect_history_restore_and_renderer_crash_use_browser_owned_tokens() {
        let child = PathBuf::from(std::env::var("AMAZING_IPC_CHILD").unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    assert!(headers.len() < 16384);
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    headers.push(byte[0]);
                }
                let request = String::from_utf8(headers).unwrap();
                let (status, location, body): (&str, &str, &[u8]) =
                    if request.starts_with("GET /start ") {
                        ("302 Found", "Location: /final\r\n", b"")
                    } else if request.starts_with("GET /final ") {
                        ("200 OK", "", b"first")
                    } else if request.starts_with("GET /child ") {
                        ("302 Found", "Location: /child-final\r\n", b"")
                    } else if request.starts_with("GET /child-final ") {
                        ("200 OK", "", b"child")
                    } else if request.starts_with("GET /other ") {
                        ("200 OK", "", b"second")
                    } else {
                        assert!(request.starts_with("GET /crash "));
                        ("200 OK", "", b"reloaded")
                    };
                write!(stream, "HTTP/1.1 {status}\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let mut driver = Driver::new(child);
        let first = driver
            .navigate(
                7,
                &format!("{origin}/start"),
                true,
                0,
                HistorySnapshot::default(),
                Eligibility::default(),
                |url, body| {
                    assert_eq!(url, format!("{origin}/final"));
                    Ok(body.to_vec())
                },
            )
            .unwrap();
        assert_eq!(first, b"first");
        let first_document = driver.history(7).unwrap().active_entry(1).unwrap().document;
        let child_navigable = driver.create_child(7, ROOT_NAVIGABLE).unwrap();
        let parent_epoch = driver.tabs[&7].network.epoch();
        driver
            .navigate_child(
                7,
                child_navigable,
                &format!("{origin}/child"),
                HistorySnapshot::default(),
                Eligibility::default(),
                |url, body| {
                    assert_eq!(url, format!("{origin}/child-final"));
                    assert_eq!(body, b"child");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(driver.tabs[&7].network.epoch(), parent_epoch);
        let child_document = driver
            .history(7)
            .unwrap()
            .active_entry(child_navigable)
            .unwrap()
            .document;
        driver
            .navigate(
                7,
                &format!("{origin}/other"),
                true,
                1,
                HistorySnapshot {
                    scroll: vec![(1, 0, 23)],
                    ..Default::default()
                },
                Eligibility::default(),
                |_, body| {
                    assert_eq!(body, b"second");
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            driver
                .history(7)
                .unwrap()
                .entry(driver.history(7).unwrap().steps()[0].entries[&1])
                .unwrap()
                .snapshot
                .scroll,
            vec![(1, 0, 23)]
        );
        assert_eq!(
            driver
                .history(7)
                .unwrap()
                .document(first_document)
                .unwrap()
                .status,
            DocumentStatus::Frozen
        );
        assert_eq!(
            driver
                .history(7)
                .unwrap()
                .document(child_document)
                .unwrap()
                .status,
            DocumentStatus::Frozen
        );
        driver
            .navigate(
                7,
                &format!("{origin}/final"),
                false,
                1,
                HistorySnapshot::default(),
                Eligibility::default(),
                |_, body| {
                    assert!(body.is_empty());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            driver.history(7).unwrap().active_entry(1).unwrap().document,
            first_document
        );
        assert_eq!(
            driver
                .history(7)
                .unwrap()
                .document(child_document)
                .unwrap()
                .status,
            DocumentStatus::Active
        );
        driver.tabs.get_mut(&7).unwrap().renderer.kill().unwrap();
        driver
            .navigate(
                7,
                &format!("{origin}/crash"),
                true,
                1,
                HistorySnapshot::default(),
                Eligibility::default(),
                |_, body| {
                    assert_eq!(body, b"reloaded");
                    Ok(())
                },
            )
            .unwrap();
        assert_ne!(
            driver.history(7).unwrap().active_entry(1).unwrap().document,
            first_document
        );
        server.join().unwrap();
    }
}
