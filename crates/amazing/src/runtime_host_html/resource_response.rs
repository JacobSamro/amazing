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
            (loader, context, encoding, mode, credentials, referrer_policy)
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
            request_mode: snapshot.3, credentials: snapshot.4, referrer_policy: snapshot.5 };
        let response = loader(&request).ok()?;
        decode_classic_response(response, snapshot.2).ok()
    }
}

fn classic_options(dom: &Dom, script: NodeIdx, default: amazing_fetch::ReferrerPolicy) ->
    (amazing_fetch::RequestMode, amazing_fetch::CredentialsMode, amazing_fetch::ReferrerPolicy) {
    use amazing_fetch::{CredentialsMode, ReferrerPolicy, RequestMode};
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
    pub(super) fn document_referrer_policy(&self, dom: &Dom, document: NodeIdx)
        -> Option<amazing_fetch::ReferrerPolicy> {
        if !matches!(dom.kind(document), Some(NodeKind::Document)) { return None; }
        let generation = dom.node_generation(document)?;
        Some(self.document_referrer_policies.get(&document).filter(|(saved, _)| *saved == generation)
            .map(|(_, policy)| *policy).unwrap_or(amazing_fetch::ReferrerPolicy::StrictOriginWhenCrossOrigin))
    }
}

impl RuntimeBindingHost<'_> {
    pub(super) fn html_document_referrer_policy(&self, document: NodeIdx)
        -> Option<amazing_fetch::ReferrerPolicy> {
        let state = self.state.borrow(); let host = state.host.borrow();
        state.frames.document_referrer_policy(&host.dom, document)
    }
    pub(super) fn html_set_document_referrer_policy(&self, document: NodeIdx,
        policy: amazing_fetch::ReferrerPolicy) -> Result<(), amazing_dom::DomError> {
        let generation = {
            let state = self.state.borrow(); let host = state.host.borrow();
            if !matches!(host.dom.kind(document), Some(NodeKind::Document)) {
                return Err(amazing_dom::DomError::InvalidStateError);
            }
            host.dom.node_generation(document).ok_or(amazing_dom::DomError::InvalidStateError)?
        };
        self.state.borrow_mut().frames.document_referrer_policies.insert(document, (generation, policy));
        Ok(())
    }
}

impl RuntimeBindings {
    pub(crate) fn set_document_response_referrer_policy(&self, document: NodeIdx,
        headers: &amazing_net::Headers) -> Result<(), amazing_dom::DomError> {
        let policy = headers.get_all("referrer-policy").into_iter()
            .filter_map(amazing_fetch::ReferrerPolicy::parse_header).last();
        let Some(policy) = policy else { return Ok(()); };
        let generation = {
            let state = self.state.borrow(); let host = state.host.borrow();
            if !matches!(host.dom.kind(document), Some(NodeKind::Document)) {
                return Err(amazing_dom::DomError::InvalidStateError);
            }
            host.dom.node_generation(document).ok_or(amazing_dom::DomError::InvalidStateError)?
        };
        self.state.borrow_mut().frames.document_referrer_policies.insert(document, (generation, policy));
        Ok(())
    }
}

/// HTML fetch-a-classic-script plus Encoding decode. No HTML meta prescan:
/// header charset overrides fallback, and a real BOM overrides either one.
fn decode_classic_response(response: amazing_net::Response,
    fallback: amazing_encoding::Encoding) -> Result<(String, String), String> {
    if !response.is_success() { return Err(format!("classic script status {}", response.status)); }
    if response.body.len() > 16 * 1024 * 1024 { return Err("classic script body too large".into()); }
    ParsedUrl::parse(&response.url).map_err(|error| format!("classic script final URL: {error:?}"))?;
    let mime = response.content_type().unwrap_or("").to_ascii_lowercase();
    let javascript = amazing_html::is_javascript_mime_type(&mime);
    let nosniff = response.headers.get("x-content-type-options")
        .is_some_and(|value| value.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("nosniff"));
    if (!javascript && nosniff) || mime.starts_with("image/") || mime.starts_with("audio/")
        || mime.starts_with("video/") || mime == "text/csv" {
        return Err(format!("blocked classic script media type: {mime}"));
    }
    let encoding = response.headers.get("content-type").and_then(|header|
        header.split(';').skip(1).find_map(|parameter| {
            let (name, value) = parameter.trim().split_once('=')?;
            if !name.trim().eq_ignore_ascii_case("charset") { return None; }
            amazing_encoding::encoding_for_label(value.trim().trim_matches('"'))
        })).unwrap_or(fallback);
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
        if let Some(mime) = mime { headers.append("Content-Type", mime).unwrap(); }
        amazing_net::Response { url: url.into(), status: 200, reason: "OK".into(), headers, body }
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
        headers.append("Referrer-Policy", "origin, invalid").unwrap();
        headers.append("Referrer-Policy", "no-referrer, same-origin").unwrap();
        headers.append("Referrer-Policy", "INVALID").unwrap();
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
            value.headers.append("X-Content-Type-Options", "nosniff").unwrap();
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
        scheduler.set_location_url("https://example.test/root/main.html");
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
        scheduler.dispatch_pending_iframe_loads().unwrap();
        assert_eq!(scheduler.run_script("var f=document.querySelector('iframe');[f.contentDocument.URL,f.contentDocument.characterSet,f.contentDocument.getElementById('p').textContent,f.contentWindow.classicValue].join('|')").unwrap(),
            JsValue::str("https://example.test/final/child.html|windows-1252|é|é"));
        let requests = requests.borrow();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].requested_url, "https://example.test/requested.xml");
        assert_eq!(requests[0].initiator.document_url, "https://example.test/root/main.html");
        assert!(requests[0].embedding.is_some());
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
            response.headers.append("Content-Security-Policy", "connect-src 'none'").unwrap();
            response.headers.append("Referrer-Policy", "origin, same-origin").unwrap();
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
        for case in 0..4 {
            let host = Rc::new(RefCell::new(BrowserDomHost { dom: amazing_html::parse(
                "<!doctype html><body><iframe src='/requested.xml'></iframe>"), native_forwarded: false }));
            let mut scheduler = DriverScheduler::new(host, JS_CAPACITY).unwrap();
            scheduler.set_location_url("https://example.test/root.html");
            scheduler.set_frame_resource_resolver(Rc::new(|_| panic!("canonical failure fell back to pinned transport")));
            scheduler.set_frame_resource_loader(Rc::new(move |_| {
                if case == 0 { return Err("actual transport error".into()); }
                let mut response = response("https://example.test/final.xml", None,
                    if case == 1 { b"<root/>".to_vec() } else { b"<!doctype html><p>child".to_vec() });
                if case == 2 { response.headers.append("X-Content-Type-Options", "nosniff").unwrap(); }
                if case == 3 { response.headers.append("X-Frame-Options", "DENY").unwrap(); }
                Ok(response)
            }));
            scheduler.dispatch_pending_iframe_loads().unwrap();
            assert_eq!(scheduler.run_script("document.querySelector('iframe').contentDocument").unwrap(), JsValue::Null);
        }
    }
}
