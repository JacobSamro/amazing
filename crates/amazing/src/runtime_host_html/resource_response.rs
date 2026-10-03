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
