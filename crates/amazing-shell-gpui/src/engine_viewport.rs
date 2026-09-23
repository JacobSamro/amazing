use std::sync::Arc;
use std::{cell::RefCell, rc::Rc};

use amazing_net::{FetchOptions, Url, fetch, resolve};
use amazing_paint::Surface;
use amazing_text_provider::TextProvider;
use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::dom_runtime::ShellDomHost;
use crate::engine_compositor::EngineCompositor;
use crate::session_state::BrowserSession;

const VIEWPORT_WIDTH: u32 = 960;
const VIEWPORT_HEIGHT: u32 = 570;
const MAX_STYLESHEETS: usize = 32;
const MAX_STYLESHEET_BYTES: usize = 4 * 1024 * 1024;
const MAX_PAGE_IMAGES: usize = 32;
const MAX_DOCUMENT_SCRIPTS: usize = 32;
const MAX_SCRIPT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct PageImage {
    node: amazing_dom::NodeIdx,
    image: amazing_media::Image,
}

#[derive(Clone, Default)]
struct PageResources {
    images: Vec<PageImage>,
    backgrounds: Vec<(String, amazing_media::Image)>,
}

impl amazing_layout::ReplacedElementProvider for PageResources {
    fn intrinsic_size(
        &self,
        node: amazing_dom::NodeIdx,
    ) -> Option<amazing_layout::IntrinsicSize> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| amazing_layout::IntrinsicSize {
                width: entry.image.width as f32,
                height: entry.image.height as f32,
            })
    }
}

impl amazing_paint::ImageProvider for PageResources {
    fn background_image(&self, _node: amazing_dom::NodeIdx, url: &str) -> Option<&amazing_media::Image> {
        self.backgrounds.iter().find(|(key,_)| key == url).map(|(_,image)| image)
    }
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| &entry.image)
    }
}

const DEMO_HTML: &str = r#"
<!doctype html>
<html>
  <body>
    <main>
      <p class="eyebrow">AMAZING BROWSER ENGINE</p>
      <h1>The web, from first principles.</h1>
      <p class="intro">This page is parsed, styled, laid out, and rasterized by Amazing. GPUI presents the final pixel surface inside the desktop browser chrome.</p>
      <section class="cards">
        <article><strong>HTML</strong><p>Spec-driven tokenization and tree construction.</p></article>
        <article><strong>CSS</strong><p>Cascade, flex, grid, and intrinsic layout.</p></article>
        <article><strong>PAINT</strong><p>Software rasterization into a native GPUI viewport.</p></article>
      </section>
      <p class="proof">LIVE ENGINE SURFACE  ·  960 × 570 RGBA8  ·  WIN-001</p>
    </main>
  </body>
</html>
"#;

