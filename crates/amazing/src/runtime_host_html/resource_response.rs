//! Canonical response transport for actual frame documents and classic scripts.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HtmlFrameResourceKind { Document, ClassicScript }

/// All policy values are captured before entering the embedding transport.
/// A navigation's initiating Document can differ from its embedding parent.
#[derive(Clone)]
pub(crate) struct HtmlFrameResourceRequest {
    pub(crate) requested_url: String,
    pub(crate) initiator: HtmlDocumentContext,
    /// Actual initiating policy, retained after its Document is retired.
    pub(crate) client_context: Option<amazing_fetch::service::ClientContext>,
    pub(crate) kind: HtmlFrameResourceKind,
    pub(crate) nonce: String,
    pub(crate) integrity: String,
    pub(crate) embedding: Option<NavigableContext>,
    pub(crate) request_mode: amazing_fetch::RequestMode,
    pub(crate) credentials: amazing_fetch::CredentialsMode,
    pub(crate) referrer: String,
    pub(crate) referrer_policy: amazing_fetch::ReferrerPolicy,
}

/// Returns the actual final URL, response headers and decoded transfer bytes.
/// Errors remain errors; they never select the legacy pinned resolver.
pub(crate) type HtmlFrameResourceLoader =
    Rc<dyn Fn(&HtmlFrameResourceRequest) -> Result<amazing_net::Response, String>>;

impl RuntimeBindings {
    pub(crate) fn set_frame_resource_loader(&self, loader: HtmlFrameResourceLoader) {
        self.state.borrow_mut().frames.resource_loader = Some(loader);
    }

    /// Root facade parser entry shares the same actual-document fetch gateway
    /// as child and reopened-document scripts.
    pub(crate) fn load_classic_page_script(&self, js: &mut Interpreter,
        script: &crate::cmd::js::PageScript, base: &str) -> Result<(String, String), Throw> {
        if script.kind != crate::cmd::js::ScriptKind::Classic {
            return Err(Throw::type_error("Expected a classic page script"));
        }
        let (document, resolver) = {
            let state = self.state.borrow();
            let host = state.host.borrow();
            if script.node_generation.is_none() || host.dom.node_generation(script.node) != script.node_generation {
                return Err(Throw::type_error("Stale script node"));
            }
            let document = host.dom.owner_document(script.node)
                .filter(|document| host.dom.node_generation(*document).is_some())
                .ok_or_else(|| Throw::type_error("Script has no live owner Document"))?;
            (document, state.frames.resolver.clone())
        };
        let Some(src) = script.src.as_deref() else {
            return Ok((script.body.clone(), self.page_script_url(js, script, base)));
        };
        self.fetch_document_classic_script(js, document, script.node, src, base, resolver.as_ref())
            .ok_or_else(|| Throw::type_error("Classic script resource could not be loaded"))
    }

    pub(super) fn fetch_document_classic_script(&self, js: &mut Interpreter,
        document: NodeIdx, script: NodeIdx, src: &str,
        base: &str, resolver: Option<&FrameResourceResolver>) -> Option<(String, String)> {
        let snapshot = {
            let state = self.state.borrow();
            let loader = state.frames.resource_loader.clone();
            let host = state.host.borrow();
            let context = state.frames.document_context(&host.dom, document);
            let label = host.dom.document_metadata(document).ok().flatten()
                .map(|metadata| metadata.character_set.as_str())
                .or_else(|| (document == NodeIdx(0)).then_some(state.document_character_set.as_deref()).flatten());
            let script_encoding = (host.dom.namespace(script) == Some(Namespace::Html))
                .then(|| host.dom.get_attribute_ns(script, None, "charset"))
                .flatten().and_then(amazing_encoding::encoding_for_label);
            let encoding = script_encoding.or_else(|| label.and_then(amazing_encoding::encoding_for_label))
                .unwrap_or(amazing_encoding::Encoding::Utf8);
            let default = state.frames.document_referrer_policy(&host.dom, document)
                .unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin);
            let (mode, credentials, referrer_policy) = classic_options(&host.dom, script, default);
            (loader, context, encoding, mode, credentials, referrer_policy,
                host.dom.get_attribute_ns(script, None, "nonce").unwrap_or("").to_owned(),
                host.dom.get_attribute_ns(script, None, "integrity").unwrap_or("").to_owned())
        };
        let Some(loader) = snapshot.0 else {
            let url = ParsedUrl::parse(base).ok()?.resolve(src).ok()?.serialize();
            return fetch_child_script(src, base, resolver).map(|text| (text, url));
        };
        let url = {
            let mut adapter = RuntimeBindingHost { js, state: self.state.clone() };
            let units = src.encode_utf16().collect::<Vec<_>>();
            String::from_utf16_lossy(&adapter.dom_resolve_element_url(script, &units).ok()?)
        };
        let initiator = snapshot.1?;
        let client_context = RuntimeBindingHost { js, state: self.state.clone() }
            .document_network_policy(initiator.document, initiator.document_generation);
        let request = HtmlFrameResourceRequest { requested_url: url,
            referrer: initiator.document_url.clone(), initiator, client_context,
            kind: HtmlFrameResourceKind::ClassicScript, embedding: None,
            nonce: snapshot.6, integrity: snapshot.7,
            request_mode: snapshot.3, credentials: snapshot.4, referrer_policy: snapshot.5 };
        let response = loader(&request).ok()?;
        decode_classic_response(response, snapshot.2).ok()
    }
}

fn classic_options(dom: &Dom, script: NodeIdx, default: amazing_fetch::ReferrerPolicy) ->
    (amazing_fetch::RequestMode, amazing_fetch::CredentialsMode, amazing_fetch::ReferrerPolicy) {
    use amazing_fetch::{CredentialsMode, RequestMode};
    let (mode, credentials) = match dom.get_attribute_ns(script, None, "crossorigin") {
        None => (RequestMode::NoCors, CredentialsMode::Include),
        Some(value) if value.eq_ignore_ascii_case("use-credentials") => (RequestMode::Cors, CredentialsMode::Include),
        Some(_) => (RequestMode::Cors, CredentialsMode::SameOrigin),
    };
    // Enumerated content attributes are ASCII case-insensitive; invalid and
    // empty states use the engine's document default policy.
    let policy = attribute_referrer_policy(dom.get_attribute_ns(script, None, "referrerpolicy"))
        .unwrap_or(default);
    (mode, credentials, policy)
}

