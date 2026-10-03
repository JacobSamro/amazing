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
