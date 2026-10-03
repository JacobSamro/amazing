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
    pub(crate) kind: HtmlFrameResourceKind,
    pub(crate) embedding: Option<NavigableContext>,
}

/// Returns the actual final URL, response headers and decoded transfer bytes.
/// Errors remain errors; they never select the legacy pinned resolver.
pub(crate) type HtmlFrameResourceLoader =
    Rc<dyn Fn(&HtmlFrameResourceRequest) -> Result<amazing_net::Response, String>>;

impl RuntimeBindings {
    pub(crate) fn set_frame_resource_loader(&self, loader: HtmlFrameResourceLoader) {
        self.state.borrow_mut().frames.resource_loader = Some(loader);
    }

    pub(super) fn fetch_document_classic_script(&self, document: NodeIdx, script: NodeIdx, src: &str,
        base: &str, resolver: Option<&FrameResourceResolver>) -> Option<(String, String)> {
        let url = ParsedUrl::parse(base).ok()?.resolve(src).ok()?.serialize();
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
            (loader, context, encoding)
        };
        let Some(loader) = snapshot.0 else {
            return fetch_child_script(src, base, resolver).map(|text| (text, url));
        };
        let request = HtmlFrameResourceRequest { requested_url: url,
            initiator: snapshot.1?, kind: HtmlFrameResourceKind::ClassicScript, embedding: None };
        let response = loader(&request).ok()?;
        decode_classic_response(response, snapshot.2).ok()
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
        scheduler.set_frame_resource_loader(Rc::new(move |request| {
            assert!(host.try_borrow_mut().is_ok(), "transport called with a live DOM borrow");
            observed.borrow_mut().push(request.clone());
            Ok(match request.kind {
                HtmlFrameResourceKind::Document => response("https://example.test/final/child.html",
                    Some("text/html; charset=windows-1252"),
                    b"<!doctype html><body><script src='script.js'></script><p id=p>\xe9</p>".to_vec()),
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
        assert_eq!(requests[1].requested_url, "https://example.test/final/script.js");
        assert_eq!(requests[1].initiator.document_url, "https://example.test/final/child.html");
        assert!(requests[1].embedding.is_none());
        assert_eq!(requests[1].initiator.ancestor_origins, vec![requests[0].initiator.origin.clone()]);
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