pub(super) fn attribute_referrer_policy(value: Option<&str>) -> Option<amazing_fetch::ReferrerPolicy> {
    let value = value?.to_ascii_lowercase();
    if !matches!(value.as_str(), "no-referrer" | "no-referrer-when-downgrade" | "same-origin"
        | "origin" | "strict-origin" | "origin-when-cross-origin"
        | "strict-origin-when-cross-origin" | "unsafe-url") { return None; }
    amazing_fetch::ReferrerPolicy::parse_header(&value)
}

pub(super) fn response_referrer_policy(headers: &amazing_net::Headers) -> amazing_fetch::ReferrerPolicy {
    headers.get_all("referrer-policy").into_iter()
        .filter_map(amazing_fetch::ReferrerPolicy::parse_header).last()
        .unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin)
}

pub(super) fn referrer_policy_name(policy: amazing_fetch::ReferrerPolicy) -> &'static str {
    use amazing_fetch::ReferrerPolicy::*;
    match policy {
        NoReferrer => "no-referrer", NoReferrerWhenDowngrade => "no-referrer-when-downgrade",
        SameOrigin => "same-origin", Origin => "origin", StrictOrigin => "strict-origin",
        OriginWhenCrossOrigin => "origin-when-cross-origin",
        StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin", UnsafeUrl => "unsafe-url",
    }
}

impl FrameRuntime {
    pub(in crate::runtime_binding_host) fn document_referrer_policy(&self, dom: &Dom, document: NodeIdx)
        -> Option<amazing_fetch::ReferrerPolicy> {
        if !matches!(dom.node_generation(document).map(|_| &dom.get(document).kind), Some(NodeKind::Document)) { return None; }
        let generation = dom.node_generation(document)?;
        Some(self.document_referrer_policies.get(&document).filter(|(saved, _)| *saved == generation)
            .map(|(_, policy)| *policy).unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin))
    }
}

impl RuntimeBindingHost<'_> {
    pub(in crate::runtime_binding_host) fn html_document_referrer_policy(&self, document: NodeIdx)
        -> Option<amazing_fetch::ReferrerPolicy> {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.document_referrer_policy(&host.dom, document)
    }
    pub(in crate::runtime_binding_host) fn html_set_document_referrer_policy(&self, document: NodeIdx,
        policy: amazing_fetch::ReferrerPolicy) -> Result<(), amazing_dom::DomError> {
        let generation = {
            let state = self.state.borrow(); let host = state.host.borrow();
            if !matches!(host.dom.node_generation(document).map(|_| &host.dom.get(document).kind), Some(NodeKind::Document)) {
                return Err(amazing_dom::DomError::InvalidStateError);
            }
            host.dom.node_generation(document).ok_or(amazing_dom::DomError::InvalidStateError)?
        };
        self.state.borrow_mut().frames.document_referrer_policies.insert(document, (generation, policy));
        Ok(())
    }
}

impl RuntimeBindings {
    pub(crate) fn document_referrer_policy(&self, document: NodeIdx)
        -> Option<amazing_fetch::ReferrerPolicy> {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.document_referrer_policy(&host.dom, document)
    }

    pub(crate) fn set_document_response_referrer_policy(&self, document: NodeIdx,
        headers: &amazing_net::Headers) -> Result<(), amazing_dom::DomError> {
        let policy = headers.get_all("referrer-policy").into_iter()
            .filter_map(amazing_fetch::ReferrerPolicy::parse_header).last();
        let Some(policy) = policy else { return Ok(()); };
        let generation = {
            let state = self.state.borrow(); let host = state.host.borrow();
            if !matches!(host.dom.node_generation(document).map(|_| &host.dom.get(document).kind), Some(NodeKind::Document)) {
                return Err(amazing_dom::DomError::InvalidStateError);
            }
            host.dom.node_generation(document).ok_or(amazing_dom::DomError::InvalidStateError)?
        };
        self.state.borrow_mut().frames.document_referrer_policies.insert(document, (generation, policy));
        Ok(())
    }
}

/// Preserve field order and duplicate values for Fetch header algorithms.
fn response_header_list(response: &amazing_net::Response) -> amazing_fetch::HeaderList {
    let mut headers = amazing_fetch::HeaderList::default();
    for (name, value) in response.headers.iter() {
        // Net Headers already validates names and field values.
        headers.append(name, value).expect("validated response header");
    }
    headers
}

/// Fetch §3.5 selects the last valid MIME record and carries charset only
/// across records with the same essence. Quoted lists/parameters stay intact.
pub(super) fn response_mime_type(response: &amazing_net::Response) -> Option<amazing_fetch::mime::MimeType> {
    amazing_fetch::mime::extract_mime_type(&response_header_list(response))
}

pub(super) fn response_nosniff(response: &amazing_net::Response) -> bool {
    amazing_fetch::determine_nosniff(&response_header_list(response))
}

/// HTML fetch-a-classic-script plus Encoding decode. No HTML meta prescan:
/// header charset overrides fallback, and a real BOM overrides either one.
fn decode_classic_response(response: amazing_net::Response,
    fallback: amazing_encoding::Encoding) -> Result<(String, String), String> {
    if !response.is_success() { return Err(format!("classic script status {}", response.status)); }
    if response.body.len() > 16 * 1024 * 1024 { return Err("classic script body too large".into()); }
    ParsedUrl::parse(&response.url).map_err(|error| format!("classic script final URL: {error:?}"))?;
    let record = response_mime_type(&response);
    let mime = record.as_ref().map(|mime| mime.essence()).unwrap_or_default();
    let javascript = amazing_html::is_javascript_mime_type(&mime);
    let nosniff = response_nosniff(&response);
    if (!javascript && nosniff) || mime.starts_with("image/") || mime.starts_with("audio/")
        || mime.starts_with("video/") || mime == "text/csv" {
        return Err(format!("blocked classic script media type: {mime}"));
    }
    let encoding = record.as_ref().and_then(|mime| mime.parameter("charset"))
        .and_then(amazing_encoding::encoding_for_label).unwrap_or(fallback);
    let (encoding, skip) = amazing_encoding::sniff_bom(&response.body).unwrap_or((encoding, 0));
    let text = amazing_encoding::decode(&response.body[skip..], encoding, false)
        .map_err(|error| format!("classic script decode: {error:?}"))?;
    Ok((text, response.url))
}