const DEMO_CSS: &str = r#"
html, body { display: block; margin: 0px; padding: 0px; background: #f7f3ed; color: #1b1e23; }
main { display: block; padding: 58px 68px; border-top: 8px solid #b1552b; }
.eyebrow { display: block; margin: 0px 0px 22px 0px; color: #b1552b; font-size: 13px; }
h1 { display: block; margin: 0px 0px 18px 0px; color: #1b1e23; font-size: 48px; }
.intro { display: block; width: 720px; margin: 0px 0px 34px 0px; color: #4e5664; font-size: 18px; }
.cards { display: flex; gap: 14px; margin: 0px 0px 28px 0px; }
article { display: block; width: 238px; min-height: 112px; padding: 18px; background: #ffffff; border: 1px solid #d8dce3; }
article strong { color: #1b1e23; font-size: 13px; }
article p { display: block; margin: 12px 0px 0px 0px; color: #5b6371; font-size: 14px; }
.proof { display: block; margin: 0px; padding-top: 18px; border-top: 1px solid #d8dce3; color: #737b88; font-size: 11px; }
"#;

const NAVIGATION_CSS: &str = r#"
html, body { display: block; margin: 0px; padding: 0px; background: #f7f3ed; color: #1b1e23; }
main { display: block; padding: 70px 76px; border-top: 8px solid #3a6ea8; }
.eyebrow { display: block; margin: 0px 0px 24px 0px; color: #3a6ea8; font-size: 13px; }
h1 { display: block; margin: 0px 0px 20px 0px; color: #1b1e23; font-size: 44px; }
.intro { display: block; width: 720px; margin: 0px 0px 32px 0px; color: #4e5664; font-size: 18px; }
.address { display: block; padding: 20px; background: #ffffff; border: 1px solid #d8dce3; color: #1b1e23; font-size: 15px; }
.note { display: block; width: 720px; margin: 26px 0px 0px 0px; color: #737b88; font-size: 13px; }
"#;

const NEW_TAB_HTML: &str = r#"
<!doctype html>
<html>
  <body>
    <main class="new-tab">
      <div class="mark">A</div>
      <h1>Search the web</h1>
      <p class="intro">Type a URL or search query in the address bar.</p>
      <section class="shortcuts">
        <article><strong>Engine status</strong><p>amazing://status</p></article>
        <article><strong>HTML Standard</strong><p>html.spec.whatwg.org</p></article>
        <article><strong>Local test server</strong><p>localhost:8080</p></article>
      </section>
      <p class="proof">AMAZING · FROM-SCRATCH BROWSER ENGINE</p>
    </main>
  </body>
</html>
"#;

const NEW_TAB_CSS: &str = r#"
html, body { display: block; margin: 0px; padding: 0px; background: #f8f9fa; color: #202124; }
.new-tab { display: block; padding: 108px 110px 70px 110px; }
.mark { display: block; width: 58px; height: 58px; padding: 13px 0px 0px 0px; background: #b1552b; color: #ffffff; font-size: 29px; text-align: center; }
h1 { display: block; margin: 24px 0px 10px 0px; font-size: 35px; color: #202124; }
.intro { display: block; margin: 0px 0px 38px 0px; color: #5f6368; font-size: 16px; }
.shortcuts { display: flex; gap: 14px; }
article { display: block; width: 220px; min-height: 92px; padding: 16px; background: #ffffff; border: 1px solid #dadce0; }
article strong { color: #202124; font-size: 13px; }
article p { display: block; margin: 10px 0px 0px 0px; color: #5f6368; font-size: 12px; }
.proof { display: block; margin: 46px 0px 0px 0px; color: #80868b; font-size: 11px; }
"#;

#[derive(Clone)]
pub struct EngineViewport {
    pub image: Arc<RenderImage>,
    pub compositor: Option<Rc<RefCell<EngineCompositor>>>,
    page: Option<Rc<RetainedPage>>,
    pub width: u32,
    pub height: u32,
    links: Arc<Vec<LinkRegion>>,
    controls: Arc<Vec<ControlRegion>>,
}

struct RetainedPage {
    root: amazing_layout::LayoutBox,
    text: TextProvider,
    styles: amazing_css::StyledDom,
    resources: PageResources,
}

#[derive(Debug, Clone)]
struct LinkRegion {
    node: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    Text,
    Submit,
}

#[derive(Debug, Clone)]
struct FormField {
    name: String,
    value: String,
}

#[derive(Debug, Clone)]
struct ControlRegion {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    kind: ControlKind,
    name: String,
    value: String,
    action: String,
    method: String,
    fields: Vec<FormField>,
}

pub type ControlRect = (f32, f32, f32, f32);

pub struct FormRequest {
    pub target: String,
    pub method: String,
    pub body: Vec<u8>,
}

pub struct LoadedPage {
    pub viewport: EngineViewport,
    pub final_address: String,
    pub title: Option<String>,
    pub favicon: Option<Arc<RenderImage>>,
    pub runtime: Option<PageRuntime>,
}

pub struct RuntimeUpdate {
    pub viewport: EngineViewport,
    pub title: Option<String>,
    pub favicon: Option<Arc<RenderImage>>,
    pub navigation: Option<String>,
    pub history_delta: Option<i32>,
    pub history_update: Option<(String, bool)>,
    pub default_prevented: bool,
}

pub struct PageRuntime {
    host: Rc<RefCell<ShellDomHost>>,
    interpreter: amazing_js::Interpreter,
    document_url: String,
    options: FetchOptions,
    session: Rc<RefCell<BrowserSession>>,
}

impl PageRuntime {
    fn new(
        dom: amazing_dom::Dom,
        document_url: String,
        options: FetchOptions,
        scripts: Vec<String>,
        session: Rc<RefCell<BrowserSession>>,
    ) -> Self {
        let host = Rc::new(RefCell::new(ShellDomHost::new(
            dom,
            document_url.clone(),
            session.clone(),
        )));
        let mut interpreter = amazing_js::Interpreter::new();
        interpreter.set_dom_host(host.clone());
        interpreter.set_location_url(document_url.clone());
        for source in scripts {
            let _ = interpreter.run_script(&source);
        }
        for _ in 0..32 {
            match interpreter.pump_timers() {
                Ok(true) => {},
                Ok(false) | Err(_) => break,
            }
        }
        Self {
            host,
            interpreter,
            document_url,
            options,
            session,
        }
    }

    fn snapshot(&mut self, default_prevented: bool) -> RuntimeUpdate {
        let navigation = self.interpreter.take_navigation();
        let history_delta = self.interpreter.take_history_delta();
        let history_update = self
            .interpreter
            .take_history_update()
            .map(|(url, replace, _)| (url, replace));
        let host = self.host.borrow();
        let dom = &host.dom;
        let (css, font_sheets) = load_document_styles(
            dom,
            &self.document_url,
            &self.options,
            Some(&self.session),
        );
        let resources = load_page_resources(
            dom,
            &css,
            &self.document_url,
            &self.options,
            Some(&self.session),
        );
        RuntimeUpdate {
            viewport: EngineViewport::from_dom_with_style_sources(
                dom,
                &css,
                resources,
                Some(&self.document_url),
                &font_sheets,
            ),
            title: document_title(dom),
            favicon: load_favicon(
                dom,
                &self.document_url,
                &self.options,
                Some(&self.session),
            ),
            navigation,
            history_delta,
            history_update,
            default_prevented,
        }
    }

    pub fn dispatch_click(&mut self, node: u32) -> RuntimeUpdate {
        let _ = self.interpreter.dispatch_dom_event(node, "click");
        let prevented = self.interpreter.last_event_default_prevented();
        self.snapshot(prevented)
    }

    pub fn pump(&mut self) -> Option<RuntimeUpdate> {
        let mut ran = false;
        for _ in 0..32 {
            match self.interpreter.pump_timers() {
                Ok(true) => ran = true,
                Ok(false) | Err(_) => break,
            }
        }
        if ran {
            Some(self.snapshot(false))
        } else {
            None
        }
    }

    pub fn set_history_length(&mut self, length: usize) {
        self.interpreter.set_history_length(length);
    }

    pub fn set_document_url(&mut self, url: String) {
        self.document_url = url.clone();
        self.interpreter.set_location_url(url);
    }
}

impl EngineViewport {
    pub fn demo() -> Self {
        Self::from_document(DEMO_HTML, DEMO_CSS)
    }

    pub fn new_tab() -> Self {
        Self::from_document(NEW_TAB_HTML, NEW_TAB_CSS)
    }

    pub fn load_request(
        address: &str,
        method: &str,
        body: Vec<u8>,
        session: Rc<RefCell<BrowserSession>>,
    ) -> Result<LoadedPage, String> {
        if address == "amazing://status" {
            return Ok(LoadedPage {
                viewport: Self::demo(),
                final_address: address.to_string(),
                title: Some("Amazing — Phase 9 status".into()),
                favicon: None,
                runtime: None,
            });
        }
        if address == "amazing://newtab" {
            return Ok(LoadedPage {
                viewport: Self::new_tab(),
                final_address: address.to_string(),
                title: Some("New tab".into()),
                favicon: None,
                runtime: None,
            });
        }
        let options = FetchOptions {
            timeout: std::time::Duration::from_secs(3),
            request_headers: if method.eq_ignore_ascii_case("POST") {
                vec![(
                    "Content-Type".into(),
                    "application/x-www-form-urlencoded".into(),
                )]
            } else {
                Vec::new()
            },
            method: method.to_string(),
            body,
            ..FetchOptions::default()
        };
        let response = session
            .borrow_mut()
            .fetch(address, &options)
            .map_err(|error| error.to_string())?;
        let content_type = response
            .content_type()
            .unwrap_or("application/octet-stream");
        if content_type != "text/html" && content_type != "application/xhtml+xml" {
            return match session.borrow_mut().save_download(&response) {
                Ok(download) => Err(format!(
                    "downloaded {} to {}",
                    download.name, download.path
                )),
                Err(error) => Err(format!(
                    "cannot render response type {content_type}; download failed: {error}"
                )),
            };
        }
        let final_address = response.url.clone();
        let html = response
            .text()
            .ok_or_else(|| "response body was not decodable text".to_string())?;
        let dom = amazing_html::parse(&html);
        let scripts = collect_document_scripts(&dom, &final_address, &options, Some(&session));
        if !scripts.is_empty() {
            let mut runtime = PageRuntime::new(
                dom,
                final_address.clone(),
                options,
                scripts,
                session,
            );
            let update = runtime.snapshot(false);
            return Ok(LoadedPage {
                viewport: update.viewport,
                final_address,
                title: update.title,
                favicon: update.favicon,
                runtime: Some(runtime),
            });
        }
        let (css, font_sheets) = load_document_styles(&dom, &final_address, &options, Some(&session));
        let resources = load_page_resources(&dom, &css, &final_address, &options, Some(&session));
        let title = document_title(&dom);
        let favicon = load_favicon(&dom, &final_address, &options, Some(&session));
        Ok(LoadedPage {
            viewport: Self::from_dom_with_style_sources(
                &dom,
                &css,
                resources,
                Some(&final_address),
                &font_sheets,
            ),
            final_address,
            title,
            favicon,
            runtime: None,
        })
    }

    pub fn error(address: &str, error: &str) -> Self {
        let safe_address = escape_html(address);
        let safe_error = escape_html(error);
        let html = if error.starts_with("downloaded ") {
            format!(
                r#"<!doctype html><html><body><main>
                <p class="eyebrow">AMAZING DOWNLOAD</p>
                <h1>Download complete.</h1>
                <p class="intro">The response was saved locally and added to Downloads.</p>
                <p class="address">{safe_address}</p>
                <p class="note">{safe_error}</p>
                </main></body></html>"#
            )
        } else {
            format!(
                r#"<!doctype html><html><body><main>
                <p class="eyebrow">AMAZING NAVIGATION ERROR</p>
                <h1>This address could not be loaded.</h1>
                <p class="intro">Amazing kept the failure inside the viewport and preserved the requested address.</p>
                <p class="address">{safe_address}</p>
                <p class="note">{safe_error}</p>
                </main></body></html>"#
            )
        };
        Self::from_document(&html, NAVIGATION_CSS)
    }

    #[cfg(test)]
    pub fn video_demo() -> Result<Self, String> {
        Self::video_demo_frames().map(|mut frames| frames.remove(0))
    }

    pub fn video_demo_frames() -> Result<Vec<Self>, String> {
        const WIDTH: usize = 320;
        const HEIGHT: usize = 180;
        let mut encoder =
            amazing_webrtc_codecs::H264EncoderAdapter::new().map_err(|error| error.to_string())?;
        let mut decoder =
            amazing_webrtc_codecs::H264DecoderAdapter::new().map_err(|error| error.to_string())?;
        let mut frames = Vec::new();
        for phase in 0..6 {
            let mut rgb = Vec::with_capacity(WIDTH * HEIGHT * 3);
            for row in 0..HEIGHT {
                for column in 0..WIDTH {
                    let panel = (column / 40 + row / 30 + phase) % 2;
                    rgb.extend_from_slice(&[
                        (35 + (column + phase * 11) % WIDTH * 180 / WIDTH) as u8,
                        (45 + (row + phase * 7) % HEIGHT * 150 / HEIGHT) as u8,
                        if panel == 0 { 185 } else { 75 },
                    ]);
                }
            }
            let encoded = encoder
                .encode_rgb(&rgb, (WIDTH, HEIGHT))
                .map_err(|error| error.to_string())?;
            let decoded = decoder
                .decode(&encoded)
                .map_err(|error| error.to_string())?;
            let image = Arc::new(rgba_to_render_image(
                decoded.width,
                decoded.height,
                decoded.rgba,
            ));
            frames.push(Self {
                image,
                compositor: None,
                page: None,
                width: decoded.width,
                height: decoded.height,
                links: Arc::new(Vec::new()),
                controls: Arc::new(Vec::new()),
            });
        }
        Ok(frames)
    }

    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, String> {
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "RGBA surface dimensions overflow".to_string())?;
        if rgba.len() != expected || !rgba.chunks_exact(4).all(|pixel| pixel[3] == 255) {
            return Err("RGBA surface is malformed or translucent".into());
        }
        Ok(Self {
            image: Arc::new(rgba_to_render_image(width, height, rgba)),
            compositor: None,
            page: None,
            width,
            height,
            links: Arc::new(Vec::new()),
            controls: Arc::new(Vec::new()),
        })
    }

    fn from_document(html: &str, css: &str) -> Self {
        let dom = amazing_html::parse(html);
        Self::from_dom(&dom, css)
    }

    fn from_dom(dom: &amazing_dom::Dom, css: &str) -> Self {
        Self::from_dom_with_resources(dom, css, &PageResources::default(), None)
    }

    fn from_dom_with_resources(
        dom: &amazing_dom::Dom,
        css: &str,
        resources: &PageResources,
        document_url: Option<&str>,
    ) -> Self {
        Self::from_dom_with_style_sources(dom, css, resources.clone(), document_url, &[])
    }

    fn from_dom_with_style_sources(
        dom: &amazing_dom::Dom,
        css: &str,
        resources: PageResources,
        document_url: Option<&str>,
        font_sheets: &[(String, String)],
    ) -> Self {
        let (surface, links, controls, scene) =
            render_dom_with_resources(dom, css, resources, document_url, font_sheets);
        let width = surface.width;
        let height = surface.height;
        let image = Arc::new(surface_to_render_image(surface));
        let (page, compositor) = match scene {
            Some((page, compositor)) => (
                Some(Rc::new(page)),
                Some(Rc::new(RefCell::new(compositor))),
            ),
            None => (None, None),
        };
        Self {
            image,
            compositor,
            page,
            width,
            height,
            links: Arc::new(links),
            controls: Arc::new(controls),
        }
    }

    /// Applies a property-only scene change while the original layout and
    /// paint providers remain alive. The caller should notify GPUI only when
    /// this returns true; a DOM/style rebuild creates a fresh viewport instead.
    pub fn update_compositor_properties(
        &self,
        change: impl FnOnce(&mut amazing_compositor::Compositor) -> Result<(), String>,
    ) -> Result<bool, String> {
        let (Some(page), Some(scene)) = (&self.page, &self.compositor) else {
            return Ok(false);
        };
        let images = amazing_paint::image::StyledImages {
            images: &page.resources,
            styles: &page.styles,
        };
        let view = amazing_paint::display_list_view::DisplayListView::new(
            &page.root,
            self.width,
            self.height,
            &page.text,
            &images,
            &amazing_paint::controls::NoControls,
        );
        let mut scene = scene.borrow_mut();
        change(scene.compositor_mut())?;
        scene.update(&view)
    }

    pub fn link_hit_at(&self, x: f32, y: f32) -> Option<(u32, &str)> {
        self.links
            .iter()
            .rev()
            .find(|region| {
                x >= region.x
                    && x <= region.x + region.width
                    && y >= region.y
                    && y <= region.y + region.height
            })
            .map(|region| (region.node, region.target.as_str()))
    }

    pub fn control_at(&self, x: f32, y: f32) -> Option<usize> {
        self.controls.iter().rposition(|control| {
            x >= control.x
                && x <= control.x + control.width
                && y >= control.y
                && y <= control.y + control.height
        })
    }

    pub fn text_control(&self, index: usize) -> Option<(&str, ControlRect)> {
        let control = self.controls.get(index)?;
        (control.kind == ControlKind::Text).then_some((
            control.value.as_str(),
            (control.x, control.y, control.width, control.height),
        ))
    }

    pub fn form_request(&self, index: usize, edited_value: Option<&str>) -> Option<FormRequest> {
        let control = self.controls.get(index)?;
        if !matches!(control.method.as_str(), "get" | "post") {
            return None;
        }
        let mut fields = control.fields.clone();
        if control.kind == ControlKind::Text
            && !control.name.is_empty()
            && let Some(field) = fields.iter_mut().find(|field| field.name == control.name)
        {
            field.value = edited_value.unwrap_or(&control.value).to_string();
        }
        if control.kind == ControlKind::Submit && !control.name.is_empty() {
            fields.push(FormField {
                name: control.name.clone(),
                value: control.value.clone(),
            });
        }
        let query = fields
            .iter()
            .filter(|field| !field.name.is_empty())
            .map(|field| {
                format!(
                    "{}={}",
                    encode_form_component(&field.name),
                    encode_form_component(&field.value)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        if control.method == "post" {
            Some(FormRequest {
                target: control.action.clone(),
                method: "POST".into(),
                body: query.into_bytes(),
            })
        } else if query.is_empty() {
            Some(FormRequest {
                target: control.action.clone(),
                method: "GET".into(),
                body: Vec::new(),
            })
        } else {
            Some(FormRequest {
                target: format!(
                    "{}{}{}",
                    control.action,
                    if control.action.contains('?') { "&" } else { "?" },
                    query
                ),
                method: "GET".into(),
                body: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
fn render_demo_surface() -> Surface {
    render_surface(DEMO_HTML, DEMO_CSS)
}

#[cfg(test)]
fn render_surface(html: &str, css: &str) -> Surface {
    let dom = amazing_html::parse(html);
    render_dom_with_resources(&dom, css, PageResources::default(), None, &[]).0
}

fn render_dom_with_resources(
    dom: &amazing_dom::Dom,
    css: &str,
    resources: PageResources,
    document_url: Option<&str>,
    font_sheets: &[(String, String)],
) -> (Surface, Vec<LinkRegion>, Vec<ControlRegion>, Option<(RetainedPage, EngineCompositor)>) {
    let text = load_page_text(css, document_url.unwrap_or("amazing://newtab"), font_sheets);
    let root = amazing_layout::layout_dom_document_with_text_and_replaced_in_viewport(
        dom,
        css,
        VIEWPORT_WIDTH as f32,
        Some(VIEWPORT_HEIGHT as f32),
        &text,
        &resources,
    );
    let page_height = amazing_layout::layout_overflow_bottom(&root)
        .ceil()
        .clamp(VIEWPORT_HEIGHT as f32, 8192.0) as u32;
    let links = document_url
        .map(|url| collect_link_regions(dom, &root, url))
        .unwrap_or_default();
    let controls = document_url
        .map(|url| collect_control_regions(dom, &root, url))
        .unwrap_or_default();
    let styles = amazing_css::compute(dom, &[amazing_css::parse_stylesheet(css)]);
    let images = amazing_paint::image::StyledImages { images: &resources, styles: &styles };
    let surface = amazing_paint::paint_document_with_text_resources_and_controls(
        &root,
        VIEWPORT_WIDTH,
        page_height,
        &text,
        &images,
        &amazing_paint::controls::NoControls,
    );
    let view = amazing_paint::display_list_view::DisplayListView::new(
        &root,
        VIEWPORT_WIDTH,
        page_height,
        &text,
        &images,
        &amazing_paint::controls::NoControls,
    );
    let compositor = match EngineCompositor::new(&view, VIEWPORT_WIDTH, page_height) {
        Ok(scene) => Some(scene),
        Err(error) => {
            eprintln!("amazing shell: compositor scene failed: {error}");
            None
        }
    };
    drop(view);
    drop(images);
    let scene = compositor.map(|compositor| (
        RetainedPage { root, text, styles, resources },
        compositor,
    ));
    (surface, links, controls, scene)
}

fn collect_link_regions(
    dom: &amazing_dom::Dom,
    layout: &amazing_layout::LayoutBox,
    document_url: &str,
) -> Vec<LinkRegion> {
    fn target_for_node(
        dom: &amazing_dom::Dom,
        mut node: amazing_dom::NodeIdx,
        document_url: &str,
    ) -> Option<(u32, String)> {
        loop {
            if dom.name(node) == Some("a")
                && let Some(href) = dom.attrs_of(node).and_then(|attrs| {
                    attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("href"))
                        .map(|(_, value)| value.as_str())
                })
            {
                return resolve_resource_address(document_url, href)
                    .map(|target| (node.0, target));
            }
            node = dom.get(node).parent?;
        }
    }

    fn walk(
        dom: &amazing_dom::Dom,
        layout: &amazing_layout::LayoutBox,
        document_url: &str,
        regions: &mut Vec<LinkRegion>,
    ) {
        for line in &layout.lines {
            for word in &line.words {
                if let Some((node, target)) = target_for_node(dom, word.node, document_url) {
                    regions.push(LinkRegion {
                        node,
                        x: word.x,
                        y: line.y_top,
                        width: word.width.max(1.0),
                        height: line.height.max(1.0),
                        target,
                    });
                }
            }
            for replaced in &line.replaced {
                if let Some((node, target)) = target_for_node(dom, replaced.node, document_url) {
                    regions.push(LinkRegion {
                        node,
                        x: replaced.x,
                        y: replaced.y,
                        width: replaced.width.max(1.0),
                        height: replaced.height.max(1.0),
                        target,
                    });
                }
            }
        }

        let children = match &layout.flow {
            amazing_layout::Flow::Blocks(children)
            | amazing_layout::Flow::Flex(children)
            | amazing_layout::Flow::Grid(children) => children.as_slice(),
            amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => &[],
        };
        for child in children {
            walk(dom, child, document_url, regions);
        }
        for child in &layout.positioned {
            walk(dom, child, document_url, regions);
        }
    }

    let mut regions = Vec::new();
    walk(dom, layout, document_url, &mut regions);
    regions
}

fn collect_control_regions(
    dom: &amazing_dom::Dom,
    layout: &amazing_layout::LayoutBox,
    document_url: &str,
) -> Vec<ControlRegion> {
    fn attribute<'a>(
        dom: &'a amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        name: &str,
    ) -> Option<&'a str> {
        dom.attrs_of(node)?
            .iter()
            .find(|(attribute, _)| attribute.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn form_node(dom: &amazing_dom::Dom, mut node: amazing_dom::NodeIdx) -> Option<amazing_dom::NodeIdx> {
        loop {
            if dom.name(node) == Some("form") {
                return Some(node);
            }
            node = dom.get(node).parent?;
        }
    }

    fn collect_fields(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        fields: &mut Vec<FormField>,
    ) {
        if dom.name(node) == Some("input")
            && attribute(dom, node, "disabled").is_none()
        {
            let kind = attribute(dom, node, "type")
                .unwrap_or("text")
                .to_ascii_lowercase();
            if !matches!(kind.as_str(), "submit" | "button" | "reset" | "image")
                && let Some(name) = attribute(dom, node, "name")
            {
                fields.push(FormField {
                    name: name.to_string(),
                    value: attribute(dom, node, "value").unwrap_or("").to_string(),
                });
            }
        }
        for child in dom.children(node) {
            collect_fields(dom, child, fields);
        }
    }

    fn descriptor(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        document_url: &str,
    ) -> Option<(ControlKind, String, String, String, String, Vec<FormField>)> {
        let tag = dom.name(node)?;
        let kind = match tag {
            "textarea" => ControlKind::Text,
            "button" => ControlKind::Submit,
            "input" => match attribute(dom, node, "type")
                .unwrap_or("text")
                .to_ascii_lowercase()
                .as_str()
            {
                "submit" | "image" => ControlKind::Submit,
                "text" | "search" | "url" | "email" | "password" | "tel" => {
                    ControlKind::Text
                },
                _ => return None,
            },
            _ => return None,
        };
        if attribute(dom, node, "disabled").is_some() {
            return None;
        }
        let form = form_node(dom, node);
        let action = form
            .and_then(|form| attribute(dom, form, "action"))
            .and_then(|action| resolve_resource_address(document_url, action))
            .unwrap_or_else(|| document_url.to_string());
        let method = form
            .and_then(|form| attribute(dom, form, "method"))
            .unwrap_or("get")
            .to_ascii_lowercase();
        let mut fields = Vec::new();
        if let Some(form) = form {
            collect_fields(dom, form, &mut fields);
        }
        Some((
            kind,
            attribute(dom, node, "name").unwrap_or("").to_string(),
            attribute(dom, node, "value").unwrap_or("").to_string(),
            action,
            method,
            fields,
        ))
    }

    fn walk(
        dom: &amazing_dom::Dom,
        layout: &amazing_layout::LayoutBox,
        document_url: &str,
        controls: &mut Vec<ControlRegion>,
    ) {
        if let Some(node) = layout.node
            && let Some((kind, name, value, action, method, fields)) =
                descriptor(dom, node, document_url)
            && layout.border_box.w > 0.0
            && layout.border_box.h > 0.0
        {
            controls.push(ControlRegion {
                x: layout.border_box.x,
                y: layout.border_box.y,
                width: layout.border_box.w,
                height: layout.border_box.h,
                kind,
                name,
                value,
                action,
                method,
                fields,
            });
        }
        let children = match &layout.flow {
            amazing_layout::Flow::Blocks(children)
            | amazing_layout::Flow::Flex(children)
            | amazing_layout::Flow::Grid(children) => children.as_slice(),
            amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => &[],
        };
        for child in children {
            walk(dom, child, document_url, controls);
        }
        for child in &layout.positioned {
            walk(dom, child, document_url, controls);
        }
    }

    let mut controls = Vec::new();
    walk(dom, layout, document_url, &mut controls);
    controls
}

fn encode_form_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            },
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn collect_inline_styles(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, out: &mut String) {
    if dom.name(node) == Some("style") {
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                out.push_str(data);
                out.push('\n');
            }
        }
        return;
    }
    for child in dom.children(node) {
        collect_inline_styles(dom, child, out);
    }
}

fn fetch_resource(
    session: Option<&Rc<RefCell<BrowserSession>>>,
    address: &str,
    options: &FetchOptions,
) -> Result<amazing_net::Response, amazing_net::NetError> {
    match session {
        Some(session) => session.borrow_mut().fetch(address, options),
        None => fetch(address, options),
    }
}

fn load_document_styles(
    dom: &amazing_dom::Dom,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
) -> (String, Vec<(String, String)>) {
    let mut css = String::new();
    let mut font_sheets = Vec::new();
    let mut external_count = 0usize;
    collect_style_sources(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        session,
        &mut external_count,
        &mut css,
        &mut font_sheets,
    );
    (css, font_sheets)
}

fn collect_document_scripts(
    dom: &amazing_dom::Dom,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
) -> Vec<String> {
    fn collect(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        document_url: &str,
        options: &FetchOptions,
        session: Option<&Rc<RefCell<BrowserSession>>>,
        scripts: &mut Vec<String>,
        total_bytes: &mut usize,
    ) {
        if scripts.len() >= MAX_DOCUMENT_SCRIPTS || *total_bytes >= MAX_SCRIPT_BYTES {
            return;
        }
        if dom.name(node) == Some("script") {
            let attrs = dom.attrs_of(node).unwrap_or(&[]);
            let script_type = attrs
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("type"))
                .map(|(_, value)| value.trim())
                .unwrap_or("");
            let classic = script_type.is_empty()
                || script_type.eq_ignore_ascii_case("text/javascript")
                || script_type.eq_ignore_ascii_case("application/javascript");
            if classic {
                let source = attrs
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("src"))
                    .and_then(|(_, source)| resolve_resource_address(document_url, source))
                    .and_then(|address| fetch_resource(session, &address, options).ok())
                    .filter(|response| response.status == 200)
                    .and_then(|response| response.text())
                    .or_else(|| {
                        let mut inline = String::new();
                        for child in dom.children(node) {
                            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                                inline.push_str(data);
                            }
                        }
                        (!inline.trim().is_empty()).then_some(inline)
                    });
                if let Some(source) = source {
                    let remaining = MAX_SCRIPT_BYTES.saturating_sub(*total_bytes);
                    if source.len() <= remaining {
                        *total_bytes += source.len();
                        scripts.push(source);
                    }
                }
            }
            return;
        }
        for child in dom.children(node) {
            collect(
                dom,
                child,
                document_url,
                options,
                session,
                scripts,
                total_bytes,
            );
        }
    }

    let mut scripts = Vec::new();
    let mut total_bytes = 0usize;
    collect(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        session,
        &mut scripts,
        &mut total_bytes,
    );
    scripts
}

fn collect_style_sources(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
    external_count: &mut usize,
    out: &mut String,
    font_sheets: &mut Vec<(String, String)>,
) {
    if dom.name(node) == Some("style") {
        let start = out.len();
        collect_inline_styles(dom, node, out);
        if out.len() > start {
            font_sheets.push((out[start..].to_owned(), document_url.to_owned()));
        }
        return;
    }

    if dom.name(node) == Some("link")
        && *external_count < MAX_STYLESHEETS
        && out.len() < MAX_STYLESHEET_BYTES
        && let Some(attrs) = dom.attrs_of(node)
    {
        let rel = attrs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("rel"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        let media = attrs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("media"))
            .map(|(_, value)| value.trim())
            .unwrap_or("");
        let disabled = attrs
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("disabled"));
        let is_stylesheet = rel
            .split_ascii_whitespace()
            .any(|token| token.eq_ignore_ascii_case("stylesheet"));
        let applies_to_screen = media.is_empty()
            || media.eq_ignore_ascii_case("all")
            || media.eq_ignore_ascii_case("screen");
        if is_stylesheet
            && applies_to_screen
            && !disabled
            && let Some(href) = attrs
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("href"))
                .map(|(_, value)| value.as_str())
            && let Some(address) = resolve_resource_address(document_url, href)
        {
            *external_count += 1;
            if let Ok(response) = fetch_resource(session, &address, options)
                && response.status == 200
                && response
                    .content_type()
                    .is_none_or(|kind| kind == "text/css" || kind == "text/plain")
                && let Some(sheet) = response.text()
            {
                let remaining = MAX_STYLESHEET_BYTES.saturating_sub(out.len());
                if sheet.len() <= remaining {
                    font_sheets.push((sheet.clone(), address));
                    out.push_str(&sheet);
                    out.push('\n');
                }
            }
        }
    }

    for child in dom.children(node) {
        collect_style_sources(
            dom,
            child,
            document_url,
            options,
            session,
            external_count,
            out,
            font_sheets,
        );
    }
}

fn resolve_resource_address(document_url: &str, reference: &str) -> Option<String> {
    let reference = reference.trim();
    if reference.is_empty() {
        return None;
    }
    if let Ok(base) = Url::parse(document_url) {
        return resolve(reference, &base).ok().map(|url| url.to_string());
    }

    let reference_path = std::path::Path::new(reference);
    let path = if reference_path.is_absolute() {
        reference_path.to_path_buf()
    } else {
        std::path::Path::new(document_url)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .join(reference_path)
    };
    Some(path.to_string_lossy().into_owned())
}

fn load_page_resources(
    dom: &amazing_dom::Dom,
    css: &str,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
) -> PageResources {
    fn walk(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        document_url: &str,
        options: &FetchOptions,
        session: Option<&Rc<RefCell<BrowserSession>>>,
        resources: &mut PageResources,
    ) {
        if resources.images.len() >= MAX_PAGE_IMAGES {
            return;
        }
        if dom.name(node) == Some("img")
            && let Some(source) = dom
                .attrs_of(node)
                .and_then(|attrs| {
                    attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("src"))
                })
                .map(|(_, value)| value.as_str())
            && let Some(address) = resolve_resource_address(document_url, source)
            && let Ok(response) = fetch_resource(session, &address, options)
            && response.status == 200
            && let Some(image) = decode_web_image(&response.body)
        {
            let retained: usize = resources.images.iter().map(|entry| entry.image.pixels.len()).sum();
            if retained.saturating_add(image.pixels.len()) > 128 * 1024 * 1024 { return; }
            resources.images.push(PageImage {
                node,
                image,
            });
        }

        for child in dom.children(node) {
            walk(dom, child, document_url, options, session, resources);
        }
    }

    let mut resources = PageResources::default();
    walk(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        session,
        &mut resources,
    );
    let styles = amazing_css::compute(dom, &[amazing_css::parse_stylesheet(css)]);
    let mut retained: usize = resources.images.iter().map(|image| image.image.pixels.len()).sum();
    let mut pending = vec![amazing_dom::NodeIdx(0)];
    let mut attempted_urls = std::collections::HashSet::new();
    while let Some(node) = pending.pop() {
        pending.extend(dom.children(node));
        if attempted_urls.len() >= MAX_PAGE_IMAGES { break; }
        let Some(style) = styles.get(node) else { continue; };
        for layer in amazing_paint::background::BackgroundPlan::from_style(style).layers {
            let Some(url) = layer.url else { continue; };
            if attempted_urls.len() >= MAX_PAGE_IMAGES { break; }
            if !attempted_urls.insert(url.clone()) { continue; }
            let Some(address) = resolve_resource_address(document_url, &url) else { continue; };
            let Ok(response) = fetch_resource(session, &address, options) else { continue; };
            if response.status != 200 { continue; }
            let Some(image) = decode_web_image(&response.body) else { continue; };
            if retained.saturating_add(image.pixels.len()) > 128 * 1024 * 1024 { continue; }
            retained += image.pixels.len();
            resources.backgrounds.push((url,image));
        }
    }
    resources
}

fn decode_web_image(bytes: &[u8]) -> Option<amazing_media::Image> {
    amazing_image::decode(bytes, amazing_image::DecodeLimits::default()).ok()
}

fn document_title(dom: &amazing_dom::Dom) -> Option<String> {
    fn find(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx) -> Option<String> {
        if dom.name(node) == Some("title") {
            let mut title = String::new();
            for child in dom.children(node) {
                if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                    title.push_str(data);
                }
            }
            let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
            return (!normalized.is_empty()).then_some(normalized);
        }
        dom.children(node).find_map(|child| find(dom, child))
    }
    find(dom, amazing_dom::NodeIdx(0))
}

fn load_favicon(
    dom: &amazing_dom::Dom,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
) -> Option<Arc<RenderImage>> {
    fn explicit_icon_href(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
    ) -> Option<String> {
        if dom.name(node) == Some("link")
            && let Some(attrs) = dom.attrs_of(node)
        {
            let is_icon = attrs
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("rel"))
                .is_some_and(|(_, value)| {
                    value
                        .split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("icon"))
                });
            if is_icon
                && let Some((_, href)) = attrs
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("href"))
            {
                return Some(href.clone());
            }
        }
        dom.children(node)
            .find_map(|child| explicit_icon_href(dom, child))
    }

    let explicit = explicit_icon_href(dom, amazing_dom::NodeIdx(0));
    let address = explicit
        .as_deref()
        .and_then(|href| resolve_resource_address(document_url, href))
        .or_else(|| resolve_resource_address(document_url, "/favicon.ico"))?;
    let response = fetch_resource(session, &address, options).ok()?;
    if response.status != 200 {
        return None;
    }
    let image = decode_web_image(&response.body)?;
    Some(Arc::new(rgba_to_render_image(
        image.width,
        image.height,
        image.pixels,
    )))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

mod page_font_loader {
//! Caller-owned font resource loading. The provider supplies source ordering and
//! format filtering; this adapter supplies stylesheet-relative URLs and bytes.

use amazing_fetch::{
    HttpUrl, Origin,
    service::{ClientContext, FontRequest, NetworkService, ServiceEvent},
};
use amazing_net::dispatch::DispatchLimits;
use amazing_text::{FeatureSetting, TextError};
use amazing_text_provider::{
    FontDescriptorRange, FontDisplay, FontFaceDescriptor, FontFaceLoader, FontFaceSource,
    FontFetchRequest, FontStyleDescriptor, UnicodeRange,
};
use std::time::{Duration, Instant};

pub(super) fn expand_font_sheets(
    sheets: &[(String, String)],
    warnings: &mut Vec<String>,
) -> Vec<(String, String)> {
    fn visit(
        source: &str,
        url: &str,
        depth: usize,
        out: &mut Vec<(String, String)>,
        seen: &mut std::collections::HashSet<String>,
        warnings: &mut Vec<String>,
    ) {
        if depth >= 8 || out.len() >= 64 {
            warnings.push(format!("font stylesheet import limit at {url}"));
            return;
        }
        if !seen.insert(url.to_owned()) {
            return;
        }
        for imported in import_urls(source) {
            let resolved = super::resolve_font_url(url, &imported);
            if seen.contains(&resolved) {
                continue;
            }
            match super::load_resource(&resolved, 16 * 1024 * 1024) {
                Ok(bytes) => visit(
                    &String::from_utf8_lossy(&bytes),
                    &resolved,
                    depth + 1,
                    out,
                    seen,
                    warnings,
                ),
                Err(error) => warnings.push(format!("font stylesheet {resolved}: {error}")),
            }
        }
        out.push((source.to_owned(), url.to_owned()));
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (source, url) in sheets {
        // Each top-level sheet is a distinct cascade entry, including inline
        // sheets that share the document URL. Cycle detection is per import
        // chain, not across those independent entries.
        seen.clear();
        visit(source, url, 0, &mut out, &mut seen, warnings);
    }
    out
}

fn import_urls(source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut quote = 0u8;
    while index < bytes.len() {
        let byte = bytes[index];
        if quote != 0 {
            if byte == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
        } else if depth == 0
            && bytes[index..]
                .get(..7)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"@import"))
        {
            let rest = source[index + 7..].trim_start();
            let rest = rest
                .strip_prefix("url(")
                .or_else(|| rest.strip_prefix("URL("));
            if let Some(rest) = rest {
                let value = rest.trim_start().trim_start_matches(['\'', '"']);
                if let Some(end) = value.find(['\'', '"', ')']) {
                    result.push(value[..end].to_owned());
                }
            } else {
                let rest = source[index + 7..].trim_start();
                if let Some(value) = rest.strip_prefix(['\'', '"'])
                    && let Some(end) = value.find(['\'', '"'])
                {
                    result.push(value[..end].to_owned());
                }
            }
            index += 7;
            continue;
        }
        index += 1;
    }
    result
}

pub(super) fn descriptor_from_css(
    face: amazing_css::FontFaceRule,
) -> Result<FontFaceDescriptor, String> {
    let weight = descriptor_range(&face.weight, 400, false)?;
    let stretch = descriptor_range(&face.stretch, 100, true)?;
    let style = match face.style.trim().to_ascii_lowercase().as_str() {
        "normal" => FontStyleDescriptor::Normal,
        "italic" => FontStyleDescriptor::Italic,
        value if value.starts_with("oblique") => {
            let angles = value[7..]
                .split_whitespace()
                .map(|part| part.strip_suffix("deg").unwrap_or(part).parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid font-style angle")?;
            FontStyleDescriptor::Oblique {
                min_degrees: *angles.first().unwrap_or(&14.0),
                max_degrees: *angles.get(1).unwrap_or(angles.first().unwrap_or(&14.0)),
            }
        }
        _ => return Err("invalid font-style".into()),
    };
    let unicode_ranges = face
        .unicode_range
        .as_deref()
        .map(parse_unicode_ranges)
        .transpose()?
        .unwrap_or_default();
    let display = match face
        .display
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "auto" => FontDisplay::Auto,
        "block" => FontDisplay::Block,
        "swap" => FontDisplay::Swap,
        "fallback" => FontDisplay::Fallback,
        "optional" => FontDisplay::Optional,
        _ => FontDisplay::Auto,
    };
    let percentage = |value: Option<&str>, default: Option<f32>| -> Result<Option<f32>, String> {
        match value.map(str::trim) {
            None | Some("normal") => Ok(default),
            Some(raw) => raw
                .strip_suffix('%')
                .ok_or("expected percentage".to_owned())?
                .trim()
                .parse::<f32>()
                .map(|n| Some(n / 100.0))
                .map_err(|_| "invalid percentage".to_owned()),
        }
    };
    let size_adjust = percentage(face.size_adjust.as_deref(), Some(1.0))?.unwrap_or(1.0);
    let sources = face
        .sources
        .into_iter()
        .map(|source| match source.local {
            Some(name) => FontFaceSource::Local(name),
            None => FontFaceSource::Url {
                url: source.url,
                formats: source.format.into_iter().collect(),
                technologies: source.technologies,
            },
        })
        .collect();
    Ok(FontFaceDescriptor {
        family: face.family,
        sources,
        unicode_ranges,
        weight,
        stretch,
        style,
        display,
        size_adjust,
        ascent_override: percentage(face.ascent_override.as_deref(), None)?,
        descent_override: percentage(face.descent_override.as_deref(), None)?,
        line_gap_override: percentage(face.line_gap_override.as_deref(), None)?,
        features: face
            .feature_settings
            .as_deref()
            .map(parse_features)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn descriptor_range(
    value: &str,
    default: u16,
    percentage: bool,
) -> Result<FontDescriptorRange, String> {
    let mut parts = value.split_whitespace().map(|part| {
        if part.eq_ignore_ascii_case("normal") {
            return Ok(default);
        }
        if part.eq_ignore_ascii_case("bold") && !percentage {
            return Ok(700);
        }
        let part = if percentage {
            part.strip_suffix('%').ok_or("expected percentage")?
        } else {
            part
        };
        part.parse::<u16>().map_err(|_| "invalid descriptor range")
    });
    let min = parts
        .next()
        .transpose()
        .map_err(str::to_owned)?
        .unwrap_or(default);
    let max = parts
        .next()
        .transpose()
        .map_err(str::to_owned)?
        .unwrap_or(min);
    if parts.next().is_some() || min < 1 || max < min || max > 1000 {
        return Err("invalid descriptor range".into());
    }
    Ok(FontDescriptorRange { min, max })
}

fn parse_unicode_ranges(value: &str) -> Result<Vec<UnicodeRange>, String> {
    value
        .split(',')
        .map(|part| {
            let raw = part
                .trim()
                .strip_prefix("U+")
                .or_else(|| part.trim().strip_prefix("u+"))
                .ok_or("invalid unicode-range")?;
            let (start, end) = if raw.contains('?') {
                let lower = raw.replace('?', "0");
                let upper = raw.replace('?', "F");
                (lower, upper)
            } else if let Some((lower, upper)) = raw.split_once('-') {
                (lower.to_owned(), upper.to_owned())
            } else {
                (raw.to_owned(), raw.to_owned())
            };
            let start = u32::from_str_radix(&start, 16).map_err(|_| "invalid unicode-range")?;
            let end = u32::from_str_radix(&end, 16).map_err(|_| "invalid unicode-range")?;
            if start > end || end > 0x10ffff {
                return Err("invalid unicode-range".into());
            }
            Ok(UnicodeRange { start, end })
        })
        .collect::<Result<_, _>>()
        .map_err(str::to_owned)
}

fn parse_features(value: &str) -> Result<Vec<FeatureSetting>, String> {
    if value.trim().eq_ignore_ascii_case("normal") {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| {
            let item = item.trim();
            let tag = item.get(1..5).ok_or("invalid feature tag")?;
            if !matches!(item.as_bytes().first(), Some(b'\'' | b'"'))
                || item.as_bytes().get(5) != item.as_bytes().first()
                || !tag.is_ascii()
            {
                return Err("invalid feature tag");
            }
            let rest = item[6..].trim();
            let value = if rest.is_empty() || rest.eq_ignore_ascii_case("on") {
                1
            } else if rest.eq_ignore_ascii_case("off") {
                0
            } else {
                rest.parse::<u32>().map_err(|_| "invalid feature value")?
            };
            Ok(FeatureSetting {
                tag: tag
                    .as_bytes()
                    .try_into()
                    .map_err(|_| "invalid feature tag")?,
                value,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(str::to_owned)
}

pub(super) struct PageFontLoader<'a> {
    pub stylesheet_url: &'a str,
    pub document_url: &'a str,
    pub warnings: Vec<String>,
}

impl PageFontLoader<'_> {
    fn read_url(&mut self, source: &str, max_bytes: usize) -> Option<Vec<u8>> {
        let resolved = super::resolve_font_url(self.stylesheet_url, source);
        let result = if resolved.starts_with("http://") || resolved.starts_with("https://") {
            self.fetch_cors(source, max_bytes)
        } else {
            super::load_resource(&resolved, max_bytes)
        };
        match result {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                self.warnings
                    .push(format!("font {source:?} ({resolved}): {error}"));
                None
            }
        }
    }

    fn fetch_cors(&self, source: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let stylesheet =
            HttpUrl::parse(self.stylesheet_url).map_err(|e| format!("stylesheet URL: {e:?}"))?;
        let origin = HttpUrl::parse(self.document_url)
            .map(|url| url.origin().clone())
            .unwrap_or_else(|_| Origin::opaque());
        let request = FontRequest::new(source, &stylesheet, origin, max_bytes)
            .map_err(|e| format!("font request: {e:?}"))?;
        let mut service = NetworkService::new(DispatchLimits::default());
        let id = service
            .start_font(request, ClientContext::default())
            .map_err(|e| format!("font start: {e:?}"))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut network_error = None;
        loop {
            for event in service.step(64, Instant::now()) {
                if let ServiceEvent::Network { id: event_id, event } = &event
                    && *event_id == id
                    && matches!(event, amazing_net::dispatch::NetEvent::Error { .. })
                {
                    network_error = Some(format!("{event:?}"));
                }
                if let ServiceEvent::Finished {
                    id: finished,
                    result,
                } = event
                    && finished == id
                {
                    let mut response = result.map_err(|e| format!("font fetch: {e:?}; {network_error:?}"))?;
                    if !(200..300).contains(&response.status) {
                        return Err(format!("HTTP {}", response.status));
                    }
                    let bytes = response
                        .body
                        .bytes()
                        .map_err(|e| format!("font body: {e:?}"))?;
                    return (bytes.len() <= max_bytes)
                        .then_some(bytes)
                        .ok_or_else(|| "font size limit".to_owned());
                }
            }
            if Instant::now() >= deadline {
                service.abort(id);
                return Err("font fetch timeout".to_owned());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl FontFaceLoader for PageFontLoader<'_> {
    fn load_local(&mut self, name: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, TextError> {
        let file = match name.trim().to_ascii_lowercase().as_str() {
            "arial" => Some(r"C:\Windows\Fonts\arial.ttf".to_owned()),
            "times new roman" | "times" => Some(r"C:\Windows\Fonts\times.ttf".to_owned()),
            "segoe ui" => Some(r"C:\Windows\Fonts\segoeui.ttf".to_owned()),
            "consolas" => Some(r"C:\Windows\Fonts\consola.ttf".to_owned()),
            "csshworientationtest" => std::env::var_os("AMAZING_WPT_ROOT").map(|root| {
                std::path::Path::new(&root)
                    .join("fonts/adobe-fonts/CSSHWOrientationTest.otf")
                    .to_string_lossy()
                    .into_owned()
            }),
            _ => None,
        };
        Ok(file.and_then(|path| super::load_resource(&path, max_bytes).ok()))
    }

    fn load_url(&mut self, request: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> {
        Ok(self.read_url(request.url, request.max_bytes))
    }
}

}

const MAX_PAGE_FONT_BYTES: usize = 8 * 1024 * 1024;

fn load_resource(url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    if url.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
        let parsed = amazing_net::ParsedUrl::parse(url).map_err(|error| format!("{error:?}"))?;
        let data = amazing_fetch::process_data_url(&parsed)
            .ok_or_else(|| "invalid data URL".to_owned())?;
        return (data.body.len() <= max_bytes)
            .then_some(data.body)
            .ok_or_else(|| "resource size limit".to_owned());
    }
    let options = amazing_net::FetchOptions {
        max_response_bytes: max_bytes,
        ..amazing_net::FetchOptions::default()
    };
    amazing_net::fetch(url, &options)
        .map(|response| response.body)
        .map_err(|error| error.to_string())
}

fn resolve_font_url(page: &str, source: &str) -> String {
    resolve_resource_address(page, source).unwrap_or_else(|| source.to_owned())
}

fn register_system_text_faces(provider: &mut amazing_text_provider::TextProvider) {
    use amazing_text::FontSlant;
    use amazing_text_provider::{FontFormat, WebFont};

    for (path, family) in [
        (r"C:\Windows\Fonts\times.ttf", "serif"),
        (r"C:\Windows\Fonts\times.ttf", "Times"),
        (r"C:\Windows\Fonts\times.ttf", "Times New Roman"),
        (r"C:\Windows\Fonts\arial.ttf", "sans-serif"),
        (r"C:\Windows\Fonts\arial.ttf", "Arial"),
        (r"C:\Windows\Fonts\segoeui.ttf", "system-ui"),
        (r"C:\Windows\Fonts\consola.ttf", "monospace"),
    ] {
        let options = amazing_net::FetchOptions {
            max_response_bytes: MAX_PAGE_FONT_BYTES,
            ..amazing_net::FetchOptions::default()
        };
        let Ok(response) = amazing_net::fetch(path, &options) else {
            continue;
        };
        let _ = provider.register_web_font(WebFont {
            family: family.into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: response.body,
        });
    }

    // The pinned WPT render runner sets this root for its child process. The
    // CSSTest faces are installed fonts in upstream tests, not @font-face URLs.
    // Register only the known fixture files, through the same bounded provider
    // seam used for page fonts, so their family names are available to CSS.
    if let Some(root) = std::env::var_os("AMAZING_WPT_ROOT") {
        for (file, family, weight, slant) in [
            ("csstest-verify.ttf", "CSSTest Verify", 400, FontSlant::Normal),
            ("csstest-fallback.ttf", "CSSTest Fallback", 400, FontSlant::Normal),
            ("csstest-ascii.ttf", "CSSTest ASCII", 400, FontSlant::Normal),
            ("csstest-familyname.ttf", "CSSTest FamilyName", 400, FontSlant::Normal),
            ("csstest-familyname-bold.ttf", "CSSTest FamilyName", 700, FontSlant::Normal),
            ("csstest-basic-regular.ttf", "CSSTest Basic", 400, FontSlant::Normal),
            ("csstest-basic-bold.ttf", "CSSTest Basic", 700, FontSlant::Normal),
            ("csstest-basic-italic.ttf", "CSSTest Basic", 400, FontSlant::Italic),
            ("csstest-basic-bolditalic.ttf", "CSSTest Basic", 700, FontSlant::Italic),
        ] {
            let path = std::path::Path::new(&root).join("fonts").join("CSSTest").join(file);
            let options = amazing_net::FetchOptions {
                max_response_bytes: MAX_PAGE_FONT_BYTES,
                ..amazing_net::FetchOptions::default()
            };
            let Ok(response) = amazing_net::fetch(&path.to_string_lossy(), &options) else {
                continue;
            };
            let _ = provider.register_web_font(WebFont {
                family: family.into(),
                weight,
                stretch: 100,
                slant,
                format: FontFormat::OpenType,
                bytes: response.body,
            });
        }
    }
}

fn load_page_text(css: &str, page: &str, sheets: &[(String, String)]) -> TextProvider {
    let mut provider = TextProvider::new();
    register_system_text_faces(&mut provider);
    let initial = if sheets.is_empty() {
        vec![(css.to_owned(), page.to_owned())]
    } else {
        sheets.to_vec()
    };
    let mut warnings = Vec::new();
    let expanded = page_font_loader::expand_font_sheets(&initial, &mut warnings);
    for warning in warnings {
        eprintln!("amazing shell: {warning}");
    }
    for (source, stylesheet_url) in &expanded {
      for face in amazing_css::parse_font_faces(source) {
        let family = face.family.clone();
        let descriptor = match page_font_loader::descriptor_from_css(face) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("amazing shell: font-face {family:?}: {error}");
                continue;
            }
        };
        let mut loader = page_font_loader::PageFontLoader {
            stylesheet_url,
            document_url: page,
            warnings: Vec::new(),
        };
        if let Err(error) = provider.begin_font_load(descriptor.display) {
            eprintln!("amazing shell: font-face {family:?}: {error:?}");
            continue;
        }
        let result = provider.register_font_face(descriptor, &mut loader);
        provider.end_font_load();
        match result {
            Ok(Some(_)) => {},
            Ok(None) => eprintln!("amazing shell: font-face {family:?}: no usable source"),
            Err(error) => eprintln!("amazing shell: font-face {family:?}: {error:?}"),
        }
        for warning in loader.warnings {
            eprintln!("amazing shell: {warning}");
        }
      }
    }
    provider
}

fn surface_to_render_image(surface: Surface) -> RenderImage {
    rgba_to_render_image(surface.width, surface.height, surface.pixels)
}

fn rgba_to_render_image(width: u32, height: u32, mut bgra: Vec<u8>) -> RenderImage {
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = RgbaImage::from_raw(width, height, bgra)
        .expect("Amazing surface dimensions match its pixel buffer");
    RenderImage::new([Frame::new(image)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[test]
    fn engine_demo_produces_an_opaque_non_white_surface() {
        let surface = render_demo_surface();
        assert_eq!((surface.width, surface.height), (960, 570));
        assert!(surface.pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(
            surface
                .pixels
                .chunks_exact(4)
                .any(|pixel| pixel[0..3] != [255, 255, 255])
        );
    }

    #[test]
    fn gpui_image_conversion_preserves_dimensions() {
        let viewport = EngineViewport::demo();
        assert_eq!((viewport.width, viewport.height), (960, 570));
        assert_eq!(viewport.image.frame_count(), 1);
        let size = viewport.image.size(0);
        assert_eq!((u32::from(size.width), u32::from(size.height)), (960, 570));
    }

    #[test]
    fn retained_live_viewport_updates_only_on_damage() {
        let viewport = EngineViewport::demo();
        let scene = viewport.compositor.as_ref().expect("live page frame");
        assert!(!scene.borrow().frame().damage.is_empty());
        assert!(!viewport.update_compositor_properties(|_| Ok(())).unwrap());
        assert!(viewport.update_compositor_properties(|compositor| {
            compositor.invalidate_item(0);
            Ok(())
        }).unwrap());
        assert!(!viewport.update_compositor_properties(|_| Ok(())).unwrap());
    }

    #[test]
    fn navigation_document_escapes_address_markup() {
        assert_eq!(
            escape_html("https://example.test/?a=<b>&q=\"x\""),
            "https://example.test/?a=&lt;b&gt;&amp;q=&quot;x&quot;"
        );
        let viewport = EngineViewport::error("https://example.test/", "unsupported scheme");
        assert_eq!((viewport.width, viewport.height), (960, 570));
    }

    #[test]
    fn decoded_video_demo_produces_a_gpui_surface() {
        let viewport = EngineViewport::video_demo().expect("video demo");
        assert_eq!((viewport.width, viewport.height), (320, 180));
        assert_eq!(viewport.image.frame_count(), 1);
        assert_eq!(EngineViewport::video_demo_frames().unwrap().len(), 6);
    }

    #[test]
    fn inline_style_collection_feeds_response_rendering() {
        let dom =
            amazing_html::parse("<style>body { background: #010203; }</style><p>response</p>");
        let mut css = String::new();
        collect_inline_styles(&dom, amazing_dom::NodeIdx(0), &mut css);
        assert!(css.contains("background: #010203"));
        let viewport = EngineViewport::from_dom(&dom, &css);
        assert_eq!((viewport.width, viewport.height), (960, 570));
    }

    #[test]
    fn external_stylesheets_resolve_and_keep_document_order() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut stream = (0..100)
                .find_map(|_| match listener.accept() {
                    Ok((stream, _)) => Some(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    },
                    Err(error) => panic!("stylesheet fixture accept: {error}"),
                })
                .expect("stylesheet request timed out");
            stream.set_nonblocking(false).unwrap();
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).starts_with(
                "GET /theme.css HTTP/1.1\r\n"
            ));
            let body = b".external { color: #123456; }";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/css\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let dom = amazing_html::parse(
            "<style>.first { color: red; }</style>\
             <link rel=stylesheet href=../theme.css>\
             <style>.last { color: blue; }</style>",
        );
        let (css, font_sheets) = load_document_styles(
            &dom,
            &format!("http://127.0.0.1:{port}/pages/index.html"),
            &FetchOptions::default(),
            None,
        );
        server.join().unwrap();
        assert_eq!(font_sheets.len(), 3);

        let first = css.find(".first").unwrap();
        let external = css.find(".external").unwrap();
        let last = css.find(".last").unwrap();
        assert!(first < external && external < last);
    }

    #[test]
    fn web_images_feed_intrinsic_layout_and_paint_resources() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut stream = (0..100)
                .find_map(|_| match listener.accept() {
                    Ok((stream, _)) => Some(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    },
                    Err(error) => panic!("image fixture accept: {error}"),
                })
                .expect("image request timed out");
            stream.set_nonblocking(false).unwrap();
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..count])
                    .starts_with("GET /assets/pixel.ppm HTTP/1.1\r\n")
            );
            let body = b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: image/x-portable-pixmap\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let dom = amazing_html::parse("<img src=../assets/pixel.ppm width=8>");
        let resources = load_page_resources(
            &dom,
            "",
            &format!("http://127.0.0.1:{port}/pages/index.html"),
            &FetchOptions::default(),
            None,
        );
        server.join().unwrap();
        assert_eq!(resources.images.len(), 1);
        let natural = amazing_layout::ReplacedElementProvider::intrinsic_size(
            &resources, resources.images[0].node,
        ).unwrap();
        assert_eq!(natural.width, 2.0);
        assert_eq!(natural.height, 1.0);
        assert_eq!(resources.images[0].image.pixels[0..4], [255, 0, 0, 255]);
    }

    #[test]
    fn document_metadata_loads_title_and_resolved_favicon() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut stream = (0..100)
                .find_map(|_| match listener.accept() {
                    Ok((stream, _)) => Some(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    },
                    Err(error) => panic!("favicon fixture accept: {error}"),
                })
                .expect("favicon request timed out");
            stream.set_nonblocking(false).unwrap();
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..count])
                    .starts_with("GET /assets/site.ppm HTTP/1.1\r\n")
            );
            let body = b"P6\n1 1\n255\n\x12\x34\x56";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: image/x-portable-pixmap\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let dom = amazing_html::parse(
            "<title>  Example   Page </title><link rel=icon href=../assets/site.ppm>",
        );
        assert_eq!(document_title(&dom).as_deref(), Some("Example Page"));
        let icon = load_favicon(
            &dom,
            &format!("http://127.0.0.1:{port}/pages/index.html"),
            &FetchOptions::default(),
            None,
        );
        server.join().unwrap();
        assert!(icon.is_some());
    }

    #[test]
    fn layout_exposes_resolved_link_hit_regions() {
        let dom = amazing_html::parse("<p><a href=../next.html>Next page</a></p>");
        let viewport = EngineViewport::from_dom_with_resources(
            &dom,
            "a { color: blue; }",
            &PageResources::default(),
            Some("https://example.test/docs/current.html"),
        );
        let region = viewport.links.first().expect("link region");
        assert_eq!(region.target, "https://example.test/next.html");
        assert_eq!(
            viewport
                .link_hit_at(
                    region.x + region.width / 2.0,
                    region.y + region.height / 2.0
                )
                .map(|(_, target)| target),
            Some("https://example.test/next.html"),
        );
    }

    #[test]
    fn positioned_links_remain_interactive() {
        let dom = amazing_html::parse("<a href=/next style='position:absolute;left:20px;top:30px;width:80px;height:20px'>Next</a>");
        let viewport = EngineViewport::from_dom_with_resources(
            &dom,
            "",
            &PageResources::default(),
            Some("https://example.test/current"),
        );
        let region = viewport.links.first().expect("positioned link region");
        assert!(region.x >= 20.0, "region x={}", region.x);
        assert!(region.y >= 30.0, "region y={}", region.y);
        assert_eq!(
            viewport
                .link_hit_at(
                    region.x + region.width / 2.0,
                    region.y + region.height / 2.0,
                )
                .map(|(_, target)| target),
            Some("https://example.test/next"),
        );
    }

    #[test]
    fn get_form_controls_build_encoded_navigation_target() {
        let dom = amazing_html::parse(
            "<form action=../search method=get>\
               <input name=q value=old>\
               <input type=submit name=source value=amazing>\
             </form>",
        );
        let viewport = EngineViewport::from_dom_with_resources(
            &dom,
            "input { display: block; width: 300px; height: 40px; }",
            &PageResources::default(),
            Some("https://example.test/docs/form.html"),
        );
        let text = viewport
            .controls
            .iter()
            .position(|control| control.kind == ControlKind::Text)
            .expect("text control");
        let submit = viewport
            .controls
            .iter()
            .position(|control| control.kind == ControlKind::Submit)
            .expect("submit control");
        let text_request = viewport.form_request(text, Some("rust browser")).unwrap();
        assert_eq!(text_request.target, "https://example.test/search?q=rust+browser");
        assert_eq!(text_request.method, "GET");
        assert!(text_request.body.is_empty());
        let submit_request = viewport.form_request(submit, None).unwrap();
        assert_eq!(
            submit_request.target,
            "https://example.test/search?q=old&source=amazing"
        );
    }

    #[test]
    fn post_form_controls_build_request_body() {
        let dom = amazing_html::parse(
            "<form action=/submit method=post><input name=q value=old></form>",
        );
        let viewport = EngineViewport::from_dom_with_resources(
            &dom,
            "input { display: block; width: 300px; height: 40px; }",
            &PageResources::default(),
            Some("https://example.test/form"),
        );
        let request = viewport.form_request(0, Some("rust browser")).unwrap();
        assert_eq!(request.target, "https://example.test/submit");
        assert_eq!(request.method, "POST");
        assert_eq!(request.body, b"q=rust+browser");
    }

    #[test]
    fn post_navigation_loads_html_response_through_session() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 2048];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                if request.ends_with(b"q=rust+browser") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("POST /submit HTTP/1.1\r\n"));
            assert!(request.contains("Content-Type: application/x-www-form-urlencoded\r\n"));
            let body = b"<title>POST complete</title><h1>POST complete</h1>";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        let session = Rc::new(RefCell::new(BrowserSession::ephemeral()));
        let page = EngineViewport::load_request(
            &format!("http://127.0.0.1:{port}/submit"),
            "POST",
            b"q=rust+browser".to_vec(),
            session,
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(page.title.as_deref(), Some("POST complete"));
    }

    #[test]
    fn inline_and_external_scripts_mutate_dom_before_paint() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(
                String::from_utf8_lossy(&request[..count])
                    .starts_with("GET /assets/page.js HTTP/1.1\r\n")
            );
            let body = b"document.querySelector('#message').textContent = 'External ran'; document.title = 'Scripted page';";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let dom = amazing_html::parse(
            "<title>Initial</title><p id=message>Before</p>\
             <script>document.title = 'Inline ran';</script>\
             <script src=../assets/page.js></script>",
        );
        let document_url = format!("http://127.0.0.1:{port}/pages/index.html");
        let options = FetchOptions::default();
        let scripts = collect_document_scripts(&dom, &document_url, &options, None);
        let runtime = PageRuntime::new(
            dom,
            document_url,
            options,
            scripts,
            Rc::new(RefCell::new(BrowserSession::ephemeral())),
        );
        server.join().unwrap();
        let host = runtime.host.borrow();
        let dom = &host.dom;
        assert_eq!(document_title(dom).as_deref(), Some("Scripted page"));

        fn find_id(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx) -> Option<amazing_dom::NodeIdx> {
            if dom.attrs_of(node).is_some_and(|attrs| {
                attrs
                    .iter()
                    .any(|(name, value)| name == "id" && value == "message")
            }) {
                return Some(node);
            }
            dom.children(node).find_map(|child| find_id(dom, child))
        }
        let message = find_id(dom, amazing_dom::NodeIdx(0)).unwrap();
        let text = dom
            .children(message)
            .find_map(|child| match &dom.get(child).kind {
                amazing_dom::NodeKind::Text { data } => Some(data.as_str()),
                _ => None,
            });
        assert_eq!(text, Some("External ran"));
    }

    #[test]
    fn retained_runtime_dispatches_clicks_and_future_timers() {
        let dom = amazing_html::parse(
            "<title>Initial</title>\
             <a id=action href=next.html>Before click</a>\
             <p id=timer>Waiting</p>\
             <script>\
               document.querySelector('#action').addEventListener('click', function(event) {\
                 event.preventDefault();\
                 document.querySelector('#action').textContent = 'JS click handled';\
                 document.title = 'Event handled';\
                 history.pushState({id: 1}, '', 'virtual.html');\
               });\
               setTimeout(function() {\
                 document.querySelector('#timer').textContent = 'Timer fired';\
               }, 20);\
             </script>",
        );
        let document_url = "https://example.test/page.html".to_string();
        let options = FetchOptions::default();
        let scripts = collect_document_scripts(&dom, &document_url, &options, None);
        let mut runtime = PageRuntime::new(
            dom,
            document_url,
            options,
            scripts,
            Rc::new(RefCell::new(BrowserSession::ephemeral())),
        );
        let initial = runtime.snapshot(false);
        let node = initial.viewport.links.first().unwrap().node;
        let clicked = runtime.dispatch_click(node);
        assert!(clicked.default_prevented);
        assert_eq!(clicked.title.as_deref(), Some("Event handled"));
        assert!(clicked.navigation.is_none());
        assert_eq!(
            clicked.history_update.as_ref().map(|(url, replace)| (url.as_str(), *replace)),
            Some(("virtual.html", false))
        );

        std::thread::sleep(Duration::from_millis(30));
        assert!(runtime.pump().is_some());
        let host = runtime.host.borrow();
        fn text_by_id(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, id: &str) -> Option<String> {
            if dom.attrs_of(node).is_some_and(|attrs| {
                attrs
                    .iter()
                    .any(|(name, value)| name == "id" && value == id)
            }) {
                return dom.children(node).find_map(|child| match &dom.get(child).kind {
                    amazing_dom::NodeKind::Text { data } => Some(data.clone()),
                    _ => None,
                });
            }
            dom.children(node)
                .find_map(|child| text_by_id(dom, child, id))
        }
        assert_eq!(
            text_by_id(&host.dom, amazing_dom::NodeIdx(0), "timer").as_deref(),
            Some("Timer fired")
        );
    }


    #[test]
    fn tall_documents_keep_their_painted_height() {
        let viewport = EngineViewport::from_document(
            "<main><p>top</p><div></div><p>bottom</p></main>",
            "main { padding: 20px; } div { display: block; height: 900px; }",
        );
        assert_eq!(viewport.width, 960);
        assert!(viewport.height > 900, "height was {}", viewport.height);
    }
}
