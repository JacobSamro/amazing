//! Child HTML navigables owned by the driver. The DOM arena can hold each
//! child document before JSRT-9 supplies a separate realm and WindowProxy.
use super::*;
use amazing_dom::document_surface::ReadyState;
use amazing_net::url::{ParsedUrl, UrlOrigin};
use amazing_fetch::{HttpUrl, HeaderList, check_frame_embedding,
    api::{ApiRequest, NavigableContext},
    service::{NetworkService, ServiceEvent}};
use amazing_net::dispatch::DispatchLimits;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) type FrameResourceResolver = Rc<dyn Fn(&str) -> Option<String>>;
pub(crate) type ChildNavigationHook = Rc<dyn Fn(ChildNavigation)>;
pub(crate) type FormNavigationHook = Rc<dyn Fn(FormNavigationRequest)>;

#[derive(Clone, Debug)]
pub(crate) struct ChildNavigation {
    pub(crate) frame: NodeIdx,
    pub(crate) requested_src: Option<String>,
    pub(crate) committed_url: String,
    pub(crate) document: Option<NodeIdx>,
}

struct FrameDocument {
    src: Option<String>,
    document: Option<NodeIdx>,
    url: String,
}

#[derive(Default)]
pub(super) struct FrameRuntime {
    documents: HashMap<NodeIdx, FrameDocument>,
    pending_loads: VecDeque<(NodeIdx, Option<NodeIdx>)>,
    resolver: Option<FrameResourceResolver>,
    navigation_hook: Option<ChildNavigationHook>,
    form_navigation_hook: Option<FormNavigationHook>,
}

fn same_origin(parent: &str, child: &str) -> bool {
    let (Ok(parent), Ok(child)) = (ParsedUrl::parse(parent), ParsedUrl::parse(child)) else {
        return false;
    };
    matches!((parent.origin(), child.origin()),
        (UrlOrigin::Tuple(a), UrlOrigin::Tuple(b)) if a == b)
}

fn content_type(response: &amazing_net::Response, logical_url: &str,
    local_file: bool) -> Option<String> {
    let mime = response.content_type().unwrap_or("").to_ascii_lowercase();
    if mime != "application/octet-stream" && !mime.is_empty() {
        return Some(mime);
    }
    if !local_file { return None; }
    // Local file fetches currently label XML and XHTML as octet-stream.
    // Their extension is the WPT server's media-type mapping for these files.
    let path = logical_url.split(['?', '#']).next().unwrap_or(logical_url);
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("xml") => Some("application/xml".into()),
        Some("xhtml") | Some("xht") => Some("application/xhtml+xml".into()),
        Some("html") | Some("htm") => Some("text/html".into()),
        _ => None,
    }
}

fn parse_response(body: &[u8], mime: &str, charset: Option<&str>) -> Result<(Dom, &'static str), String> {
    if mime == "text/html" {
        let decoded = amazing_html::encoding::decode(body, charset);
        return Ok((amazing_html::parse_bytes(body, charset), decoded.encoding.name()));
    }
    if matches!(mime, "text/xml" | "application/xml" | "application/xhtml+xml")
        || mime.ends_with("+xml") {
        let decoded = amazing_html::encoding::decode(body, charset);
        let parsed = amazing_html::parse_xml(&decoded.text)
            .map_err(|error| format!("XML parse failed: {error:?}"))?;
        return Ok((parsed, decoded.encoding.name()));
    }
    Err(format!("unsupported iframe media type: {mime}"))
}

