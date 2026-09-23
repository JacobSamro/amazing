//! Renderer process: HTML bytes and networked subresources to an RGBA frame.
use crate::network::fetch_resource;
use crate::protocol::*;
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_ipc::codec::ByteBuf;
use amazing_ipc::{Connection, Error};
use amazing_ipc_platform::{FrameBudget, FrameHandle, HandleChannel, MAX_MAPPED_BYTES};
use amazing_layout::{IntrinsicSize, ReplacedElementProvider};
use amazing_paint::ImageProvider;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Images(BTreeMap<u32, amazing_media::Image>);
impl ReplacedElementProvider for Images {
    fn intrinsic_size(&self, node: NodeIdx) -> Option<IntrinsicSize> {
        self.0.get(&node.0).map(|image| IntrinsicSize {
            width: image.width as f32,
            height: image.height as f32,
        })
    }
}
impl ImageProvider for Images {
    fn image(&self, node: NodeIdx) -> Option<&amazing_media::Image> {
        self.0.get(&node.0)
    }
}
fn attribute<'a>(dom: &'a Dom, node: NodeIdx, name: &str) -> Option<&'a str> {
    dom.attrs_of(node)?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
fn image(bytes: &[u8]) -> Result<amazing_media::Image, Error> {
    if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'<') {
        amazing_svg::decode(bytes, amazing_svg::DecodeLimits::default())
            .map_err(|e| Error::Remote(format!("SVG: {e}")))
    } else {
        amazing_media::decode_image(bytes).map_err(|e| Error::Remote(format!("image: {e}")))
    }
}
/// The exact same public engine path runs in the renderer and in a browser-side
/// parity oracle. The loader is the only I/O seam.
pub fn render_document(
    request: &Navigate,
    mut load: impl FnMut(&str, &str) -> Result<Vec<u8>, Error>,
) -> Result<Frame, Error> {
    render_document_with_limit(request, &mut load, amazing_ipc::codec::MAX_FRAME - 1024)
}
fn render_document_with_limit(
    request: &Navigate,
    load: &mut impl FnMut(&str, &str) -> Result<Vec<u8>, Error>,
    limit: usize,
) -> Result<Frame, Error> {
    let width = request.viewport.width;
    let height = request.viewport.height;
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0
        || height == 0
        || width > amazing_ipc_platform::MAX_DIMENSION
        || height > amazing_ipc_platform::MAX_DIMENSION
        || pixels.is_none_or(|n| n > limit)
    {
        return Err(Error::Codec(amazing_ipc::codec::Error::Limit));
    }
    let dom = amazing_html::parse_bytes(&request.html_bytes.0, None);
    let mut css = String::new();
    let mut images = Images(BTreeMap::new());
    for n in 0..dom.node_count() {
        let node = NodeIdx(n as u32);
        match dom.name(node) {
            Some("style") => {
                for child in dom.children(node) {
                    if let NodeKind::Text { data } = &dom.get(child).kind {
                        css.push_str(data);
                        css.push('\n');
                    }
                }
            }
            Some("link")
                if attribute(&dom, node, "rel")
                    .is_some_and(|rel| rel.eq_ignore_ascii_case("stylesheet")) =>
            {
                if let Some(url) = attribute(&dom, node, "href") {
                    let bytes = load(url, &request.url)?;
                    css.push_str(
                        &String::from_utf8(bytes)
                            .map_err(|_| Error::Codec(amazing_ipc::codec::Error::Utf8))?,
                    );
                    css.push('\n');
                }
            }
            Some("img") => {
                if let Some(url) = attribute(&dom, node, "src") {
                    images.0.insert(node.0, image(&load(url, &request.url)?)?);
                }
            }
            _ => {}
        }
    }
    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_dom_document_with_replaced_in_viewport(
        &dom,
        &css,
        width as f32,
        Some(height as f32),
        &fonts,
        &images,
    );
    let surface = amazing_paint::paint_document_with_resources(
        &root,
        width,
        height,
        &fonts,
        &amazing_paint::RectInkProvider,
        &images,
    );
    Ok(Frame {
        sequence: 0,
        width,
        height,
        rgba: ByteBuf(surface.pixels),
    })
}
struct RendererState {
    sequence: u64,
    frame: Option<Frame>,
    scroll_y: i32,
    budget: FrameBudget,
    shared: BTreeMap<u64, FrameHandle>,
    in_flight: Option<u64>,
}
pub struct RendererEndpoint {
    network: NetworkProxy,
    events: BrowserEventsProxy,
    state: Mutex<RendererState>,
    channel: Option<HandleChannel>,
}
impl RendererEndpoint {
    pub fn new(connection: Arc<Connection>, channel: Option<HandleChannel>) -> Self {
        Self {
            network: NetworkProxy::new(connection.clone(), Duration::from_secs(5)),
            events: BrowserEventsProxy::new(connection, Duration::from_secs(5)),
            state: Mutex::new(RendererState {
                sequence: 0,
                frame: None,
                scroll_y: 0,
                budget: FrameBudget::new(),
                shared: BTreeMap::new(),
                in_flight: None,
            }),
            channel,
        }
    }
    fn notify(&self, sequence: u64, shared: Option<SharedFrameReady>) -> Result<(), Error> {
        self.events.frame_ready(FrameReady { sequence, shared })
    }
}
impl RendererControls for RendererEndpoint {
    fn navigate(&self, request: Navigate) -> Result<Frame, Error> {
        if !cfg!(feature = "byte-frame-fallback") {
            return Err(Error::Remote("byte frame fallback disabled".into()));
        }
        let mut frame = render_document(&request, |url, base| {
            fetch_resource(&self.network, url, base)
        })?;
        let mut state = self.state.lock().unwrap();
        for generation in state.budget.discard_unpresented() {
            state.shared.remove(&generation);
        }
        state.sequence += 1;
        frame.sequence = state.sequence;
        state.frame = Some(frame.clone());
        state.scroll_y = 0;
        drop(state);
        self.notify(frame.sequence, None)?;
        Ok(frame)
    }
    fn input(&self, event: InputEvent) -> Result<InputAck, Error> {
        let mut state = self.state.lock().unwrap();
        if state.frame.is_none() {
            return Err(Error::Remote("no document".into()));
        }
        for generation in state.budget.discard_unpresented() {
            state.shared.remove(&generation);
        }
        state.sequence += 1;
        if let InputEvent::Scroll(delta) = event {
            state.scroll_y = state.scroll_y.saturating_add(delta.y);
            if let Some(frame) = &mut state.frame {
                let row = frame.width as usize * 4;
                let shift = (delta.y.unsigned_abs() as usize).min(frame.height as usize);
                if delta.y > 0 {
                    frame.rgba.0.copy_within(shift * row.., 0);
                } else if delta.y < 0 {
                    frame
                        .rgba
                        .0
                        .copy_within(..(frame.height as usize - shift) * row, shift * row);
                }
            }
        }
        let sequence = state.sequence;
        drop(state);
        self.notify(sequence, None)?;
        Ok(InputAck { sequence })
    }
    fn navigate_shared(&self, request: Navigate) -> Result<SharedFrameReady, Error> {
        self.channel
            .as_ref()
            .ok_or_else(|| Error::Remote("shared frames unavailable".into()))?;
        let frame = render_document_with_limit(
            &request,
            &mut |url, base| fetch_resource(&self.network, url, base),
            MAX_MAPPED_BYTES,
        )?;
        let mut state = self.state.lock().unwrap();
        for generation in state.budget.discard_unpresented() {
            state.shared.remove(&generation);
        }
        let sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| Error::Remote("frame sequence exhausted".into()))?;
        let replaced = state
            .budget
            .reserve(sequence, frame.rgba.0.len())
            .map_err(|e| Error::Remote(e.to_string()))?;
        for generation in replaced {
            state.shared.remove(&generation);
        }
        let handle = match FrameHandle::new(sequence, frame.width, frame.height, &frame.rgba.0) {
            Ok(handle) => handle,
            Err(e) => {
                state.budget.release(sequence);
                return Err(Error::Remote(e.to_string()));
            }
        };
        let ready = SharedFrameReady::from(handle.metadata());
        state.shared.insert(sequence, handle);
        state.sequence = sequence;
        state.frame = Some(frame);
        state.scroll_y = 0;
        drop(state);
        self.notify(sequence, Some(ready.clone()))?;
        Ok(ready)
    }
    fn release_frame(&self, request: ReleaseFrame) -> Result<bool, Error> {
        let mut state = self.state.lock().unwrap();
        let released = state.shared.remove(&request.generation).is_some();
        if released {
            state.budget.release(request.generation);
        }
        Ok(released)
    }
    fn acquire_frame(&self, request: AcquireFrame) -> Result<AcquireResult, Error> {
        let channel = self
            .channel
            .as_ref()
            .ok_or_else(|| Error::Remote("shared frames unavailable".into()))?;
        let mut state = self.state.lock().unwrap();
        if request.generation != state.sequence || !state.shared.contains_key(&request.generation) {
            return Ok(AcquireResult::Superseded(state.sequence));
        }
        if state.in_flight.is_some() {
            return Ok(AcquireResult::Busy(()));
        }
        state
            .shared
            .get(&request.generation)
            .unwrap()
            .publish(channel)
            .map_err(|e| Error::Remote(e.to_string()))?;
        state.in_flight = Some(request.generation);
        state.budget.protect(request.generation); // Acquired frames cannot be replaced.
        Ok(AcquireResult::Transferred(()))
    }
    fn frame_received(&self, request: AcquireFrame) -> Result<bool, Error> {
        let mut state = self.state.lock().unwrap();
        if state.in_flight == Some(request.generation) {
            state.in_flight = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
/// Bind a renderer process to its browser IPC connection.
pub fn serve_renderer(connection: &Arc<Connection>, channel: Option<HandleChannel>) {
    connection.bind(
        7,
        RendererReceiver(RendererEndpoint::new(connection.clone(), channel)),
    );
    connection.bind(0, crate::Ping);
}