pub(super) fn load_document_response(request: &HtmlFrameResourceRequest,
    loader: &HtmlFrameResourceLoader) -> Result<amazing_net::Response, String> {
    let embedding = request.embedding.as_ref()
        .ok_or_else(|| "frame document has no embedding context".to_owned())?;
    let target = HttpUrl::parse(&request.requested_url)
        .map_err(|error| format!("iframe URL: {error:?}"))?;
    ApiRequest::child_navigation(target, embedding)
        .map_err(|error| format!("iframe request: {error:?}"))?;
    // The caller clones both the callback and all policy values before here.
    let response = loader(request)?;
    let final_url = HttpUrl::parse(&response.url)
        .map_err(|error| format!("iframe final URL: {error:?}"))?;
    if response.body.len() > 16 * 1024 * 1024 { return Err("iframe body too large".into()); }
    if !response.is_success() { return Err(format!("iframe status {}", response.status)); }
    let mut headers = HeaderList::default();
    for (name, value) in response.headers.iter() {
        headers.append(name, value).map_err(|error| format!("iframe header: {error:?}"))?;
    }
    check_frame_embedding(&final_url, &headers, &embedding.ancestor_origins)
        .map_err(|error| format!("iframe embedding: {error}"))?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_scheduler::{DriverScheduler, JS_CAPACITY};

    fn response(url: &str, mime: Option<&str>, body: Vec<u8>) -> amazing_net::Response {
        let mut headers = amazing_net::Headers::new();
        if let Some(mime) = mime { headers.append("Content-Type", mime.to_owned()).unwrap(); }
        amazing_net::Response { url: url.into(), status: 200, reason: "OK".into(), headers, body }
    }

    #[test]
    fn frame_encoding_inherits_actual_same_origin_container_after_explicit_choices() {
        for (parent, meta, header, sandbox, child_url, bom, expected, encoding) in [
            ("UTF-8", "bogus", None, "", "https://example.test/child", false, "�", "UTF-8"),
            ("windows-1253", "bogus", Some("bogus"), "", "https://example.test/child", false, "Ά", "windows-1253"),
            ("UTF-16LE", "bogus", None, "", "https://example.test/child", false, "¢", "windows-1252"),
            ("windows-1253", "windows-1252", None, "", "https://example.test/child", false, "¢", "windows-1252"),
            ("windows-1253", "windows-1252", Some("UTF-8"), "", "https://example.test/child", false, "�", "UTF-8"),
            ("windows-1253", "bogus", None, "sandbox='allow-scripts'", "https://example.test/child", false, "¢", "windows-1252"),
            ("windows-1253", "bogus", None, "sandbox='allow-same-origin'", "https://example.test/child", false, "Ά", "windows-1253"),
            ("windows-1253", "bogus", None, "", "https://foreign.test/child", false, "¢", "windows-1252"),
            ("windows-1253", "bogus", Some("windows-1252"), "", "https://example.test/child", true, "é", "UTF-8"),
        ] {
            let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(&format!(
                "<!doctype html><body><iframe {sandbox} src='/child'></iframe>")), native_forwarded: false }));
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
            scheduler.set_frame_resource_loader(Rc::new(move |_| {
                let mut body = format!("<!doctype html><meta charset={meta}><p>").into_bytes();
                if bom { body.splice(0..0, [0xef,0xbb,0xbf]); body.extend([0xc3,0xa9]); }
                else { body.push(0xa2); }
                let mut value = response(child_url, Some("text/html"), body);
                if let Some(label) = header {
                    value.headers.append("Content-Type", format!("text/html; charset={label}")).unwrap();
                }
                Ok(value)
            }));
            // Set the actual parent's selected metadata before first navigation.
            scheduler.set_document_metadata(parent, "text/html", "");
            scheduler.set_location_url("https://example.test/root.html");
            scheduler.dispatch_pending_iframe_loads().unwrap();
            let bindings = scheduler.test_bindings();
            let state = bindings.state.borrow();
            let document = state.frames.documents.values().find_map(|entry| entry.document).unwrap();
            let host = host.borrow();
            assert_eq!(host.dom.document_metadata(document).unwrap().unwrap().character_set, encoding,
                "parent={parent} meta={meta} header={header:?} sandbox={sandbox} final={child_url}");
            let text = host.dom.tree_order(document).into_iter().filter_map(|node| host.dom.character_data(node)).collect::<String>();
            assert_eq!(text, expected);
        }
    }

    #[test]
    fn response_mime_lists_quotes_charset_and_nosniff_use_fetch_records() {
        use amazing_encoding::Encoding::{Utf8, Windows1252};
        let url = "https://example.test/final/resource";
        for (fields, essence, charset) in [
            (vec!["text/html; charset=windows-1252", "text/html"], "text/html", Some("windows-1252")),
            (vec!["text/html; charset=windows-1252", "text/html; charset=UTF-8", "text/html"], "text/html", Some("windows-1252")),
            (vec!["text/html; charset=windows-1252", "application/xhtml+xml"], "application/xhtml+xml", None),
            (vec!["bogus", "*/*", "TEXT/HTML; x=\"a,b;c\"; charset=\"windows\\-1252\"", "invalid"], "text/html", Some("windows-1252")),
            (vec!["application/xml, text/html; charset=windows-1252"], "text/html", Some("windows-1252")),
        ] {
            let mut value = response(url, None, b"<!doctype html><p>test".to_vec());
            for field in fields { value.headers.append("Content-Type", field.to_owned()).unwrap(); }
            let mime = response_mime_type(&value).unwrap();
            assert_eq!(mime.essence(), essence);
            assert_eq!(mime.parameter("charset"), charset);
            assert_eq!(content_type(&value, url, false).as_deref(), Some(essence));
        }
        for (fields, allowed, expected) in [
            (vec!["image/png", "text/javascript; charset=\"windows\\-1252\""], true, "é"),
            (vec!["text/javascript", "image/png"], false, ""),
            (vec!["text/plain", "invalid", "TEXT/JAVASCRIPT"], true, "�"),
            (vec!["text/javascript; charset=windows-1252", "text/javascript"], true, "é"),
            (vec!["text/javascript; charset=windows-1252", "application/javascript"], true, "�"),
        ] {
            let mut value = response(url, None, vec![0xe9]);
            for field in fields { value.headers.append("Content-Type", field.to_owned()).unwrap(); }
            value.headers.append("X-Content-Type-Options", "NOSNIFF, ignored".to_owned()).unwrap();
            let result = decode_classic_response(value, Utf8);
            assert_eq!(result.is_ok(), allowed);
            if allowed { assert_eq!(result.unwrap().0, expected); }
        }
        // Missing/invalid MIME remains unknown; no HTTP extension inference.
        let mut value = response(url, Some("invalid, */*"), b"<!doctype html><p>html".to_vec());
        assert!(response_mime_type(&value).is_none());
        assert_eq!(content_type(&value, "https://example.test/r.xml", false).as_deref(), Some("text/html"));
        value.headers.append("X-Content-Type-Options", "invalid".to_owned()).unwrap();
        value.headers.append("X-Content-Type-Options", "nosniff".to_owned()).unwrap();
        assert!(!response_nosniff(&value));
        assert_eq!(content_type(&value, url, false).as_deref(), Some("text/html"));
        let mut value = response(url, Some("text/javascript; charset=windows-1252"), vec![0xef,0xbb,0xbf,0xc3,0xa9]);
        value.headers.append("Content-Type", "text/javascript".to_owned()).unwrap();
        assert_eq!(decode_classic_response(value, Windows1252).unwrap().0, "é");
    }

    #[test]
    fn actual_frame_document_and_script_decode_selected_duplicate_response_mime() {
        for bom in [false, true] {
            let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
                "<!doctype html><body><iframe src='/child.xml'></iframe>"), native_forwarded: false }));
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
            scheduler.set_frame_resource_loader(Rc::new(move |request| {
                let mut value = match request.kind {
                    HtmlFrameResourceKind::Document => response("https://example.test/final/child.xml", Some("application/xml; charset=UTF-8"),
                        if bom { b"\xef\xbb\xbf<!doctype html><p id=p>\xc3\xa9</p><script src='child.js'></script>".to_vec() }
                        else { b"<!doctype html><p id=p>\xe9</p><script src='child.js'></script>".to_vec() }),
                    HtmlFrameResourceKind::ClassicScript => response("https://example.test/final/child.js", Some("image/png"),
                        b"window.mimeValue='\xe9';".to_vec()),
                };
                value.headers.append("Content-Type", match request.kind {
                    HtmlFrameResourceKind::Document => "text/html; x=\"a,b;c\"; charset=\"windows\\-1252\"",
                    HtmlFrameResourceKind::ClassicScript => "text/javascript; charset=windows-1252",
                }.to_owned()).unwrap();
                if request.kind == HtmlFrameResourceKind::ClassicScript {
                    value.headers.append("X-Content-Type-Options", "nosniff".to_owned()).unwrap();
                }
                Ok(value)
            }));
            scheduler.set_location_url("https://example.test/root.html");
            scheduler.dispatch_pending_iframe_loads().unwrap();
            assert_eq!(scheduler.run_script("var f=document.querySelector('iframe');[f.contentDocument.contentType,f.contentDocument.getElementById('p').textContent,f.contentWindow.mimeValue].join('|')").unwrap(),
                JsValue::str("text/html|é|é"));
            assert!(host.try_borrow_mut().is_ok());
        }
    }

    #[test]
    fn root_classic_bridge_captures_actual_metadata_before_transport_and_executes_decoded_source() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><base href='/assets/'><script src='root.js?q=é' nonce=' actual ' integrity=' sha256-test ' crossorigin=use-credentials referrerpolicy=ORIGIN></script>"), native_forwarded: false }));
        let node = host.borrow().dom.tree_order(NodeIdx(0)).into_iter()
            .find(|node| host.borrow().dom.local_name(*node) == Some("script")).unwrap();
        let script = crate::cmd::js::script_source_from_node(&host.borrow().dom, node).unwrap();
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root/main.html");
        scheduler.set_document_metadata("windows-1252", "text/html", "");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observed = seen.clone();
        let callback_host = host.clone();
        scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("canonical transport selected legacy resolver")));
        scheduler.set_frame_resource_loader(Rc::new(move |request| {
            observed.borrow_mut().push(request.clone());
            let mut host = callback_host.try_borrow_mut().expect("callback retained a DOM borrow");
            host.dom.set_attribute(node, "nonce", "changed").unwrap();
            host.dom.set_attribute(node, "integrity", "changed").unwrap();
            Ok(response("https://example.test/final/root.js", Some("text/javascript; charset=windows-1252"),
                b"window.rootBridge='\xe9';".to_vec()))
        }));
        let (text, url) = scheduler.load_classic_page_script(&script, "https://example.test/wrong/caller.html").unwrap();
        assert_eq!(text, "window.rootBridge='é';");
        assert_eq!(url, "https://example.test/final/root.js");
        let requests = seen.borrow();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.kind, HtmlFrameResourceKind::ClassicScript);
        assert_eq!(request.requested_url, "https://example.test/assets/root.js?q=%E9");
        assert_eq!(request.nonce, " actual ");
        assert_eq!(request.integrity, " sha256-test ");
        assert_eq!(request.initiator.document, NodeIdx(0));
        assert_eq!(Some(request.initiator.document_generation), host.borrow().dom.node_generation(NodeIdx(0)));
        assert_eq!(request.referrer, "https://example.test/root/main.html");
        assert_eq!(request.referrer_policy, amazing_fetch::ReferrerPolicy::Origin);
        assert_eq!(request.request_mode, amazing_fetch::RequestMode::Cors);
        assert_eq!(request.credentials, amazing_fetch::CredentialsMode::Include);
        drop(requests);
        scheduler.run_page_script(&script, &text, &url).unwrap();
        assert_eq!(scheduler.run_script("rootBridge").unwrap(), JsValue::str("é"));
    }

    #[test]
    fn root_classic_bridge_preserves_inline_and_genuine_owner_failure_boundaries() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><script>window.inlineBridge=1;</script>"), native_forwarded: false }));
        let node = host.borrow().dom.tree_order(NodeIdx(0)).into_iter()
            .find(|node| host.borrow().dom.local_name(*node) == Some("script")).unwrap();
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/root/main.html");
        scheduler.set_frame_resource_loader(Rc::new(|_| Err("genuine transport failure".into())));
        scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("canonical error selected legacy resolver")));
        let mut script = crate::cmd::js::script_source_from_node(&host.borrow().dom, node).unwrap();
        let (body, url) = scheduler.load_classic_page_script(&script, "https://example.test/root/main.html").unwrap();
        assert_eq!(body, "window.inlineBridge=1;");
        assert_eq!(url, "https://example.test/root/main.html");
        script.kind = crate::cmd::js::ScriptKind::Module;
        assert!(scheduler.load_classic_page_script(&script, &url).is_err());
        script.kind = crate::cmd::js::ScriptKind::Classic;
        script.src = Some("failure.js".into());
        assert!(scheduler.load_classic_page_script(&script, &url).is_err());
        script.node = NodeIdx(u32::MAX);
        assert!(scheduler.load_classic_page_script(&script, &url).is_err());
        script.node = NodeIdx(0);
        assert!(scheduler.load_classic_page_script(&script, &url).is_err());
    }

    #[test]
    fn root_classic_bridge_rejects_same_slot_with_a_different_captured_generation() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><script src='live.js'></script>"), native_forwarded: false }));
        let node = host.borrow().dom.tree_order(NodeIdx(0)).into_iter()
            .find(|node| host.borrow().dom.local_name(*node) == Some("script")).unwrap();
        let mut script = crate::cmd::js::script_source_from_node(&host.borrow().dom, node).unwrap();
        let captured = script.node_generation.unwrap();
        let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/page.html");
        let calls = Rc::new(RefCell::new(0));
        let recorded = calls.clone();
        scheduler.set_frame_resource_loader(Rc::new(move |request| {
            *recorded.borrow_mut() += 1;
            Ok(response(&request.requested_url, Some("text/javascript"), b"window.actualGeneration=1;".to_vec()))
        }));
        // A stale generation from the same slot must never fetch the replacement's attributes.
        script.node_generation = Some(captured.wrapping_add(1));
        assert!(scheduler.load_classic_page_script(&script, "https://example.test/page.html").is_err());
        script.src = None;
        assert!(scheduler.load_classic_page_script(&script, "https://example.test/page.html").is_err());
        script.node_generation = None;
        assert!(scheduler.load_classic_page_script(&script, "https://example.test/page.html").is_err());
        assert_eq!(*calls.borrow(), 0);
        script.node_generation = Some(captured);
        script.src = Some("live.js".into());
        assert!(scheduler.load_classic_page_script(&script, "https://example.test/page.html").is_ok());
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn classic_options_capture_actual_enumerated_attributes_without_header_token_rules() {
        use amazing_fetch::{CredentialsMode, ReferrerPolicy, RequestMode};
        for (attribute, mode, credentials) in [
            ("", RequestMode::NoCors, CredentialsMode::Include),
            ("crossorigin", RequestMode::Cors, CredentialsMode::SameOrigin),
            ("crossorigin=anonymous", RequestMode::Cors, CredentialsMode::SameOrigin),
            ("crossorigin=USE-CREDENTIALS", RequestMode::Cors, CredentialsMode::Include),
            ("crossorigin=' use-credentials '", RequestMode::Cors, CredentialsMode::SameOrigin),
            ("crossorigin=bogus", RequestMode::Cors, CredentialsMode::SameOrigin),
        ] {
            for (value, expected) in [
                ("NO-REFERRER", ReferrerPolicy::NoReferrer),
                ("origin", ReferrerPolicy::Origin),
                ("", ReferrerPolicy::StrictOriginWhenCrossOrigin),
                ("bogus,origin", ReferrerPolicy::StrictOriginWhenCrossOrigin),
                (" origin ", ReferrerPolicy::StrictOriginWhenCrossOrigin),
            ] {
                let dom = amazing_html::parse(&format!("<script {attribute} referrerpolicy='{value}'></script>"));
                let script = dom.tree_order(NodeIdx(0)).into_iter()
                    .find(|node| dom.local_name(*node) == Some("script")).unwrap();
                assert_eq!(classic_options(&dom, script, ReferrerPolicy::StrictOriginWhenCrossOrigin), (mode, credentials, expected));
            }
        }
    }

    #[test]
    fn referrer_policy_headers_attributes_and_generation_keep_actual_document_state() {
        use amazing_fetch::ReferrerPolicy::*;
        let mut headers = amazing_net::Headers::new();
        headers.append("Referrer-Policy", "origin, invalid".to_owned()).unwrap();
        headers.append("Referrer-Policy", "no-referrer, same-origin".to_owned()).unwrap();
        headers.append("Referrer-Policy", "INVALID".to_owned()).unwrap();
        assert_eq!(response_referrer_policy(&headers), SameOrigin);
        for value in ["", " origin ", "origin,no-referrer", "bogus"] {
            assert_eq!(attribute_referrer_policy(Some(value)), None);
        }
        assert_eq!(attribute_referrer_policy(Some("NO-REFERRER")), Some(NoReferrer));
        let dom = amazing_html::parse("<script referrerpolicy='bogus'></script>");
        let script = dom.tree_order(NodeIdx(0)).into_iter().find(|node| dom.local_name(*node)==Some("script")).unwrap();
        assert_eq!(classic_options(&dom, script, NoReferrer).2, NoReferrer);
        let mut frames = FrameRuntime::default();
        let generation = dom.node_generation(NodeIdx(0)).unwrap();
        frames.document_referrer_policies.insert(NodeIdx(0), (generation, Origin));
        assert_eq!(frames.document_referrer_policy(&dom, NodeIdx(0)), Some(Origin));
        frames.document_referrer_policies.insert(NodeIdx(0), (generation+1, NoReferrer));
        assert_eq!(frames.document_referrer_policy(&dom, NodeIdx(0)), Some(StrictOriginWhenCrossOrigin));
        assert_eq!(frames.document_referrer_policy(&dom, script), None);
    }

    #[test]
    fn classic_response_uses_bom_header_then_actual_fallback_and_media_policy() {
        use amazing_encoding::Encoding::{Utf8, Windows1252};
        let url = "https://example.test/redirect/script.js";
        for (header, fallback, bytes, expected) in [
            (Some("text/javascript; charset=windows-1252"), Utf8, vec![0xe9], "é"),
            (Some("text/javascript; charset=windows-1252"), Windows1252, vec![0xef,0xbb,0xbf,0xc3,0xa9], "é"),
            (Some("text/javascript; charset=bogus"), Windows1252, vec![0xe9], "é"),
            (None, Utf8, vec![0xe9], "�"),
        ] {
            let result = decode_classic_response(response(url, header, bytes), fallback).unwrap();
            assert_eq!(result, (expected.into(), url.into()));
        }
        let bytes = b"// <meta charset=utf-8>\nwindow.value='\xe9';".to_vec();
        assert!(decode_classic_response(response(url, None, bytes), Windows1252).unwrap().0.ends_with("'é';"));
        for mime in ["image/png", "audio/wav", "video/mp4", "text/csv"] {
            assert!(decode_classic_response(response(url, Some(mime), b"1".to_vec()), Utf8).is_err());
        }
        for mime in [None, Some("text/plain"), Some("TEXT/JAVASCRIPT")] {
            let mut value = response(url, mime, b"1".to_vec());
            value.headers.append("X-Content-Type-Options", "nosniff".to_owned()).unwrap();
            assert_eq!(decode_classic_response(value, Utf8).is_ok(), mime == Some("TEXT/JAVASCRIPT"));
        }
        let mut value = response(url, Some("text/javascript"), Vec::new());
        value.status = 404;
        assert!(decode_classic_response(value, Utf8).is_err());
        assert!(decode_classic_response(response("not a URL", None, Vec::new()), Utf8).is_err());
        assert!(decode_classic_response(response(url, None, vec![0;16*1024*1024+1]), Utf8).is_err());
    }

    #[test]
    fn canonical_document_and_classic_requests_keep_real_redirects_bytes_and_borrow_boundaries() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
            "<!doctype html><body><iframe src='/requested.xml'></iframe>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        let requests = Rc::new(RefCell::new(Vec::new()));
        let observed = requests.clone();
        let callback_host = host.clone();
        scheduler.set_frame_resource_loader(Rc::new(move |request| {
            assert!(callback_host.try_borrow_mut().is_ok(), "transport called with a live DOM borrow");
            observed.borrow_mut().push(request.clone());
            Ok(match request.kind {
                HtmlFrameResourceKind::Document => response("https://example.test/final/child.html",
                    Some("text/html; charset=windows-1252"),
                    b"<!doctype html><base href='/assets/'><body><script src='script.js?q=\xe9'></script><p id=p>\xe9</p>".to_vec()),
                HtmlFrameResourceKind::ClassicScript => response("https://example.test/scripts/final.js",
                    Some("text/javascript; charset=windows-1252"), b"window.classicValue='\xe9';".to_vec()),
            })
        }));
        scheduler.set_location_url("https://example.test/root/main.html");
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("var f=document.querySelector('iframe');[f.contentDocument.URL,f.contentDocument.characterSet,f.contentDocument.getElementById('p').textContent,f.contentWindow.classicValue].join('|')").unwrap(),
            JsValue::str("https://example.test/final/child.html|windows-1252|é|é"));
        let requests = requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].requested_url, "https://example.test/requested.xml");
        assert_eq!(requests[0].initiator.document_url, "https://example.test/root/main.html");
        assert!(requests[0].embedding.is_some());
        assert!(requests[0].nonce.is_empty() && requests[0].integrity.is_empty());
        assert_eq!(requests[1].requested_url, "https://example.test/assets/script.js?q=%E9");
        assert_eq!(requests[1].initiator.document_url, "https://example.test/final/child.html");
        assert_eq!(requests[1].referrer, requests[1].initiator.document_url);
        assert_eq!(requests[1].request_mode, amazing_fetch::RequestMode::NoCors);
        assert_eq!(requests[1].credentials, amazing_fetch::CredentialsMode::Include);
        for request in requests.iter() {
            assert_eq!(Some(request.initiator.document_generation),
                host.borrow().dom.node_generation(request.initiator.document));
        }
        assert_ne!(requests[1].initiator.document, requests[0].initiator.document);
        assert!(requests[1].embedding.is_none());
        assert_eq!(requests[1].initiator.ancestor_origins, vec![requests[0].initiator.origin.clone()]);
    }

    #[test]
    fn actual_creator_and_response_policies_are_installed_before_child_script_fetches() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><body>"), native_forwarded: false }));
        let mut js = Interpreter::new_with_external_scheduler(JS_CAPACITY).unwrap();
        js.set_dom_host(host.clone());
        let bindings = crate::runtime_binding_host::install(&mut js, host.clone()).unwrap();
        bindings.set_document_url(&mut js, "https://example.test/root.html");
        let mut creator = amazing_fetch::service::ClientContext::default();
        let mut headers = amazing_fetch::HeaderList::default();
        headers.append("Content-Security-Policy", "img-src 'none'").unwrap();
        creator.csp = amazing_fetch::CspPolicyList::from_headers(&headers).unwrap();
        RuntimeBindingHost { js: &mut js, state: bindings.state.clone() }
            .set_document_network_policy(NodeIdx(0), creator.clone()).unwrap();
        RuntimeBindingHost { js: &mut js, state: bindings.state.clone() }
            .html_set_document_referrer_policy(NodeIdx(0), amazing_fetch::ReferrerPolicy::NoReferrer).unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observed = seen.clone();
        bindings.set_frame_resource_loader(Rc::new(move |request| {
            observed.borrow_mut().push(request.clone());
            let mut response = response(&request.requested_url, Some("text/html"), b"<!doctype html><body>".to_vec());
            response.headers.append("Content-Security-Policy", "connect-src 'none'".to_owned()).unwrap();
            response.headers.append("Referrer-Policy", "origin, same-origin".to_owned()).unwrap();
            Ok(response)
        }));
        let frames = {
            let mut host = host.borrow_mut();
            let body = host.dom.tree_order(NodeIdx(0)).into_iter().find(|node| host.dom.local_name(*node) == Some("body")).unwrap();
            let blank = host.dom.create_element(body, "iframe");
            let srcdoc = host.dom.create_element(body, "iframe");
            host.dom.set_attribute(srcdoc, "srcdoc", "<p>child").unwrap();
            let remote = host.dom.create_element(body, "iframe");
            host.dom.set_attribute(remote, "src", "/remote").unwrap();
            [blank, srcdoc, remote]
        };
        bindings.navigate_iframes(&mut js);
        let documents = frames.map(|frame| bindings.state.borrow().frames.documents[&frame].document.unwrap());
        let mut adapter = RuntimeBindingHost { js: &mut js, state: bindings.state.clone() };
        for document in documents[..2].iter().copied() {
            let generation = host.borrow().dom.node_generation(document).unwrap();
            assert_eq!(adapter.document_network_policy(document, generation).unwrap().csp, creator.csp);
            assert_eq!(adapter.html_document_referrer_policy(document), Some(amazing_fetch::ReferrerPolicy::NoReferrer));
        }
        let remote_generation = host.borrow().dom.node_generation(documents[2]).unwrap();
        let remote = adapter.document_network_policy(documents[2], remote_generation).unwrap();
        assert_ne!(remote.csp, creator.csp);
        assert_eq!(adapter.html_document_referrer_policy(documents[2]), Some(amazing_fetch::ReferrerPolicy::SameOrigin));
        assert_eq!(seen.borrow()[0].referrer_policy, amazing_fetch::ReferrerPolicy::NoReferrer);
        assert_eq!(seen.borrow()[0].client_context.as_ref().unwrap().csp, creator.csp);
        // Replacing the creator's policy cannot change inherited snapshots.
        let queued = adapter.html_navigation_source("about:blank", false).unwrap();
        adapter.html_set_document_referrer_policy(NodeIdx(0), amazing_fetch::ReferrerPolicy::UnsafeUrl).unwrap();
        assert_eq!(queued.source.referrer_policy, amazing_fetch::ReferrerPolicy::NoReferrer);
        adapter.set_document_network_policy(NodeIdx(0), amazing_fetch::service::ClientContext::default()).unwrap();
        for document in documents[..2].iter().copied() {
            let generation = host.borrow().dom.node_generation(document).unwrap();
            assert_eq!(adapter.document_network_policy(document, generation).unwrap().csp, creator.csp);
            assert_eq!(adapter.html_document_referrer_policy(document), Some(amazing_fetch::ReferrerPolicy::NoReferrer));
        }
        drop(adapter);
        let script = host.borrow_mut().dom.create_element(documents[2], "script");
        bindings.fetch_document_classic_script(&mut js, documents[2], script, "/script.js",
            "https://example.test/remote", None).unwrap();
        assert_eq!(seen.borrow().last().unwrap().client_context.as_ref().unwrap().csp, remote.csp);
        assert_eq!(seen.borrow().last().unwrap().referrer_policy, amazing_fetch::ReferrerPolicy::SameOrigin);
        assert!(host.try_borrow_mut().is_ok());
    }

    #[test]
    fn reopened_document_external_classic_script_uses_real_response_and_owner_context() {
        let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse("<!doctype html><body>"), native_forwarded: false }));
        let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
        scheduler.set_location_url("https://example.test/active/index.html");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let observed = seen.clone();
        scheduler.set_frame_resource_loader(Rc::new(move |request| {
            observed.borrow_mut().push(request.clone());
            assert_eq!(request.kind, HtmlFrameResourceKind::ClassicScript);
            assert_eq!(request.initiator.document_url, "https://example.test/active/index.html");
            assert_eq!(request.requested_url, "https://example.test/active/stream.js");
            Ok(response("https://example.test/redirect/stream.js", Some("text/javascript; charset=windows-1252"),
                b"window.stream_loaded=(window.stream_loaded||0)+1;window.stream_text='\xe9';".to_vec()))
        }));
        scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("canonical written script used pinned resolver")));
        scheduler.run_script(r#"document.open();document.write('<script src="stream.js"><\/script>');document.close();"#)
            .unwrap_or_else(|error| panic!("canonical written script: {}", error.describe()));
        assert_eq!(scheduler.run_script("stream_loaded===1 && stream_text==='é'").unwrap(), JsValue::Bool(true));
        assert_eq!(seen.borrow().len(), 1);
        assert!(host.try_borrow_mut().is_ok());
    }

    #[test]
    fn canonical_document_errors_and_unknown_mime_do_not_use_pinned_extensions_or_fallback() {
        for case in 0..12 {
            let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
                "<!doctype html><body><iframe src='/requested.xml'></iframe>"), native_forwarded: false }));
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
            let calls = Rc::new(RefCell::new(0));
            let seen = calls.clone(); let callback_host = host.clone();
            scheduler.set_frame_resource_loader(Rc::new(move |request| {
                *seen.borrow_mut() += 1;
                assert_eq!(request.kind, HtmlFrameResourceKind::Document);
                assert!(callback_host.try_borrow_mut().is_ok());
                if case == 0 { return Err("actual transport error".into()); }
                let body = match case {
                    1 => b"<root/>".to_vec(),
                    7 => vec![0; 16 * 1024 * 1024 + 1],
                    8 => b"<?xml version='1.0'?><r>\xff</r>".to_vec(),
                    9 => b"<r/>".to_vec(),
                    11 => b" \r\n<?xml-stylesheet type='text/css'?><r>child</r>".to_vec(),
                    _ => b"<!doctype html><p>child</p>".to_vec(),
                };
                let mime = match case {
                    4 => Some("application/octet-stream"),
                    8 => Some("application/xml"),
                    9 => Some("application/xml; charset=unsupported"),
                    _ => None,
                };
                let mut value = response(if case == 6 { "not a URL" } else { "https://example.test/final.xml" }, mime, body);
                if case == 2 { value.headers.append("X-Content-Type-Options", "nosniff".into()).unwrap(); }
                if case == 3 { value.headers.append("X-Frame-Options", "DENY".into()).unwrap(); }
                if case == 5 { value.status = 500; }
                Ok(value)
            }));
            scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("canonical failure fell back to pinned transport")));
            // Configuration precedes the actual initial navigation, so failures
            // prove this response path rather than a cached legacy-network error.
            scheduler.set_location_url("https://example.test/root.html");
            scheduler.dispatch_pending_iframe_loads().unwrap();
            let expression = if case == 10 { "document.querySelector('iframe').contentDocument.contentType === 'text/html'" }
                else if case == 11 { "document.querySelector('iframe').contentDocument.contentType === 'text/xml' && document.querySelector('iframe').contentDocument.documentElement.textContent === 'child'" }
                else { "document.querySelector('iframe').contentDocument === null" };
            assert_eq!(scheduler.run_script(expression).unwrap_or_else(|error| panic!("case {case}: {}", error.describe())), JsValue::Bool(true), "case {case}");
            assert_eq!(*calls.borrow(), 1, "case {case} never reached canonical callback or navigated twice");
        }
    }
    #[test]
    fn unknown_document_html_xml_signatures_follow_actual_byte_rows() {
        for prefix in [b"<?xml".as_slice(), b" \t\n\r\x0c<?xml", b"<?xml-stylesheet"] {
            assert!(has_xml_signature(prefix));
            let value = response("https://example.test/r", None, prefix.to_vec());
            assert_eq!(content_type(&value, &value.url, false).as_deref(), Some("text/xml"));
        }
        for prefix in [b"<?XML".as_slice(), b"x<?xml", b"\0<?xml", b"\xef\xbb\xbf<?xml", b"<?xm"] { assert!(!has_xml_signature(prefix)); }
        for mime in ["text/html", "text/plain", "application/octet-stream", "application/json"] {
            let value = response("https://example.test/r.xml", Some(mime), b"<?xml version='1.0'?><r/>".to_vec());
            let expected = (mime != "application/octet-stream").then_some(mime);
            assert_eq!(content_type(&value, &value.url, false).as_deref(), expected);
        }
        let mut value = response("https://example.test/r.xml", None, b"<?xml version='1.0'?><r/>".to_vec());
        value.headers.append("X-Content-Type-Options", "nosniff".into()).unwrap();
        assert_eq!(content_type(&value, &value.url, false), None);
    }
    #[test]
    fn xml_response_bytes_use_xml_authorities_and_fatal_errors() {
        let (dom, label) = parse_response("<r>é</r>".as_bytes(), "application/xml", None).unwrap();
        assert_eq!(label, "UTF-8");
        assert!(dom.dump(NodeIdx(0)).contains("é"));
        let declared = b"<?xml version='1.0' encoding='windows-1253'?><r>\xa2</r>";
        let (dom, label) = parse_response(declared, "application/xhtml+xml", None).unwrap();
        assert_eq!(label, "windows-1253");
        assert!(dom.dump(NodeIdx(0)).contains('\u{0386}'));
        assert!(parse_response(declared, "text/xml", Some("UTF-8")).is_err());
        assert!(parse_response(b"<r><meta charset='windows-1252'/>\xe9</r>", "image/svg+xml", None).is_err());
        assert!(parse_response(b"<r>\xe9</r>", "application/xml", None).is_err());
        let bytes = [b"\xef\xbb\xbf".as_slice(), "<r>é</r>".as_bytes()].concat();
        assert_eq!(parse_response(&bytes, "text/xml", Some("windows-1252")).unwrap().1, "UTF-8");
        let doubled = [b"\xef\xbb\xbf".as_slice(), bytes.as_slice()].concat();
        assert!(parse_response(&doubled, "application/xml", None).is_err());
        for encoding in [amazing_encoding::Encoding::Utf16Le, amazing_encoding::Encoding::Utf16Be] {
            let marker = if encoding == amazing_encoding::Encoding::Utf16Le { [0xff, 0xfe] } else { [0xfe, 0xff] };
            let content = amazing_encoding::encode("<r/>", encoding, true).unwrap();
            assert!(parse_response(&[marker.as_slice(), content.as_slice()].concat(), "application/xml", None).is_ok());
            assert!(parse_response(&[marker.as_slice(), marker.as_slice(), content.as_slice()].concat(), "application/xml", None).is_err());
        }
        assert_eq!(parse_response(b"<r>\xe9</r>", "application/xml", Some("windows-1252")).unwrap().1, "windows-1252");
        assert!(parse_response(b"<r/>", "application/xml", Some("unknown")).is_err());
        for (bytes, charset) in [("<r>é</r>".as_bytes().to_vec(), Some("UTF-8")),
            (declared.to_vec(), Some("windows-1253")), (b"<r>\xe9</r>".to_vec(), None)] {
            let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
                "<!doctype html><body><iframe src='/entity.xml'></iframe>"), native_forwarded: false }));
            let mut scheduler = DriverScheduler::new(host.clone(), JS_CAPACITY).unwrap();
            scheduler.set_frame_resource_loader(Rc::new(move |_| Ok(response(
                "https://example.test/final.xml", Some("application/xml"), bytes.clone()))));
            scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("XML entity used legacy transport")));
            scheduler.set_location_url("https://example.test/root.html");
            scheduler.dispatch_pending_iframe_loads().unwrap();
            let expression = match charset {
                Some(label) => format!("document.querySelector('iframe').contentDocument.characterSet === '{label}'"),
                None => "document.querySelector('iframe').contentDocument === null".into(),
            };
            assert_eq!(scheduler.run_script(&expression).unwrap_or_else(|error| panic!("{expression}: {}", error.describe())), JsValue::Bool(true));
            assert!(host.try_borrow_mut().is_ok());
        }
    }

    #[test]
    fn xml_response_version_declarations_keep_xml_one_zero_processing_rules() {
        for version in ["1.0", "1.1", "1.2", "1.00", "1.123456789"] {
            let source = format!("<?xml version='{version}' encoding='windows-1253'?><r>&#x85;</r>");
            let (dom, label) = parse_response(source.as_bytes(), "application/xml", None).unwrap();
            assert_eq!(label, "windows-1253");
            assert!(dom.tree_order(NodeIdx(0)).into_iter().any(|node| matches!(&dom.get(node).kind, NodeKind::Text { data } if data.as_str() == "\u{85}")));
            let bytes = amazing_encoding::encode(&source, amazing_encoding::Encoding::Utf16Be, true).unwrap();
            let (dom, label) = parse_response(&[b"\xfe\xff".as_slice(), bytes.as_slice()].concat(), "text/xml", Some("UTF-8")).unwrap();
            assert_eq!(label, "UTF-16BE");
            assert!(dom.tree_order(NodeIdx(0)).into_iter().any(|node| matches!(&dom.get(node).kind, NodeKind::Text { data } if data.as_str() == "\u{85}")));
        }
        for source in ["<?xml version='2.0'?><r/>", "<?xml version='1.'?><r/>",
            "<?xml version='1.1'?><r>&#x1;</r>", "<?xml version='1.2' standalone='true'?><r/>"] {
            assert!(parse_response(source.as_bytes(), "application/xml", Some("UTF-8")).is_err());
        }
    }

}