fn fetch_child_document(logical_url: &str, parent_url: &str,
    resolver: Option<&FrameResourceResolver>) -> Result<amazing_net::Response, String> {
    let parent = HttpUrl::parse(parent_url).map_err(|error| format!("iframe parent URL: {error:?}"))?;
    let target = HttpUrl::parse(logical_url).map_err(|error| format!("iframe URL: {error:?}"))?;
    let origin = parent.origin().clone();
    let context = NavigableContext {
        top_level_origin: origin.clone(), parent_origin: origin.clone(),
        ancestor_origins: vec![origin], sandbox_flags: 0,
        frame_csp: Default::default(), referrer: parent.serialize(),
        referrer_policy: "strict-origin-when-cross-origin".into(),
        now_unix: SystemTime::now().duration_since(UNIX_EPOCH)
            .unwrap_or_default().as_secs().min(i64::MAX as u64) as i64,
    };
    // A fixture resolver only changes the transport address. Policy always
    // sees the document's logical URL and its real parent origin.
    if let Some(resolve) = resolver {
        ApiRequest::child_navigation(target.clone(), &context)
            .map_err(|error| format!("iframe request: {error:?}"))?;
        let fetch_url = resolve(logical_url)
            .ok_or_else(|| "unavailable pinned iframe resource".to_owned())?;
        let response = amazing_net::fetch(&fetch_url, &amazing_net::FetchOptions {
            max_response_bytes: 16 * 1024 * 1024, ..Default::default()
        }).map_err(|error| format!("iframe fetch: {error}"))?;
        let mut headers = HeaderList::default();
        for (name, value) in response.headers.iter() {
            headers.append(name, value).map_err(|error| format!("iframe header: {error:?}"))?;
        }
        check_frame_embedding(&target, &headers, &context.ancestor_origins)
            .map_err(|error| format!("iframe embedding: {error}"))?;
        return Ok(response);
    }
    let mut service = NetworkService::new(DispatchLimits::default());
    let id = service.start_child_navigation(target, &context)
        .map_err(|error| format!("iframe navigation: {error:?}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut network_error = None;
    loop {
        for event in service.step(128, Instant::now()) {
            if let ServiceEvent::Network { id: network_id, event } = &event
                && *network_id == id
                && matches!(event, amazing_net::dispatch::NetEvent::Error { .. }) {
                network_error = Some(format!("{event:?}"));
            }
            if let ServiceEvent::Finished { id: finished, result } = event && finished == id {
                let mut response = result.map_err(|error|
                    format!("iframe fetch: {error:?}; {network_error:?}"))?;
                let mut headers = amazing_net::Headers::new();
                for (name, value) in response.headers.raw_entries() {
                    headers.append(name, value.clone())
                        .map_err(|error| format!("iframe header: {error:?}"))?;
                }
                let body = response.body.bytes()
                    .map_err(|error| format!("iframe body: {error:?}"))?;
                if body.len() > 16 * 1024 * 1024 { return Err("iframe body too large".into()); }
                return Ok(amazing_net::Response { url: response.url, status: response.status,
                    reason: response.status_text, headers, body });
            }
        }
        if Instant::now() >= deadline { return Err("iframe fetch timed out".into()); }
        std::thread::sleep(Duration::from_millis(1));
    }
}

impl RuntimeBindings {
    /// IPC-10 installs this when its navigator can consume method, body,
    /// enctype and target together. The current interpreter entry accepts URL only.
    pub(crate) fn set_form_navigation_hook(&self, hook: FormNavigationHook) {
        self.state.borrow_mut().frames.form_navigation_hook = Some(hook);
    }
    pub(crate) fn set_frame_resource_resolver(&self, resolver: FrameResourceResolver) {
        self.state.borrow_mut().frames.resolver = Some(resolver);
    }

    /// IPC-10 can observe committed child navigations without owning HTML parsing.
    pub(crate) fn set_child_navigation_hook(&self, hook: ChildNavigationHook) {
        self.state.borrow_mut().frames.navigation_hook = Some(hook);
    }

    pub(crate) fn take_iframe_loads(&self) -> Vec<(NodeIdx, Option<NodeIdx>)> {
        self.state.borrow_mut().frames.pending_loads.drain(..).collect()
    }

    pub(crate) fn navigate_iframes(&self) {
        let (host, base, resolver) = {
            let state = self.state.borrow();
            (state.host.clone(), state.document_url.clone(), state.frames.resolver.clone())
        };
        let Some(base) = base else { return };
        let candidates = {
            let host = host.borrow();
            host.dom.tree_order(NodeIdx(0)).into_iter().filter_map(|node| {
                (host.dom.namespace(node) == Some(Namespace::Html)
                    && host.dom.local_name(node) == Some("iframe"))
                    .then(|| (node, host.dom.get_attribute_ns(node, None, "src").map(str::to_owned)))
            }).collect::<Vec<_>>()
        };
        let connected: HashSet<_> = candidates.iter().map(|(node, _)| *node).collect();
        self.state.borrow_mut().frames.documents.retain(|node, _| connected.contains(node));
        for (node, src) in candidates {
            if self.state.borrow().frames.documents.get(&node).is_some_and(|old| old.src == src) {
                continue;
            }
            let loaded = (|| -> Result<(Dom, String, String, &'static str), String> {
                let Some(source) = src.as_deref().filter(|source| !source.is_empty()) else {
                    return Ok((amazing_html::parse("<!doctype html><html><head></head><body></body></html>"),
                        "about:blank".into(), "text/html".into(), "UTF-8"));
                };
                let logical_url = ParsedUrl::parse(&base).and_then(|base| base.resolve(source))
                    .map_err(|error| format!("iframe URL: {error:?}"))?.serialize();
                let response = fetch_child_document(&logical_url, &base, resolver.as_ref())?;
                if !response.is_success() { return Err(format!("iframe status {}", response.status)); }
                let mime = content_type(&response, &logical_url,
                    resolver.is_some() || logical_url.starts_with("file:"))
                    .ok_or_else(|| "iframe response has no supported media type".to_owned())?;
                let charset = response.headers.get("content-type").and_then(|header| {
                    header.split(';').skip(1).find_map(|parameter| {
                        let (name, value) = parameter.trim().split_once('=')?;
                        name.trim().eq_ignore_ascii_case("charset")
                            .then(|| value.trim().trim_matches('"'))
                    })
                });
                let (parsed, encoding) = parse_response(&response.body, &mime, charset)?;
                let document_url = if resolver.is_none() { response.url } else { logical_url };
                Ok((parsed, document_url, mime, encoding))
            })();
            let (document, url) = match loaded {
                Ok((parsed, url, mime, encoding)) => {
                    let imported = host.borrow_mut().dom.import_inert_document(&parsed, &url, &mime, encoding);
                    match imported {
                        Ok(document) => {
                            let _ = host.borrow_mut().dom.set_document_ready_state(document, ReadyState::Complete);
                            (Some(document), url)
                        }
                        Err(_) => (None, url),
                    }
                }
                Err(_) => (None, String::new()),
            };
            let mut state = self.state.borrow_mut();
            let navigation = ChildNavigation { frame: node, requested_src: src.clone(),
                committed_url: url.clone(), document };
            state.frames.documents.insert(node, FrameDocument { src, document, url });
            state.frames.pending_loads.push_back((node, document));
            let hook = state.frames.navigation_hook.clone();
            drop(state);
            if let Some(hook) = hook { hook(navigation); }
        }
    }
}

impl RuntimeBindingHost<'_> {
    /// `[LegacyFactoryFunction=Option]` from the HTML standard: builds a
    /// detached `<option>` with text, value, default-selected and selectedness.
    pub(super) fn install_html_form_globals(&mut self) -> Result<(), Throw> {
        let option = self.native("Option", 4, NativeEntry::OptionConstructor,
            HostCallPolicy::CallAndConstruct);
        let global = self.js.binding_global_object();
        self.define_data_property(&global, "Option", option, true, false, true)
    }

    pub(super) fn option_constructor(&mut self, args: &[JsValue]) -> Result<JsValue, Throw> {
        // WebIDL optional DOMString arguments treat an explicit `undefined` as
        // absent, so `new Option(undefined, undefined)` has no text or value.
        let text = match args.first() {
            Some(value) if !matches!(value, JsValue::Undefined) => Some(self.to_dom_string(value)?),
            _ => None,
        };
        let value = match args.get(1) {
            Some(value) if !matches!(value, JsValue::Undefined) => Some(self.to_dom_string(value)?),
            _ => None,
        };
        let has_default = args.get(2).is_some();
        let default_selected = args.get(2).is_some_and(|v| self.to_boolean(v));
        let has_selected = args.get(3).is_some();
        let selected = args.get(3).is_some_and(|v| self.to_boolean(v));
        let document = self.current_document();
        let node = self.with_dom(|dom| {
            let node = dom.create_element_local_in(
                document, Some("http://www.w3.org/1999/xhtml"), "option")?;
            if let Some(text) = &text {
                if !text.is_empty() {
                    dom.create_text(node, &String::from_utf16_lossy(text));
                }
            }
            if let Some(value) = &value {
                dom.set_attribute(node, "value", &String::from_utf16_lossy(value))?;
            }
            if default_selected {
                dom.set_attribute(node, "selected", "")?;
            }
            // Selectedness comes only from the `selected` argument; a
            // defaultSelected-only option keeps the attribute but is not
            // selected, and neither path sets the dirty flag.
            if has_selected || has_default {
                let selectedness = has_selected && selected;
                dom.set_option_selectedness(node, selectedness)?;
            }
            Ok(node)
        })?;
        self.wrap_node(node)
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn navigate_form(&mut self, request: FormNavigationRequest) -> Result<(), Throw> {
        let hook = self.state.borrow().frames.form_navigation_hook.clone();
        if let Some(hook) = hook {
            hook(request);
        } else {
            // Current top-level navigation entry; IPC-10 replaces this URL-only
            // fallback with the structured request above for POST and targets.
            self.js.binding_location_navigate(&request.url, false);
            self.state.borrow_mut().document_url = Some(request.url);
        }
        Ok(())
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn install_iframe_members(&mut self) -> Result<(), Throw> {
        let prototype = self.prototype(Interface::HTMLIFrameElement)
            .ok_or_else(|| Throw::type_error("HTMLIFrameElement prototype missing"))?;
        let getter = self.native("get contentDocument", 0,
            NativeEntry::IFrameContentDocument, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "contentDocument", Some(getter), None, true, true)?;
        let src_getter = self.native("get src", 0, NativeEntry::IFrameSrcGet, HostCallPolicy::CallOnly);
        let src_setter = self.native("set src", 1, NativeEntry::IFrameSrcSet, HostCallPolicy::CallOnly);
        self.define_accessor_property(&prototype, "src", Some(src_getter), Some(src_setter), true, true)
    }

    fn iframe_node(&self, receiver: &JsValue) -> Result<NodeIdx, Throw> {
        let key = self.platform_object(receiver, InterfaceBrand::NODE)
            .ok_or_else(|| Throw::type_error("Invalid HTMLIFrameElement receiver"))?;
        let node = self.node_from_platform_key(key)
            .ok_or_else(|| Throw::type_error("Stale HTMLIFrameElement receiver"))?;
        let host = self.state.borrow().host.clone();
        let host = host.borrow();
        if host.dom.namespace(node) != Some(Namespace::Html)
            || host.dom.local_name(node) != Some("iframe") {
            return Err(Throw::type_error("Invalid HTMLIFrameElement receiver"));
        }
        Ok(node)
    }

    pub(super) fn iframe_src_get(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        let source = state.host.borrow().dom.get_attribute_ns(node, None, "src").map(str::to_owned);
        let Some(source) = source else { return Ok(JsValue::str("")) };
        let resolved = state.document_url.as_deref()
            .and_then(|base| ParsedUrl::parse(base).ok())
            .and_then(|base| base.resolve(&source).ok())
            .map_or(source, |url| url.serialize());
        Ok(JsValue::str(&resolved))
    }

    pub(super) fn iframe_src_set(&mut self, receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        self.iframe_node(&receiver)?;
        let value = args.first().cloned().unwrap_or(JsValue::Undefined);
        self.invoke_operation_with_html_hooks(Operation::ElementSetAttribute, receiver,
            &[JsValue::str("src"), value])
    }

    pub(super) fn iframe_content_document(&mut self, receiver: JsValue) -> Result<JsValue, Throw> {
        let node = self.iframe_node(&receiver)?;
        let state = self.state.borrow();
        let host = state.host.borrow();
        if host.dom.root(node) != NodeIdx(0) { return Ok(JsValue::Null); }
        let Some(frame) = state.frames.documents.get(&node) else { return Ok(JsValue::Null) };
        let Some(document) = frame.document else { return Ok(JsValue::Null) };
        let visible = frame.url == "about:blank" || state.document_url.as_deref()
            .is_some_and(|parent| same_origin(parent, &frame.url));
        drop(host);
        drop(state);
        if visible { self.wrap_node(document) } else { Ok(JsValue::Null) }
    }

    pub(super) fn invoke_operation_with_html_hooks(&mut self, operation: Operation,
        receiver: JsValue, args: &[JsValue]) -> Result<JsValue, Throw> {
        let result = self.invoke_operation_with_canvas_reset(operation, receiver, args);
        if result.is_ok() && matches!(operation,
            Operation::Append | Operation::Prepend | Operation::ReplaceChildren
            | Operation::Before | Operation::After | Operation::ReplaceWith | Operation::Remove
            | Operation::AppendChild | Operation::InsertBefore | Operation::RemoveChild
            | Operation::ReplaceChild | Operation::ElementSetInnerHTML
            | Operation::ElementInsertAdjacentHTML | Operation::ElementSetAttribute
            | Operation::ElementSetAttributeNS | Operation::ElementRemoveAttribute
            | Operation::ElementRemoveAttributeNS | Operation::ElementSetAttributeNode
            | Operation::ElementSetAttributeNodeNS | Operation::ElementRemoveAttributeNode) {
            RuntimeBindings { state: self.state.clone() }.navigate_iframes();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{dom_host::BrowserDomHost, js_scheduler::{DriverScheduler, JS_CAPACITY}};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{Shutdown, TcpListener};

    fn serve_xml_once(extra_header: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/child.xml", listener.local_addr().unwrap());
        let extra_header = extra_header.to_owned();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                if request.read_line(&mut line).unwrap() == 0 || line == "\r\n" { break; }
            }
            let body = "<?xml version='1.0'?><root>network child</root>";
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                body.len(), extra_header, body).unwrap();
            stream.flush().unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let _ = stream.read_to_end(&mut Vec::new());
        });
        (url, worker)
    }

    #[test]
    fn iframe_network_navigation_applies_embedding_policy() {
        for (header, visible) in [("", true), ("X-Frame-Options: DENY\r\n", false)] {
            let (url, worker) = serve_xml_once(header);
            let host = Rc::new(RefCell::new(BrowserDomHost {
                dom: amazing_html::parse(&format!("<body><iframe src='{url}'></iframe></body>")),
                native_forwarded: false,
            }));
            let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
            scheduler.set_location_url(url.replace("/child.xml", "/parent.html"));
            let result = scheduler.run_script(
                "var frame = document.querySelector('iframe'); \
                 frame.contentDocument !== null && \
                 frame.contentDocument.documentElement.textContent === 'network child'")
                .unwrap();
            assert_eq!(result, JsValue::Bool(visible));
            worker.join().unwrap();
        }
    }

    #[test]
    fn iframe_loads_xml_document_before_its_element_load_event() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()));
        std::fs::write(&path, "<?xml version='1.0'?><root>child text</root>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe src='/child.xml'></iframe></body>"),
            native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        let navigations = Rc::new(RefCell::new(Vec::<ChildNavigation>::new()));
        let observed = navigations.clone();
        scheduler.set_child_navigation_hook(Rc::new(move |navigation| {
            observed.borrow_mut().push(navigation);
        }));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); var order = []; \
             frame.contentDocument.addEventListener('load', () => order.push('child')); \
             frame.addEventListener('load', () => order.push('frame')); \
             frame.contentDocument.contentType + '|' + \
             frame.contentDocument.documentElement.textContent"
        ).unwrap(), JsValue::str("application/xml|child text"));
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("order.join(',')").unwrap(), JsValue::str("child,frame"));
        assert_eq!(navigations.borrow().len(), 1);
        assert_eq!(navigations.borrow()[0].committed_url, "https://example.test/child.xml");
        std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-{}.xml", std::process::id()))).unwrap();
    }

    #[test]
    fn iframe_content_document_enforces_same_origin_after_src_change() {
        let path = std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()));
        std::fs::write(&path, "<root/>").unwrap();
        let host = Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse("<body><iframe></iframe></body>"), native_forwarded: false,
        }));
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_frame_resource_resolver(Rc::new(move |_| Some(path.to_string_lossy().into_owned())));
        scheduler.set_location_url("https://example.test/main.html");
        assert_eq!(scheduler.run_script(
            "var frame = document.querySelector('iframe'); \
             frame.contentDocument !== null && frame.contentDocument.URL === 'about:blank'"
        ).unwrap(), JsValue::Bool(true));
        assert_eq!(scheduler.run_script(
            "frame.src = 'https://other.test/child.xml'; \
             frame.getAttribute('src') === 'https://other.test/child.xml' && \
             frame.contentDocument === null"
        ).unwrap(), JsValue::Bool(true));
        std::fs::remove_file(std::env::temp_dir().join(format!("amazing-iframe-cross-{}.xml", std::process::id()))).unwrap();
    }
}
