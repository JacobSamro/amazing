use std::sync::Arc;
use std::{cell::RefCell, rc::{Rc, Weak}};

use amazing_net::{FetchOptions, Url, fetch, resolve};
use amazing_paint::Surface;
use amazing_text_provider::TextProvider;
use gpui::RenderImage;
use image::{Frame, RgbaImage};

use crate::dom_runtime::ShellDomHost;
use crate::engine_compositor::EngineCompositor;
use crate::compositor_presenter::GpuiFrame;
use crate::session_state::BrowserSession;
use crate::viewport_metrics::ViewportMetrics;

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
    density: f32,
}

#[derive(Clone)]
struct DeferredImage {
    node: amazing_dom::NodeIdx,
    address: String,
    density: f32,
}

#[derive(Clone, Default)]
struct PageResources {
    images: Vec<PageImage>,
    deferred_images: Vec<DeferredImage>,
    backgrounds: Vec<(String, amazing_media::Image)>,
    metrics: ViewportMetrics,
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
                width: entry.image.width as f32 / entry.density,
                height: entry.image.height as f32 / entry.density,
            })
            .or_else(|| self.deferred_images.iter().any(|entry| entry.node == node)
                .then_some(amazing_layout::IntrinsicSize { width: 300.0, height: 150.0 }))
    }
}

impl amazing_paint::ImageProvider for PageResources {
    fn device_scale(&self) -> f32 { self.metrics.device_scale }
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
      <p class="proof">LIVE ENGINE SURFACE  ·  NATIVE SOFTWARE RASTER</p>
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
    pub metrics: ViewportMetrics,
    static_document: Option<Rc<StaticDocument>>,
    pub compositor: Option<Rc<RefCell<EngineCompositor>>>,
    scroll_controller: Rc<RefCell<amazing_compositor::ScrollController>>,
    scroll_frame: Rc<RefCell<Option<GpuiFrame>>>,
    page: Option<Rc<RetainedPage>>,
    pub width: u32,
    pub height: u32,
    links: Arc<Vec<LinkRegion>>,
    controls: Arc<Vec<ControlRegion>>,
}

#[derive(Clone)]
pub struct ViewportIdentity {
    pub metrics: ViewportMetrics,
    static_document: Option<Weak<StaticDocument>>,
    page: Option<Weak<RetainedPage>>,
}

impl ViewportIdentity {
    pub fn is_document(&self) -> bool { self.static_document.is_some() || self.page.is_some() }
}

struct StaticDocument {
    acquisition: Option<Rc<RefCell<crate::navigation_acquisition::Replay>>>,
    dom: amazing_dom::Dom,
    css: String,
    font_sheets: Vec<(String, String)>,
    document_url: Option<String>,
    options: FetchOptions,
    session: Option<Rc<RefCell<BrowserSession>>>,
}

struct RetainedPage {
    root: amazing_layout::LayoutBox,
    text: TextProvider,
    styles: amazing_css::StyledDom,
    resources: PageResources,
    /// A11Y-11: the accessibility tree for this retained page, built once
    /// from the same `dom`/`styles`/`root` triple used for layout and paint
    /// (Stage 1 has no incremental invalidation, so a fresh
    /// `amazing_a11y::build` per navigation/reload is the same rebuild-on-
    /// change model `amazing-a11y`'s own `event::rebuild_and_diff` already
    /// uses). `EngineViewport::accessibility_tree()` exposes it read-only.
    accessibility: amazing_a11y::AxTree,
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
    node: u32,
    generation: u64,
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

impl LoadedPage {
    pub fn acquisition_replay(&self) -> Option<Rc<RefCell<crate::navigation_acquisition::Replay>>> {
        self.runtime.as_ref().and_then(|runtime| runtime.acquisition.clone())
            .or_else(|| self.viewport.static_document.as_ref().and_then(|document| document.acquisition.clone()))
    }
}

/// One original UI-owned DOM survives all initial acquisition passes.
pub struct PreparedDocument {
    dom: amazing_dom::Dom,
    final_address: String,
    options: FetchOptions,
}
pub struct PreparedPage {
    scripts: Vec<String>,
    viewport: Option<EngineViewport>,
    title: Option<String>,
    favicon: Option<Arc<RenderImage>>,
    css: String,
    font_sheets: Vec<(String, String)>,
}
impl PreparedDocument {
    pub fn new(response: &amazing_net::Response, options: FetchOptions) -> Result<Self, String> {
        let html = response.text().ok_or("response body was not decodable text")?;
        Ok(Self { dom: amazing_html::parse(&html), final_address: response.url.clone(), options })
    }
    pub fn discover(&self, session: Rc<RefCell<BrowserSession>>, metrics: ViewportMetrics) -> Result<PreparedPage, String> {
        let dom = &self.dom;
        let address = &self.final_address;
        let scripts = collect_document_scripts(dom, address, &self.options, Some(&session));
        if !scripts.is_empty() {
            // Preserve existing author ordering: resources are selected from the post-script DOM.
            // The retained runtime's first snapshot discovers them without re-evaluating script.
            return Ok(PreparedPage { scripts, viewport: None, title: None, favicon: None,
                css: String::new(), font_sheets: Vec::new() });
        }
        // Do not select pre-script CSS/images while an external classic script is still pending.
        if crate::navigation_acquisition::has_missing() { return Err("script acquisition pending".into()); }
        let (css, font_sheets) = load_document_styles(dom, address, &self.options, Some(&session));
        let resources = load_page_resources_with_metrics(dom, &css, address, &self.options, Some(&session), metrics);
        let title = document_title(dom);
        let favicon = load_favicon(dom, address, &self.options, Some(&session));
        let viewport = EngineViewport::from_dom_with_style_sources_and_metrics(
            dom, &css, resources, Some(address), &font_sheets, metrics)?;
        Ok(PreparedPage { scripts, viewport: Some(viewport), title, favicon, css, font_sheets })
    }
    pub fn finish(self, prepared: PreparedPage, session: Rc<RefCell<BrowserSession>>, metrics: ViewportMetrics) -> Result<LoadedPage, String> {
        if prepared.scripts.is_empty() {
            let mut viewport = prepared.viewport.ok_or("missing prepared static viewport")?;
            viewport.static_document = Some(Rc::new(StaticDocument {
                acquisition: crate::navigation_acquisition::current_replay(),
                dom: self.dom, css: prepared.css, font_sheets: prepared.font_sheets,
                document_url: Some(self.final_address.clone()), options: self.options, session: Some(session),
            }));
            return Ok(LoadedPage { viewport, final_address: self.final_address,
                title: prepared.title, favicon: prepared.favicon, runtime: None });
        }
        let mut runtime = PageRuntime::new_with_metrics(self.dom, self.final_address.clone(),
            self.options, prepared.scripts, session, metrics);
        let update = runtime.snapshot(false);
        Ok(LoadedPage { viewport: update.viewport.ok_or_else(|| update.render_failure.unwrap_or_else(|| "initial runtime render unavailable".into()))?,
            final_address: self.final_address, title: update.title, favicon: update.favicon, runtime: Some(runtime) })
    }
}

pub struct RuntimeUpdate {
    pub viewport: Option<EngineViewport>,
    pub render_failure: Option<String>,
    pub metrics: ViewportMetrics,
    pub title: Option<String>,
    pub favicon: Option<Arc<RenderImage>>,
    pub navigation: Option<String>,
    pub history_delta: Option<i32>,
    pub history_update: Option<(String, bool)>,
    pub default_prevented: bool,
}

pub struct PageRuntime {
    acquisition: Option<Rc<RefCell<crate::navigation_acquisition::Replay>>>,
    metrics: ViewportMetrics,
    host: Rc<RefCell<ShellDomHost>>,
    interpreter: amazing_js::Interpreter,
    document_url: String,
    options: FetchOptions,
    session: Rc<RefCell<BrowserSession>>,
}

impl PageRuntime {
    pub fn font_display_trace(&self) -> String { crate::navigation_acquisition::font_trace(self.acquisition.as_ref()) }

    #[cfg(test)]
    pub(crate) fn navigation_runtime_identity(&self) -> usize { Rc::as_ptr(&self.host) as usize }

    #[cfg(test)]
    pub(super) fn failure_fixture() -> Self {
        Self::new_with_metrics(amazing_html::parse(
            "<style>html,body{margin:0} #target{width:40px;height:40px;border:4px solid red} p{height:1000px}</style><div id=target></div><a href=next>Link</a><p>Scroll</p>"),
            "https://example.test/failure".into(), FetchOptions::default(), Vec::new(),
            Rc::new(RefCell::new(BrowserSession::ephemeral())),
            ViewportMetrics::new(120.0, 100.0, 2.0).unwrap())
    }

    #[cfg(test)]
    pub(super) fn metrics_for_fixture(&self) -> ViewportMetrics { self.metrics }

    #[cfg(test)]
    pub(super) fn fixture_update(&mut self, source: &str) -> RuntimeUpdate {
        self.interpreter.run_script(source).expect("actual fixture script");
        self.snapshot(false)
    }

    /// Conservative listener gate until shell hit testing maps a pixel to a
    /// DOM event target. A non-passive listener anywhere keeps wheel on the
    /// main-thread path.
    pub fn has_blocking_scroll_listener(&self) -> bool {
        let host = self.host.borrow();
        let types = &["wheel", "mousewheel", "touchstart", "touchmove"];
        (0..host.dom.node_count()).any(|index| {
            host.dom.has_blocking_listener(
                amazing_dom::events::EventTarget::Node(amazing_dom::NodeIdx(index as u32)), types)
        })
    }
    /// Fetch lazy images the viewport scrolled into view using this page's
    /// session, then repaint the retained scroll frame.
    pub fn load_deferred_lazy_images(&self, viewport: &mut EngineViewport) -> Result<bool, String> {
        viewport.load_deferred_lazy_images(Some(&self.session), &self.options)
    }

    fn new(
        dom: amazing_dom::Dom,
        document_url: String,
        options: FetchOptions,
        scripts: Vec<String>,
        session: Rc<RefCell<BrowserSession>>,
    ) -> Self {
        Self::new_with_metrics(dom, document_url, options, scripts, session, ViewportMetrics::default())
    }

    fn new_with_metrics(
        dom: amazing_dom::Dom, document_url: String, options: FetchOptions,
        scripts: Vec<String>, session: Rc<RefCell<BrowserSession>>, metrics: ViewportMetrics,
    ) -> Self {
        let host = Rc::new(RefCell::new(ShellDomHost::new(
            dom,
            document_url.clone(),
            session.clone(),
        )));
        let mut interpreter = amazing_js::Interpreter::new();
        interpreter.set_dom_host(host.clone());
        interpreter.set_location_url(document_url.clone());
        interpreter.set_viewport_metrics(metrics.css_width as u32, metrics.css_height as u32,
            f64::from(metrics.device_scale)).expect("validated actual viewport metrics");
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
            acquisition: crate::navigation_acquisition::current_replay(),
            metrics,
            host,
            interpreter,
            document_url,
            options,
            session,
        }
    }

    fn snapshot(&mut self, default_prevented: bool) -> RuntimeUpdate {
        if let Some(replay) = self.acquisition.clone() {
            return crate::navigation_acquisition::with_replay(replay, || self.snapshot_inner(default_prevented));
        }
        self.snapshot_inner(default_prevented)
    }
    fn snapshot_inner(&mut self, default_prevented: bool) -> RuntimeUpdate {
        let navigation = self.interpreter.take_navigation();
        let history_delta = self.interpreter.take_history_delta();
        let history_update = self
            .interpreter
            .take_history_update()
            .map(|(url, replace, _)| (url, replace));
        let (viewport, render_failure) = match self.render_viewport() {
            Ok(viewport) => (Some(viewport), None),
            Err(error) => (None, Some(error)),
        };
        let host = self.host.borrow();
        let dom = &host.dom;
        RuntimeUpdate {
            viewport, render_failure, metrics: self.metrics,
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

    fn render_viewport(&self) -> Result<EngineViewport, String> {
        if let Some(replay) = &self.acquisition {
            return crate::navigation_acquisition::with_replay(replay.clone(), || self.render_viewport_inner());
        }
        self.render_viewport_inner()
    }
    fn render_viewport_inner(&self) -> Result<EngineViewport, String> {
        let host = self.host.borrow();
        let (css, font_sheets) = load_document_styles(&host.dom, &self.document_url,
            &self.options, Some(&self.session));
        let resources = load_page_resources_with_metrics(&host.dom, &css, &self.document_url,
            &self.options, Some(&self.session), self.metrics);
        EngineViewport::from_dom_with_style_sources_and_metrics(&host.dom, &css, resources,
            Some(&self.document_url), &font_sheets, self.metrics)
    }

    /// Reflow the authentic runtime DOM without consuming navigation/history
    /// directives, pumping timers, or evaluating author script.
    pub fn reflow(&mut self, metrics: ViewportMetrics) -> Result<EngineViewport, String> {
        self.metrics = metrics;
        self.interpreter.set_viewport_metrics(metrics.css_width as u32, metrics.css_height as u32,
            f64::from(metrics.device_scale)).expect("validated actual viewport metrics");
        self.render_viewport()
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
        Self::load_request_with_metrics(address, method, body, session, ViewportMetrics::default())
    }

    pub fn load_request_with_metrics(
        address: &str, method: &str, body: Vec<u8>,
        session: Rc<RefCell<BrowserSession>>, metrics: ViewportMetrics,
    ) -> Result<LoadedPage, String> {
        if address == "amazing://status" {
            return Ok(LoadedPage {
                viewport: Self::demo().try_reflow_static(metrics)?.ok_or("missing demo document")?,
                final_address: address.to_string(),
                title: Some("Amazing — Phase 9 status".into()),
                favicon: None,
                runtime: None,
            });
        }
        if address == "amazing://newtab" {
            return Ok(LoadedPage {
                viewport: Self::new_tab().try_reflow_static(metrics)?.ok_or("missing newtab document")?,
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
            let mut runtime = PageRuntime::new_with_metrics(
                dom, final_address.clone(), options, scripts, session, metrics,
            );
            let update = runtime.snapshot(false);
            return Ok(LoadedPage {
                viewport: update.viewport.ok_or_else(|| update.render_failure.unwrap_or_else(|| "initial runtime render unavailable".into()))?,
                final_address,
                title: update.title,
                favicon: update.favicon,
                runtime: Some(runtime),
            });
        }
        let (css, font_sheets) = load_document_styles(&dom, &final_address, &options, Some(&session));
        let resources = load_page_resources_with_metrics(&dom, &css, &final_address, &options, Some(&session), metrics);
        let title = document_title(&dom);
        let favicon = load_favicon(&dom, &final_address, &options, Some(&session));
        let mut viewport = Self::from_dom_with_style_sources_and_metrics(
            &dom, &css, resources, Some(&final_address), &font_sheets, metrics)?;
        viewport.static_document = Some(Rc::new(StaticDocument {
            acquisition: crate::navigation_acquisition::current_replay(),
            dom, css, font_sheets, document_url: Some(final_address.clone()),
            options, session: Some(session),
        }));
        Ok(LoadedPage {
            viewport,
            final_address,
            title,
            favicon,
            runtime: None,
        })
    }

    pub fn refresh_initial_page(page: &mut LoadedPage, metrics: ViewportMetrics) -> Result<(), String> {
        if let Some(runtime) = page.runtime.as_mut() {
            // Never rerun author script while awaiting newly selected resources.
            page.viewport = runtime.reflow(metrics)?;
            let host = runtime.host.borrow();
            page.title = document_title(&host.dom);
            page.favicon = load_favicon(&host.dom, &runtime.document_url, &runtime.options, Some(&runtime.session));
        } else if let Some(viewport) = page.viewport.try_reflow_static(metrics)? {
            page.viewport = viewport;
        }
        Ok(())
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
                metrics: ViewportMetrics::new(decoded.width as f32, decoded.height as f32, 1.).expect("video extent"),
                static_document: None,
                compositor: None,
                scroll_controller: Rc::new(RefCell::new(amazing_compositor::ScrollController::default())),
                scroll_frame: Rc::new(RefCell::new(None)),
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
            metrics: ViewportMetrics::new(width as f32, height as f32, 1.).ok_or("invalid RGBA extent")?,
            static_document: None,
            compositor: None,
            scroll_controller: Rc::new(RefCell::new(amazing_compositor::ScrollController::default())),
            scroll_frame: Rc::new(RefCell::new(None)),
            page: None,
            width,
            height,
            links: Arc::new(Vec::new()),
            controls: Arc::new(Vec::new()),
        })
    }

    fn from_document(html: &str, css: &str) -> Self {
        let dom = amazing_html::parse(html);
        let mut viewport = Self::from_dom(&dom, css);
        viewport.static_document = Some(Rc::new(StaticDocument {
            acquisition: crate::navigation_acquisition::current_replay(),
            dom, css: css.into(), font_sheets: Vec::new(), document_url: None,
            options: FetchOptions::default(), session: None,
        }));
        viewport
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
        Self::from_dom_with_style_sources_and_metrics(dom, css, resources, document_url,
            font_sheets, ViewportMetrics::default()).expect("unit shell document render")
    }

    fn from_dom_with_style_sources_and_metrics(
        dom: &amazing_dom::Dom, css: &str, resources: PageResources,
        document_url: Option<&str>, font_sheets: &[(String, String)], metrics: ViewportMetrics,
    ) -> Result<Self, String> {
        let (surface, links, controls, scene) =
            render_dom_with_resources(dom, css, resources, document_url, font_sheets, metrics)?;
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
        Ok(Self {
            image,
            metrics,
            static_document: None,
            compositor,
            scroll_controller: Rc::new(RefCell::new(amazing_compositor::ScrollController::default())),
            scroll_frame: Rc::new(RefCell::new(None)),
            page,
            width,
            height,
            links: Arc::new(links),
            controls: Arc::new(controls),
        })
    }

    /// Static reflow retains the actual originally parsed arena. Runtime pages
    /// instead reflow through their authentic ShellDomHost owner.
    pub fn reflow_static(&self, metrics: ViewportMetrics) -> Option<Self> {
        self.try_reflow_static(metrics).expect("static shell render")
    }

    /// Actual successfully decoded images retained in the painted page snapshot.
    pub fn decoded_image_count(&self) -> usize { self.page.as_ref().map_or(0, |page| page.resources.images.len()) }

    pub fn font_display_trace(&self) -> String {
        crate::navigation_acquisition::font_trace(self.static_document.as_ref().and_then(|document| document.acquisition.as_ref()))
    }
    pub fn try_reflow_static(&self, metrics: ViewportMetrics) -> Result<Option<Self>, String> {
        let Some(input) = self.static_document.as_ref() else { return Ok(None) };
        if let Some(replay) = &input.acquisition {
            return crate::navigation_acquisition::with_replay(replay.clone(), || self.reflow_static_input(input, metrics));
        }
        self.reflow_static_input(input, metrics)
    }
    fn reflow_static_input(&self, input: &Rc<StaticDocument>, metrics: ViewportMetrics) -> Result<Option<Self>, String> {
        let resources = if let Some(url) = &input.document_url {
            load_page_resources_with_metrics(&input.dom, &input.css, url, &input.options,
                input.session.as_ref(), metrics)
        } else { PageResources { metrics, ..Default::default() } };
        let mut next = Self::from_dom_with_style_sources_and_metrics(&input.dom, &input.css,
            resources, input.document_url.as_deref(), &input.font_sheets, metrics)?;
        next.static_document = Some(input.clone());
        Ok(Some(next))
    }

    /// The new root owns its own scroll IDs; only the logical offset crosses
    /// a reflow. The compositor clamps against the newly laid-out extents.
    pub fn restore_root_scroll(&self, offset: (f32, f32)) -> Result<(), String> {
        if let Some(node) = self.root_scroll_id() {
            self.scroll_input(amazing_compositor::ScrollInput::ScrollTo {
                node, target: offset, behavior: amazing_compositor::ScrollBehavior::Instant,
            }, false)?;
        }
        Ok(())
    }

    pub fn identity(&self) -> ViewportIdentity {
        ViewportIdentity {
            metrics: self.metrics,
            static_document: self.static_document.as_ref().map(Rc::downgrade),
            page: self.page.as_ref().map(Rc::downgrade),
        }
    }

    pub fn accepts_identity(&self, other: &ViewportIdentity) -> bool {
        if self.metrics != other.metrics { return false; }
        match (&self.static_document, &other.static_document) {
            (Some(a), Some(b)) => Weak::ptr_eq(&Rc::downgrade(a), b),
            _ => self.page.as_ref().zip(other.page.as_ref())
                .is_some_and(|(a, b)| Weak::ptr_eq(&Rc::downgrade(a), b)),
        }
    }

    pub fn same_document_owner(&self, other: &Self) -> bool {
        match (&self.static_document, &other.static_document) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => self.page.as_ref().zip(other.page.as_ref()).is_some_and(|(a, b)| Rc::ptr_eq(a, b)),
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
        ).with_raster_viewport(self.metrics.raster_viewport()?)
            .map_err(|error| format!("retained paint view: {error:?}"))?;
        let mut scene = scene.borrow_mut();
        change(scene.compositor_mut())?;
        scene.update(&view)
    }

    /// The root viewport is a compositor scroll node. The shell keeps input
    /// state here so wheel events and keyboard steps share one event queue.
    pub fn scroll_input(
        &self,
        input: amazing_compositor::ScrollInput,
        blocking_listener: bool,
    ) -> Result<bool, String> {
        static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let now_ms = CLOCK.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() * 1000.;
        let (Some(_), Some(scene)) = (&self.page, &self.compositor) else { return Ok(false) };
        let result = {
            let mut scene = scene.borrow_mut();
            let input = self.device_scroll_input(input, scene.compositor_mut())?;
            self.scroll_controller.borrow_mut()
                .handle_input(scene.compositor_mut(), input, blocking_listener, now_ms)
                .map_err(|error| error.to_string())?
        };
        if !matches!(result, amazing_compositor::ScrollInputResult::Scrolled { .. }) {
            return Ok(false);
        }
        self.rebuild_retained_frame()?;
        Ok(true)
    }

    /// Input and DOM event offsets remain CSS pixels; the retained scene is physical.
    fn device_scroll_input(
        &self, input: amazing_compositor::ScrollInput,
        compositor: &amazing_compositor::Compositor,
    ) -> Result<amazing_compositor::ScrollInput, String> {
        use amazing_compositor::{ScrollInput as I, ScrollKey as K, ScrollBehavior};
        let viewport = self.metrics.raster_viewport()?;
        let pair = |value| viewport.css_scroll_to_device(value)
            .ok_or_else(|| "nonfinite device scroll input".to_string());
        Ok(match input {
            I::Wheel { x, y, delta } => { let (x,y)=pair((x,y))?; I::Wheel{x,y,delta:pair(delta)?} }
            I::Trackpad { x, y, delta } => { let (x,y)=pair((x,y))?; I::Trackpad{x,y,delta:pair(delta)?} }
            I::TouchPan { x, y, delta } => { let (x,y)=pair((x,y))?; I::TouchPan{x,y,delta:pair(delta)?} }
            I::ScrollTo { node, target, behavior } => I::ScrollTo{node,target:pair(target)?,behavior},
            I::Momentum { node, velocity } => I::Momentum{node,velocity:pair(velocity)?},
            I::Keyboard { node, key } if matches!(key,K::ArrowUp|K::ArrowDown|K::ArrowLeft|K::ArrowRight) => {
                let old = compositor.trees().scroll(node).ok_or("missing scroll node")?.offset;
                let delta = match key { K::ArrowUp=>(0.,-40.), K::ArrowDown=>(0.,40.),
                    K::ArrowLeft=>(-40.,0.), _=>(40.,0.) };
                let delta = pair(delta)?;
                I::ScrollTo{node,target:(old.0+delta.0,old.1+delta.1),behavior:ScrollBehavior::Instant}
            }
            // Page distance and endpoints already derive from physical scene bounds.
            other => other,
        })
    }

    /// Animates a transform property (e.g. a CSS `transform` transition) on
    /// the live viewport through the same retained, cached-tile frame path
    /// `scroll_input` uses, instead of a full DOM-to-paint repaint (COMP-18).
    pub fn animate_transform(
        &self,
        id: amazing_compositor::TransformId,
        matrix: amazing_compositor::Matrix4,
    ) -> Result<(), String> {
        let Some(scene) = &self.compositor else { return Ok(()) };
        let mut matrix = matrix;
        let axes = [self.metrics.device_scale, self.metrics.device_scale, 1., 1.];
        for (row, factor) in axes.iter().enumerate() {
            for (column, inverse) in axes.iter().enumerate() {
                matrix.0[row][column] *= factor / inverse;
                if !matrix.0[row][column].is_finite() { return Err("nonfinite device transform".into()); }
            }
        }
        scene
            .borrow_mut()
            .compositor_mut()
            .set_transform(id, matrix)
            .map_err(|error| error.to_string())?;
        self.rebuild_retained_frame()
    }

    /// Animates an effect's opacity (e.g. a CSS `opacity` transition) on the
    /// live viewport through the same retained, cached-tile frame path
    /// `scroll_input` uses, instead of a full DOM-to-paint repaint (COMP-18).
    pub fn animate_opacity(
        &self,
        id: amazing_compositor::EffectId,
        opacity: f32,
    ) -> Result<(), String> {
        let Some(scene) = &self.compositor else { return Ok(()) };
        scene
            .borrow_mut()
            .compositor_mut()
            .set_opacity(id, opacity)
            .map_err(|error| error.to_string())?;
        self.rebuild_retained_frame()
    }

    /// Rebuilds the retained scroll/animation frame from the compositor's
    /// current property state, reusing cached tiles whenever the scene's
    /// tile set is still valid (`build_cached_frame`), and only falling back
    /// to a full `DisplayListView` rebuild when a tile was actually evicted.
    /// Shared by `scroll_input`, `animate_transform` and `animate_opacity` so
    /// scroll, transform and opacity property updates all reuse tiles
    /// instead of repainting (COMP-18).
    fn rebuild_retained_frame(&self) -> Result<(), String> {
        let (Some(page), Some(scene)) = (&self.page, &self.compositor) else { return Ok(()) };
        let first = self.scroll_frame.borrow().is_none();
        let mut scene = scene.borrow_mut();
        match scene.compositor_mut().build_cached_frame(self.width, self.height) {
            Ok(frame) => scene.present_cached_viewport_frame(frame, first)?,
            Err(amazing_compositor::SceneError::MissingCachedTile) => {
                let images = amazing_paint::image::StyledImages { images: &page.resources, styles: &page.styles };
                let view = amazing_paint::display_list_view::DisplayListView::new(
                    &page.root, self.width, self.height, &page.text, &images,
                    &amazing_paint::controls::NoControls,
                ).with_raster_viewport(self.metrics.raster_viewport()?)
                    .map_err(|error| format!("retained raster viewport: {error:?}"))?;
                scene.update_viewport_frame(&view)?;
            }
            Err(error) => return Err(error.to_string()),
        }
        *self.scroll_frame.borrow_mut() = Some(scene.frame().clone());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn completed_presentations_for_fixture(&self) -> u64 {
        self.compositor.as_ref().unwrap().borrow().completed_presentations()
    }

    pub fn load_static_deferred_images(&mut self) -> Result<bool, String> {
        let Some(input) = self.static_document.clone() else { return Ok(false) };
        self.load_deferred_lazy_images(input.session.as_ref(), &input.options)
    }

    /// Fetch deferred lazy images the root scroll offset has brought into view,
    /// then repaint the scrolled frame. Layout already reserved each deferred
    /// box, so the fetched pixels paint into it; the next DOM snapshot relayouts
    /// with the real intrinsic size. Returns true when new pixels were loaded.
    pub fn load_deferred_lazy_images(
        &mut self,
        session: Option<&Rc<RefCell<BrowserSession>>>,
        options: &FetchOptions,
    ) -> Result<bool, String> {
        let (scroll_x, scroll_y) = self.root_scroll_offset();
        let viewport = amazing_media::loading::ViewportRect {
            x: scroll_x,
            y: scroll_y,
            width: self.metrics.css_width,
            height: self.metrics.css_height,
        };
        let loaded = {
            let Some(page) = self.page.as_mut() else { return Ok(false) };
            let Some(page) = Rc::get_mut(page) else { return Ok(false) };
            if page.resources.deferred_images.is_empty() {
                return Ok(false);
            }
            let pending = std::mem::take(&mut page.resources.deferred_images);
            let mut loaded = false;
            for entry in pending {
                let rect = image_rect(&page.root, entry.node).map(|rect| {
                    amazing_media::loading::ViewportRect {
                        x: rect.x,
                        y: rect.y,
                        width: rect.w,
                        height: rect.h,
                    }
                });
                let fetch = amazing_media::loading::should_fetch(
                    amazing_media::loading::Loading::Lazy,
                    rect,
                    viewport,
                    0.0,
                );
                if !fetch {
                    page.resources.deferred_images.push(entry);
                    continue;
                }
                let retained: usize = page
                    .resources
                    .images
                    .iter()
                    .map(|entry| entry.image.pixels.len())
                    .sum();
                if page.resources.images.len() >= MAX_PAGE_IMAGES
                    || retained >= 128 * 1024 * 1024
                {
                    page.resources.deferred_images.push(entry);
                    continue;
                }
                let Ok(response) = fetch_resource(session, &entry.address, options) else {
                    page.resources.deferred_images.push(entry);
                    continue;
                };
                if response.status != 200 {
                    continue;
                }
                let Some(image) = decode_web_image(&response.body) else { continue };
                let retained: usize = page
                    .resources
                    .images
                    .iter()
                    .map(|entry| entry.image.pixels.len())
                    .sum();
                if retained.saturating_add(image.pixels.len()) > 128 * 1024 * 1024 {
                    continue;
                }
                page.resources.images.push(PageImage {
                    node: entry.node,
                    image,
                    density: entry.density,
                });
                loaded = true;
            }
            loaded
        };
        if !loaded {
            return Ok(false);
        }
        self.repaint_scroll_frame()?;
        Ok(true)
    }

    /// Repaint the retained scroll frame from the current page resources.
    fn repaint_scroll_frame(&self) -> Result<(), String> {
        let (Some(page), Some(scene)) = (&self.page, &self.compositor) else { return Ok(()) };
        let images = amazing_paint::image::StyledImages { images: &page.resources, styles: &page.styles };
        let view = amazing_paint::display_list_view::DisplayListView::new(
            &page.root, self.width, self.height, &page.text, &images,
            &amazing_paint::controls::NoControls,
        ).with_raster_viewport(self.metrics.raster_viewport()?)
            .map_err(|error| format!("retained paint view: {error:?}"))?;
        let mut scene = scene.borrow_mut();
        scene.update_viewport_frame(&view)?;
        *self.scroll_frame.borrow_mut() = Some(scene.frame().clone());
        Ok(())
    }

    pub fn scroll_element(&self) -> Option<gpui::AnyElement> {
        self.scroll_frame.borrow().as_ref().map(|frame| frame.element_with_device_scale(self.metrics.device_scale))
    }

    /// A11Y-11: the accessibility tree for this page, if one was retained
    /// (a plain `Surface`-only render with no compositor scene — e.g. a
    /// failed paint fallback — has no `RetainedPage` and so no tree either).
    pub fn accessibility_tree(&self) -> Option<&amazing_a11y::AxTree> {
        self.page.as_deref().map(|page| &page.accessibility)
    }

    pub fn root_scroll_id(&self) -> Option<amazing_compositor::ScrollId> {
        self.compositor.as_ref()?.borrow_mut().compositor_mut().root_scroll_id()
    }

    pub fn root_scroll_offset(&self) -> (f32, f32) {
        self.compositor.as_ref().map_or((0., 0.), |scene|
            { let offset = scene.borrow_mut().compositor_mut().root_scroll_offset();
              (offset.0 / self.metrics.device_scale, offset.1 / self.metrics.device_scale) })
    }

    pub fn root_scroll_fraction(&self) -> Option<f32> {
        let mut scene = self.compositor.as_ref()?.borrow_mut();
        let compositor = scene.compositor_mut();
        let root = compositor.root_scroll_id()?;
        let max = compositor.scroll_range(root)?.1;
        (max > 0.).then(|| compositor.root_scroll_offset().1 / max)
    }

    pub fn root_scroll_thumb(&self) -> Option<(f32, f32)> {
        let mut scene = self.compositor.as_ref()?.borrow_mut();
        let compositor = scene.compositor_mut();
        let root = compositor.root_scroll_id()?;
        let node = compositor.trees().scroll(root)?;
        let max = (node.contents.h - node.container.h).max(0.);
        if max == 0. { return None }
        let size = (node.container.h / node.contents.h).clamp(0.06, 1.);
        Some((node.offset.1 / max * (1. - size), size))
    }

    pub fn take_scroll_events(&self) -> Vec<amazing_compositor::AsyncScrollEvent> {
        let mut events = self.scroll_controller.borrow_mut().take_scroll_events();
        for event in &mut events {
            event.offset.0 /= self.metrics.device_scale;
            event.offset.1 /= self.metrics.device_scale;
        }
        events
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

    pub fn control_identity(&self, index: usize) -> Option<(u32, u64)> {
        self.controls.get(index).map(|control| (control.node, control.generation))
    }

    pub fn text_control_by_identity(&self, identity: (u32, u64)) -> Option<(usize, ControlRect)> {
        let index = self.controls.iter().position(|control|
            control.kind == ControlKind::Text && (control.node, control.generation) == identity)?;
        self.text_control(index).map(|(_, rect)| (index, rect))
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
    render_dom_with_resources(&dom, css, PageResources::default(), None, &[], ViewportMetrics::default()).expect("unit document render").0
}

fn render_dom_with_resources(
    dom: &amazing_dom::Dom,
    css: &str,
    resources: PageResources,
    document_url: Option<&str>,
    font_sheets: &[(String, String)],
    metrics: ViewportMetrics,
) -> Result<(Surface, Vec<LinkRegion>, Vec<ControlRegion>, Option<(RetainedPage, EngineCompositor)>), String> {
    let text = load_page_text(css, document_url.unwrap_or("amazing://newtab"), font_sheets);
    let environment = amazing_css::MediaEnvironment {
        viewport_width_px: metrics.css_width, viewport_height_px: metrics.css_height,
        device_pixel_ratio: metrics.device_scale, ..Default::default()
    };
    let styles = amazing_css::compute_with_environment(dom,
        &[amazing_css::parse_stylesheet(css)], &environment);
    let root = amazing_layout::layout_styled_document_from_root(dom, amazing_dom::NodeIdx(0),
        &styles, &text, &resources, metrics.css_width, Some(metrics.css_height))
        .expect("shell renders its actual Document root");
    record_font_display_state(&text)?;
    let links = document_url
        .map(|url| collect_link_regions(dom, &root, url))
        .unwrap_or_default();
    let controls = document_url
        .map(|url| collect_control_regions(dom, &root, url))
        .unwrap_or_default();
    let images = amazing_paint::image::StyledImages { images: &resources, styles: &styles };
    let raster = metrics.raster_viewport()?;
    let (physical_width, physical_height) = raster.physical_size();
    let surface = amazing_paint::paint_document_at_viewport(&root, raster, &text,
        &images, &amazing_paint::controls::NoControls).map_err(|error| format!("raster: {error:?}"))?;
    let view = amazing_paint::display_list_view::DisplayListView::new(&root,
        physical_width, physical_height, &text, &images, &amazing_paint::controls::NoControls)
        .with_raster_viewport(raster).map_err(|error| format!("paint view: {error:?}"))?;
    let compositor = Some(EngineCompositor::new(&view, physical_width, physical_height)?);
    drop(view);
    drop(images);
    let accessibility = amazing_a11y::build(dom, &styles, &root);
    let scene = compositor.map(|compositor| (
        RetainedPage { root, text, styles, resources, accessibility },
        compositor,
    ));
    Ok((surface, links, controls, scene))
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
            && let Some(generation) = dom.node_generation(node)
            && layout.border_box.w > 0.0
            && layout.border_box.h > 0.0
        {
            controls.push(ControlRegion {
                node: node.0,
                generation,
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
    let request = if session.is_some() {
        crate::navigation_acquisition::ResourceRequest::Session(address.into(), options.clone())
    } else {
        crate::navigation_acquisition::ResourceRequest::Raw(address.into(), options.clone())
    };
    if let Some(result) = crate::navigation_acquisition::response(request) { return result; }
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

fn image_rect(layout: &amazing_layout::LayoutBox, node: amazing_dom::NodeIdx) -> Option<amazing_layout::Rect> {
    for line in &layout.lines {
        if let Some(fragment) = line.replaced.iter().find(|fragment| fragment.node == node) {
            return Some(amazing_layout::Rect {
                x: fragment.x, y: fragment.y, w: fragment.width, h: fragment.height,
            });
        }
    }
    if layout.node == Some(node) { return Some(layout.content_box); }
    let children = match &layout.flow {
        amazing_layout::Flow::Blocks(children) | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => children.as_slice(),
        _ => &[],
    };
    children.iter().chain(&layout.positioned).find_map(|child| image_rect(child, node))
}

fn load_page_resources(
    dom: &amazing_dom::Dom,
    css: &str,
    document_url: &str,
    options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>,
) -> PageResources {
    load_page_resources_with_metrics(dom, css, document_url, options, session, ViewportMetrics::default())
}

fn load_page_resources_with_metrics(
    dom: &amazing_dom::Dom, css: &str, document_url: &str, options: &FetchOptions,
    session: Option<&Rc<RefCell<BrowserSession>>>, metrics: ViewportMetrics,
) -> PageResources {
    fn attr<'a>(dom: &'a amazing_dom::Dom, node: amazing_dom::NodeIdx, name: &str) -> Option<&'a str> {
        dom.attrs_of(node)?.iter().find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
    fn source_for(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, metrics: ViewportMetrics) -> Option<amazing_media::responsive::SelectedSource> {
        use amazing_media::responsive::{ImageSources, PictureSource, SourceEnvironment, select_source_with_sizes};
        let mut picture = Vec::new();
        if let Some(parent) = dom.get(node).parent.filter(|parent| dom.name(*parent) == Some("picture")) {
            for sibling in dom.children(parent) {
                if sibling == node { break; }
                if dom.name(sibling) == Some("source") {
                    picture.push(PictureSource {
                        srcset: attr(dom, sibling, "srcset").unwrap_or(""),
                        sizes: attr(dom, sibling, "sizes"),
                        media: attr(dom, sibling, "media"),
                        mime_type: attr(dom, sibling, "type"),
                    });
                }
            }
        }
        select_source_with_sizes(ImageSources {
            src: attr(dom, node, "src"), srcset: attr(dom, node, "srcset"),
            sizes: attr(dom, node, "sizes"), picture: &picture,
        }, SourceEnvironment {
            viewport_width: metrics.css_width, device_pixel_ratio: metrics.device_scale,
            slot_width: None, allow_auto_sizes: false,
        }, |query| {
            amazing_css::evaluate_media_query(&amazing_css::MediaQueryList::parse(query),
                &amazing_css::MediaEnvironment {
                    viewport_width_px: metrics.css_width,
                    viewport_height_px: metrics.css_height,
                    ..Default::default()
                })
        }, |mime| matches!(mime.to_ascii_lowercase().as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" |
            "image/bmp" | "image/x-ms-bmp" | "image/x-icon" |
            "image/x-portable-pixmap" | "image/svg+xml"), |sizes, env, _| {
                amazing_css::parse_sizes_attribute(sizes.unwrap_or(""),
                    &amazing_css::MediaEnvironment {
                        viewport_width_px: env.viewport_width,
                        viewport_height_px: metrics.css_height,
                        device_pixel_ratio: env.device_pixel_ratio,
                        ..Default::default()
                    }, env.allow_auto_sizes.then_some(env.slot_width).flatten())
            })
    }
    fn walk(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        document_url: &str,
        options: &FetchOptions,
        session: Option<&Rc<RefCell<BrowserSession>>>,
        resources: &mut PageResources,
        metrics: ViewportMetrics,
    ) {
        if resources.images.len() + resources.deferred_images.len() >= MAX_PAGE_IMAGES {
            return;
        }
        if dom.name(node) == Some("img")
            && let Some(source) = source_for(dom, node, metrics)
            && let Some(address) = resolve_resource_address(document_url, &source.url) {
            if attr(dom, node, "loading").is_some_and(|value| value.eq_ignore_ascii_case("lazy")) {
                resources.deferred_images.push(DeferredImage { node, address, density: source.density });
            } else if let Ok(response) = fetch_resource(session, &address, options)
                && response.status == 200
                && let Some(image) = decode_web_image(&response.body) {
                let retained: usize = resources.images.iter().map(|entry| entry.image.pixels.len()).sum();
                if retained.saturating_add(image.pixels.len()) <= 128 * 1024 * 1024 {
                    resources.images.push(PageImage { node, image, density: source.density });
                }
            }
        } else if let Some(attribute) = match dom.name(node) {
            Some("embed") => Some("src"),
            Some("object") => Some("data"),
            Some("video") => Some("poster"),
            _ => None,
        } {
            if let Some(address) = attr(dom, node, attribute)
                .filter(|source| !source.trim().is_empty())
                .and_then(|source| resolve_resource_address(document_url, source))
                && let Ok(response) = fetch_resource(session, &address, options)
                && response.status == 200
                && let Some(image) = decode_web_image(&response.body) {
                let retained: usize = resources.images.iter().map(|entry| entry.image.pixels.len()).sum();
                if retained.saturating_add(image.pixels.len()) <= 128 * 1024 * 1024 {
                    resources.images.push(PageImage { node, image, density: 1.0 });
                }
            }
        }

        for child in dom.children(node) {
            walk(dom, child, document_url, options, session, resources, metrics);
        }
    }

    let mut resources = PageResources { metrics, ..Default::default() };
    walk(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        session,
        &mut resources,
        metrics,
    );
    if !resources.deferred_images.is_empty() {
        let environment = amazing_css::MediaEnvironment {
            viewport_width_px: metrics.css_width, viewport_height_px: metrics.css_height,
            device_pixel_ratio: metrics.device_scale, ..Default::default()
        };
        let styled = amazing_css::compute_with_environment(dom, &[amazing_css::parse_stylesheet(css)], &environment);
        let preliminary = amazing_layout::layout_styled_document_from_root(dom, amazing_dom::NodeIdx(0),
            &styled, &amazing_text::FallbackTextSystem::new(), &resources,
            metrics.css_width, Some(metrics.css_height)).expect("actual lazy-image Document");
        let viewport = amazing_media::loading::ViewportRect {
            x: 0.0, y: 0.0, width: metrics.css_width, height: metrics.css_height,
        };
        let pending = std::mem::take(&mut resources.deferred_images);
        for entry in pending {
            let rect = image_rect(&preliminary, entry.node).map(|rect| amazing_media::loading::ViewportRect {
                x: rect.x, y: rect.y, width: rect.w, height: rect.h,
            });
            if !amazing_media::loading::should_fetch(amazing_media::loading::Loading::Lazy,
                rect, viewport, 0.0) {
                resources.deferred_images.push(entry);
                continue;
            }
            if resources.images.len() >= MAX_PAGE_IMAGES { break; }
            let Ok(response) = fetch_resource(session, &entry.address, options) else { continue; };
            if response.status != 200 { continue; }
            let Some(image) = decode_web_image(&response.body) else { continue; };
            let retained: usize = resources.images.iter().map(|entry| entry.image.pixels.len()).sum();
            if retained.saturating_add(image.pixels.len()) > 128 * 1024 * 1024 { continue; }
            resources.images.push(PageImage { node: entry.node, image, density: entry.density });
        }
    }
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
    #[cfg(test)]
    { return amazing_image::decode(bytes, amazing_image::DecodeLimits::default()).ok(); }
    #[cfg(not(test))]
    {
        thread_local! {
            static DECODER: RefCell<Option<amazing_process::ProcessHost>> = const { RefCell::new(None) };
        }
        DECODER.with(|decoder| {
            use amazing_process::{Capabilities, ProcessHost, RestartPolicy};
            let mut host = decoder.borrow_mut();
            if host.is_none() {
                let exe = std::env::current_exe().ok()?;
                match ProcessHost::spawn_sandboxed_utility(&exe,
                    Capabilities(std::collections::BTreeMap::new()), RestartPolicy::Never) {
                    Ok(process) => *host = Some(process),
                    Err(_) => {
                        #[cfg(target_os = "linux")]
                        if bytes.starts_with(b"BM") || bytes.starts_with(b"P3")
                            || bytes.starts_with(b"P6") || bytes.starts_with(b"<") {
                            return amazing_image::decode(bytes, amazing_image::DecodeLimits::default()).ok();
                        }
                        return None;
                    }
                }
            }
            let result = host.as_mut()?.decode_untrusted(amazing_process::utility::IMAGE,
                bytes, std::time::Duration::from_secs(5));
            match result {
                Ok(frame) => Some(amazing_media::Image {
                    width: frame.width, height: frame.height, pixels: frame.bytes().to_vec(),
                }),
                Err(_) => { *host = None; None }
            }
        })
    }
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

// Keep font discovery identical to the engine driver on every platform.
pub(crate) mod page_font_loader;

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
    let result = crate::navigation_acquisition::response(
        crate::navigation_acquisition::ResourceRequest::Raw(url.into(), options.clone()))
        .unwrap_or_else(|| amazing_net::fetch(url, &options));
    result.map(|response| response.body)
        .map_err(|error| error.to_string())
}

pub(crate) fn resolve_font_url(page: &str, source: &str) -> String {
    resolve_resource_address(page, source).unwrap_or_else(|| source.to_owned())
}

fn register_system_text_faces(provider: &mut amazing_text_provider::TextProvider) {
    use amazing_text::FontSlant;
    use amazing_text_provider::{FontFormat, WebFont};

    #[cfg(not(target_os = "windows"))]
    page_font_loader::register_linux_system_faces(provider);

    #[cfg(target_os = "windows")]
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

fn record_font_display_state(text: &TextProvider) -> Result<(), String> {
    let Some(replay) = crate::navigation_acquisition::current_replay() else { return Ok(()) };
    let faces = {
        let state = replay.borrow(); state.current_font_faces.iter().filter_map(|key|
            state.font_timelines.get(key).map(|timeline| (key.clone(), timeline.clone()))).collect::<Vec<_>>()
    };
    let mut states = std::collections::BTreeMap::new();
    for (key, timeline) in faces {
        states.insert(key, text.font_display_face_state(&timeline).map_err(|error| format!("font display state: {error:?}"))?);
    }
    let deadline = text.next_font_display_deadline().map_err(|error| format!("font display deadline: {error:?}"))?;
    let mut state = replay.borrow_mut(); state.font_states = states; state.next_font_deadline = deadline;
    Ok(())
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
    let replay = crate::navigation_acquisition::current_replay();
    if let Some(replay) = &replay { replay.borrow_mut().current_font_faces.clear(); }
    let mut font_fetch_cache = std::collections::HashMap::new();
    for (sheet_index, (source, stylesheet_url)) in expanded.iter().enumerate() {
      for (rule_index, face) in amazing_css::parse_font_faces(source).into_iter().enumerate() {
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
            cache: &mut font_fetch_cache,
        };
        let result = if let Some(replay) = &replay {
            let key = format!("{page:?}|{stylesheet_url:?}|{sheet_index}|{rule_index}|{descriptor:?}");
            let (timeline, epoch) = {
                let mut state = replay.borrow_mut();
                if state.font_timelines.len() >= 512 && !state.font_timelines.contains_key(&key) {
                    eprintln!("amazing shell: font display declaration limit"); continue;
                }
                let timeline = state.font_timelines.entry(key.clone())
                    .or_insert_with(|| amazing_text_provider::FontDisplayTimeline::new(descriptor.clone())).clone();
                (timeline, state.page_epoch)
            };
            let result = provider.register_font_face_with_clock(descriptor, &mut loader, &timeline, || epoch.elapsed());
            if result.is_ok() { replay.borrow_mut().current_font_faces.push(key); }
            result
        } else {
            if let Err(error) = provider.begin_font_load(descriptor.display) {
                eprintln!("amazing shell: font-face {family:?}: {error:?}"); continue;
            }
            let result = provider.register_font_face(descriptor, &mut loader);
            provider.end_font_load(); result
        };
        match result {
            Ok(Some(key)) => {
                if std::env::var_os("AMAZING_NAVIGATION_TRACE").is_some() {
                    eprintln!("AMAZING_FONT_REGISTERED family={family:?} key={key:?}");
                }
            },
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
    #[test]
    fn physical_dpr_reflow_keeps_css_scroll_and_hit_geometry() {
        use super::*;
        let mut original = EngineViewport::from_document(
            "<a href='/target'>target</a><div>tall</div>",
            "body{margin:0}a{display:block;width:120px;height:40px;background:red}div{height:1800px}",
        );
        Rc::get_mut(original.static_document.as_mut().unwrap()).unwrap().document_url = Some("https://example.test/".into());
        for scale in [1., 1.25, 2., 3.] {
            let metrics = ViewportMetrics::new(400., 300., scale).unwrap();
            let viewport = original.try_reflow_static(metrics).unwrap().unwrap();
            assert_eq!((viewport.width,viewport.height),(metrics.raster_width,metrics.raster_height));
            assert_eq!(viewport.link_hit_at(10.,10.).unwrap().1,"https://example.test/target");
            let node = viewport.root_scroll_id().unwrap();
            viewport.scroll_input(amazing_compositor::ScrollInput::Keyboard {
                node,key:amazing_compositor::ScrollKey::ArrowDown,
            },false).unwrap();
            assert!((viewport.root_scroll_offset().1-40.).abs()<0.01);
            let events = viewport.take_scroll_events();
            assert!(events.iter().any(|event| (event.offset.1-40.).abs()<0.01));
            viewport.restore_root_scroll((0.,120.)).unwrap();
            assert!((viewport.root_scroll_offset().1-120.).abs()<0.01);
        }
    }
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use amazing_compositor::PaintViewRasterMode;

    #[test]
    fn measured_static_reflow_retains_document_and_changes_media_layout() {
        let original = EngineViewport::from_document(
            "<body><div id='box'>responsive</div></body>",
            "body{margin:0}#box{width:50vw;height:10vh;background:red}@media(min-width:1000px){#box{height:80px}}",
        );
        let owner = original.identity();
        let small = original.reflow_static(ViewportMetrics::new(800., 600., 1.).unwrap()).unwrap();
        let large = small.reflow_static(ViewportMetrics::new(1200., 700., 1.).unwrap()).unwrap();
        assert!(original.same_document_owner(&small));
        assert!(small.same_document_owner(&large));
        assert!(!small.accepts_identity(&owner));
        let input = large.static_document.as_ref().unwrap();
        let box_node = (0..input.dom.node_count()).map(|i| amazing_dom::NodeIdx(i as u32))
            .find(|node| input.dom.attrs_of(*node).is_some_and(|attrs|
                attrs.iter().any(|(key, value)| key == "id" && value == "box"))).unwrap();
        let small_rect = image_rect(&small.page.as_ref().unwrap().root, box_node).unwrap();
        let large_rect = image_rect(&large.page.as_ref().unwrap().root, box_node).unwrap();
        assert!((small_rect.w - 400.).abs() < 0.01);
        assert!((small_rect.h - 60.).abs() < 0.01);
        assert!((large_rect.w - 600.).abs() < 0.01);
        assert!((large_rect.h - 80.).abs() < 0.01);
        let other = EngineViewport::from_document("<body>different</body>", "");
        assert!(!other.accepts_identity(&large.identity()));
    }

    #[test]
    fn reflow_remaps_an_editable_control_by_native_node_generation() {
        let dom = amazing_html::parse("<body><form><input id=first value=old><input id=second value=edit></form></body>");
        let css = "body{margin:0}input{display:block;width:50vw;height:30px}@media(max-width:800px){#first{display:none}}";
        let mut original = EngineViewport::from_dom_with_resources(&dom, css,
            &PageResources::default(), Some("https://example.test/form"));
        let identity = original.control_identity(1).unwrap();
        original.static_document = Some(Rc::new(StaticDocument {
            acquisition: crate::navigation_acquisition::current_replay(),
            dom, css: css.into(), font_sheets: Vec::new(),
            document_url: Some("https://example.test/form".into()), options: FetchOptions::default(), session: None,
        }));
        let resized = original.reflow_static(ViewportMetrics::new(700., 500., 1.).unwrap()).unwrap();
        let (index, rect) = resized.text_control_by_identity(identity).unwrap();
        assert_eq!(index, 0);
        assert!((rect.2 - 350.).abs() < 0.01);
        assert_eq!(resized.text_control(index).unwrap().0, "edit");
        assert!(resized.text_control_by_identity((identity.0, identity.1 + 1)).is_none());
    }

    #[test]
    fn measurement_receipt_does_not_keep_retained_page_mutably_shared() {
        let viewport = EngineViewport::from_document("<body>page</body>", "");
        let receipt = viewport.identity();
        assert!(viewport.accepts_identity(&receipt));
        assert_eq!(Rc::strong_count(viewport.page.as_ref().unwrap()), 1);
        assert!(ViewportMetrics::new(0., 600., 1.).is_none());
        assert!(viewport.accepts_identity(&receipt));
    }

    #[test]
    fn resize_reprojects_logical_scroll_into_the_new_root_extent() {
        let original = EngineViewport::from_document("<body><div>tall</div></body>",
            "body{margin:0}div{height:3000px;background:red}");
        original.restore_root_scroll((0., 2000.)).unwrap();
        let previous = original.root_scroll_offset();
        assert!(previous.1 > 0.);
        let resized = original.reflow_static(ViewportMetrics::new(800., 2000., 1.).unwrap()).unwrap();
        resized.restore_root_scroll(previous).unwrap();
        let offset = resized.root_scroll_offset();
        assert!(offset.1 <= 1000. && offset.1 >= 0.);
        assert_eq!(offset.0, 0.);
        assert_eq!(resized.metrics.css_height, 2000.);
    }

    #[test]
    fn runtime_resize_preserves_pending_navigation_and_actual_arena() {
        let session = Rc::new(RefCell::new(BrowserSession::ephemeral()));
        let mut runtime = PageRuntime::new(amazing_html::parse("<body>page</body>"),
            "https://example.test/start".into(), FetchOptions::default(), Vec::new(), session);
        runtime.interpreter.run_script("location.href = 'https://example.test/next'").unwrap();
        runtime.interpreter.run_script("var resizeTimer = 0; setTimeout(function(){ resizeTimer = 1; }, 0)").unwrap();
        let node_count = runtime.host.borrow().dom.node_count();
        let resized = runtime.reflow(ViewportMetrics::new(700., 400., 1.).unwrap()).unwrap();
        assert_eq!(runtime.host.borrow().dom.node_count(), node_count);
        assert_eq!(runtime.interpreter.run_script("resizeTimer").unwrap(), amazing_js::value::JsValue::num(0.));
        assert!(runtime.interpreter.pump_timers().unwrap());
        assert_eq!(runtime.interpreter.run_script("resizeTimer").unwrap(), amazing_js::value::JsValue::num(1.));
        assert_eq!(resized.metrics.css_width, 700.);
        assert_eq!(runtime.interpreter.take_navigation().as_deref(), Some("https://example.test/next"));
        let update = runtime.snapshot(false);
        assert_eq!(update.viewport.as_ref().unwrap().metrics, resized.metrics);
    }

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
    fn accessibility_tree_is_retained_alongside_the_compositor_scene() {
        // A11Y-11: the tab's viewport carries a real amazing_a11y::AxTree
        // built from the same dom/styles/layout as the rendered surface,
        // not just a compositor scene -- this is what main.rs's tab render
        // hook mounts via a11y_bridge::ax_tree_element.
        let viewport = EngineViewport::demo();
        let tree = viewport.accessibility_tree().expect("demo page retains a page scene");
        assert!(tree.root().is_some(), "a real page should expose a root AxNode");
        assert!(!tree.nodes().is_empty(), "a real page should expose more than a bare root");
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
        let node = initial.viewport.as_ref().unwrap().links.first().unwrap().node;
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
    fn tall_documents_keep_a_fixed_viewport_and_scroll_root() {
        let viewport = EngineViewport::from_document(
            "<main><p>top</p><div></div><p>bottom</p></main>",
            "main { padding: 20px; } div { display: block; height: 900px; }",
        );
        assert_eq!(viewport.width, 960);
        assert_eq!(viewport.height, VIEWPORT_HEIGHT);
        let root = viewport.root_scroll_id().expect("document viewport scroll node");
        let range = viewport.compositor.as_ref().unwrap().borrow_mut()
            .compositor_mut().scroll_range(root).unwrap();
        assert!(range.1 > 300., "scrollable range was {range:?}");
        assert!(viewport.scroll_input(amazing_compositor::ScrollInput::Keyboard {
            node: root, key: amazing_compositor::ScrollKey::ArrowDown,
        }, false).unwrap());
        assert!(viewport.root_scroll_offset().1 > 0.);
        assert!(viewport.scroll_element().is_some());
    }

    #[test]
    fn scrolled_deferred_lazy_image_is_fetched_and_repainted() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut stream = None;
            for _ in 0..6000 {
                match listener.accept() {
                    Ok((accepted, _)) => {
                        stream = Some(accepted);
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("lazy fixture accept: {error}"),
                }
            }
            let Some(stream) = stream else { return };
            stream.set_nonblocking(false).unwrap();
            let mut stream = stream;
            let mut request = [0u8; 2048];
            let count = stream.read(&mut request).unwrap();
            assert!(String::from_utf8_lossy(&request[..count])
                .starts_with("GET /assets/pixel.ppm HTTP/1.1\r\n"));
            let body = b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00";
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: image/x-portable-pixmap\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });

        let document_url = format!("http://127.0.0.1:{port}/pages/index.html");
        let options = FetchOptions::default();
        let dom = amazing_html::parse("<div></div><img src=../assets/pixel.ppm loading=lazy width=8>");
        let css = "body{margin:0}div{display:block;height:1200px}img{display:block;width:8px}";
        let resources = load_page_resources(&dom, css, &document_url, &options, None);
        assert_eq!(resources.deferred_images.len(), 1, "below-fold image starts deferred");
        assert!(resources.images.is_empty());
        let node = resources.deferred_images[0].node;
        let mut viewport =
            EngineViewport::from_dom_with_resources(&dom, css, &resources, Some(&document_url));
        let root = viewport.root_scroll_id().expect("root scroll node");
        assert!(viewport
            .scroll_input(
                amazing_compositor::ScrollInput::Keyboard {
                    node: root,
                    key: amazing_compositor::ScrollKey::End,
                },
                false,
            )
            .unwrap());
        assert!(viewport.root_scroll_offset().1 > 0.0, "End scroll moved the root");
        let loaded = viewport.load_deferred_lazy_images(None, &options).unwrap();
        let page = viewport.page.as_ref().expect("retained page");
        assert!(
            loaded,
            "scrolled lazy image is fetched: offset={:?} deferred={} images={} node={node:?}",
            viewport.root_scroll_offset(),
            page.resources.deferred_images.len(),
            page.resources.images.len(),
        );
        server.join().unwrap();
        assert!(page.resources.deferred_images.is_empty());
        let image = page
            .resources
            .images
            .iter()
            .find(|entry| entry.node == node)
            .expect("loaded image");
        assert_eq!(image.image.pixels[0..4], [255, 0, 0, 255]);
        assert!(viewport.scroll_element().is_some());
    }

    /// COMP-18: prepends the same `fixed`/`scroller`/`moving`/`fade` scaffold
    /// COMP-17's compositor-crate frame-time trace uses inside each real-page
    /// fixture's own `<body>`, so `scroll`/`transform`/`opacity` keep the same
    /// deterministic property ids (`ScrollId(2)`, `TransformId(1)`,
    /// `EffectId(2)`) that pipeline already relies on. Building through
    /// `EngineViewport::from_document` runs the shell's own DOM-to-paint
    /// pipeline (`render_dom_with_resources`), not the compositor crate's
    /// `PaintViewSource`/`DisplayListView` directly, so this proves the real
    /// shell wiring (`scroll_input`/`animate_transform`/`animate_opacity`,
    /// which all now share `rebuild_retained_frame`) reuses tiles instead of
    /// repainting on 3 real pages. See
    /// `evidence/comp18/shell-frame-time.md` for the committed numbers.
    #[test]
    fn shell_viewport_frame_time_trace() {
        const SCAFFOLD: &str = "<div class='fixed'>Fixed</div><section class='scroller'>\
            <div class='moving'>Move</div><div class='fade'>Fade</div>\
            <p>Overflow content</p><p>More overflow content</p></section>";
        let root_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/real-pages");
        let base_css = std::fs::read_to_string(root_dir.join("assets/base.css"))
            .expect("real-pages base.css fixture");
        let scroll = amazing_compositor::ScrollId(2);
        let transform = amazing_compositor::TransformId(1);
        let effect = amazing_compositor::EffectId(2);
        println!(
            "page,scenario,frames,full_paint_us,retained_call_us,build_cache_hits,direct_tiles"
        );
        for page in ["app-kanban", "docs-guide", "news-section"] {
            let raw = std::fs::read_to_string(root_dir.join("pages").join(format!("{page}.html")))
                .unwrap_or_else(|error| panic!("read {page}.html: {error}"));
            let real_body = raw.split_once("<body>").map_or(raw.as_str(), |(_, rest)| rest);
            let html = format!("<body>{SCAFFOLD}{real_body}");
            let css = format!(
                "body{{margin:0;background:white}}\
                 .fixed{{position:fixed;left:2px;top:2px;background:red}}\
                 .scroller{{width:300px;height:180px;overflow:scroll;background:#ddd}}\
                 .moving{{will-change:transform;transform:translate(3px,4px);\
                     background:blue;width:120px;height:80px}}\
                 .fade{{opacity:.5;background:green;width:120px;height:80px}}\
                 {base_css}"
            );
            let viewport = EngineViewport::from_document(&html, &css);
            let scene = viewport.compositor.as_ref().unwrap_or_else(|| {
                panic!("{page}: expected a live compositor scene")
            });
            assert_eq!(
                scene.borrow().mode(),
                PaintViewRasterMode::RawLocal,
                "{page}: expected the fast raw-raster path"
            );
            let page_ref = viewport.page.as_ref().expect("retained page");
            let images = amazing_paint::image::StyledImages {
                images: &page_ref.resources,
                styles: &page_ref.styles,
            };
            const STEPS: u32 = 8;
            for scenario in ["scroll", "transform", "opacity"] {
                let mut full_us: u128 = 0;
                let mut retained_us: u128 = 0;
                let mut build_cache_hits = 0;
                for index in 0..STEPS {
                    let start = Instant::now();
                    let _ = amazing_paint::paint_document_with_text_resources_and_controls(
                        &page_ref.root,
                        VIEWPORT_WIDTH,
                        VIEWPORT_HEIGHT,
                        &page_ref.text,
                        &images,
                        &amazing_paint::controls::NoControls,
                    );
                    full_us += start.elapsed().as_micros();
                    let start = Instant::now();
                    match scenario {
                        "scroll" => {
                            assert!(viewport
                                .scroll_input(
                                    amazing_compositor::ScrollInput::ScrollTo {
                                        node: scroll,
                                        target: (0., (index + 1) as f32),
                                        behavior: amazing_compositor::ScrollBehavior::Instant,
                                    },
                                    false,
                                )
                                .unwrap_or_else(|error| panic!("{page}/scroll: {error}")));
                        }
                        "transform" => viewport
                            .animate_transform(
                                transform,
                                amazing_compositor::Matrix4::translation(
                                    (index + 1) as f32, 4., 0.,
                                ),
                            )
                            .unwrap_or_else(|error| panic!("{page}/transform: {error}")),
                        "opacity" => viewport
                            .animate_opacity(effect, 0.5 + index as f32 / 100.)
                            .unwrap_or_else(|error| panic!("{page}/opacity: {error}")),
                        _ => unreachable!(),
                    }
                    retained_us += start.elapsed().as_micros();
                    // The property update above already ran through the
                    // compositor's own `build_cached_frame` inside
                    // `rebuild_retained_frame`; re-running it here (cheap,
                    // untimed: no raster happens on a hit) directly proves
                    // that call reused tiles rather than rerastering --
                    // `Ok` means every tile the frame needed was already
                    // cached and non-dirty (`quad.rs`'s `build_frame_inner`
                    // returns `Err(MissingCachedTile)` otherwise).
                    let recheck = scene
                        .borrow_mut()
                        .compositor_mut()
                        .build_cached_frame(VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
                    assert!(
                        recheck.is_ok(),
                        "{page}/{scenario} step {index} rerasterized: {recheck:?}"
                    );
                    build_cache_hits += 1;
                }
                let direct_tiles = viewport
                    .scroll_frame
                    .borrow()
                    .as_ref()
                    .map(|frame| frame.direct_tiles);
                println!(
                    "{page},{scenario},{STEPS},{full_us},{retained_us},\
                     {build_cache_hits},direct_tiles={direct_tiles:?}"
                );
            }
        }
    }
}

#[cfg(test)]
mod font_display_emphasis_controls {
    use super::*;
    use std::time::Duration;
    use amazing_paint::paint::{TextEmphasis, EmphasisShape};
    use amazing_text::{GlyphVisibility, TextSystem, TextError};
    use amazing_text_provider::{FontFaceLoader, FontFetchRequest, FontSourceLoad, FontDisplayTimeline};
    struct PendingSource;
    impl FontFaceLoader for PendingSource {
        fn load_local(&mut self, _: &str, _: usize) -> Result<Option<Vec<u8>>, TextError> { Ok(None) }
        fn load_url(&mut self, _: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> { Ok(None) }
        fn load_url_state(&mut self, _: FontFetchRequest<'_>) -> Result<FontSourceLoad, TextError> { Ok(FontSourceLoad::Pending) }
    }
    struct Marks;
    impl amazing_paint::ImageProvider for Marks {
        fn image(&self, _: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> { None }
        fn text_emphasis(&self, _: amazing_dom::NodeIdx) -> Option<TextEmphasis> {
            Some(TextEmphasis { shape: EmphasisShape::Circle,
                filled: true, color: [255, 0, 0, 255], over: true })
        }
    }
    fn glyphs(root: &amazing_layout::LayoutBox, out: &mut Vec<amazing_layout::GlyphFrag>) {
        for line in &root.lines { out.extend(line.glyphs.iter().cloned()); }
        match &root.flow {
            amazing_layout::Flow::Blocks(children) | amazing_layout::Flow::Flex(children) | amazing_layout::Flow::Grid(children) =>
                for child in children { glyphs(child, out); },
            _ => {},
        }
        for child in &root.positioned { glyphs(child, out); }
    }
    #[test]
    fn native_timed_mixed_range_hides_only_invisible_emphasis_and_keeps_metrics_and_non_text_ink() {
        let dom = amazing_html::parse("<!doctype html><body><div>AB</div></body>");
        let css = "body{margin:0;padding-top:20px;border:2px solid blue;background:white}div{font-family:NavProof;font-size:20px;color:black}";
        let styles = amazing_css::compute(&dom, &[amazing_css::parse_stylesheet(css)]);
        let face = amazing_css::parse_font_faces("@font-face{font-family:NavProof;src:url(font.ttf);font-display:block;unicode-range:U+41}").pop().unwrap();
        let descriptor = page_font_loader::descriptor_from_css(face).unwrap();
        let timeline = FontDisplayTimeline::new(descriptor.clone());
        let mut text = TextProvider::new();
        text.register_font_face_with_timeline(descriptor, &mut PendingSource, &timeline, Duration::ZERO).unwrap();
        let resources = PageResources::default();
        let build = |text: &TextProvider| amazing_layout::layout_styled_document_from_root(
            &dom, amazing_dom::NodeIdx(0), &styles, text, &resources, 100., Some(80.)).unwrap();
        let hidden_root = build(&text); let mut hidden_glyphs = Vec::new(); glyphs(&hidden_root, &mut hidden_glyphs);
        assert_eq!(hidden_glyphs.len(), 2);
        assert_eq!(text.glyph_visibility(hidden_glyphs[0].font).unwrap(), GlyphVisibility::InvisibleFallback);
        assert_eq!(text.glyph_visibility(hidden_glyphs[1].font).unwrap(), GlyphVisibility::Visible);
        let mut hidden_counts = Vec::new();
        for density in [1.0, 2.0] {
            let raster = ViewportMetrics::new(100., 80., density).unwrap().raster_viewport().unwrap();
            let hidden = amazing_paint::paint_document_at_viewport(&hidden_root, raster, &text, &Marks, &amazing_paint::controls::NoControls).unwrap();
            let red = |surface: &Surface| surface.pixels.chunks_exact(4).filter(|pixel| *pixel == [255, 0, 0, 255]).count();
            let hidden_red = red(&hidden); assert!(hidden_red > 0, "visible-range emphasis must still paint");
            assert_eq!(hidden.get_pixel(0, 0), Some([0, 0, 255, 255]), "non-text border ink must remain");
            hidden_counts.push(hidden_red);
        }
        assert!(text.set_font_display_time(Duration::from_millis(3000)).unwrap());
        let visible_root = build(&text); let mut visible_glyphs = Vec::new(); glyphs(&visible_root, &mut visible_glyphs);
        assert_eq!(hidden_glyphs.iter().map(|g| (g.x,g.y,g.advance_x,g.advance_y)).collect::<Vec<_>>(),
            visible_glyphs.iter().map(|g| (g.x,g.y,g.advance_x,g.advance_y)).collect::<Vec<_>>());
        for (index, density) in [1.0, 2.0].into_iter().enumerate() {
            let raster = ViewportMetrics::new(100., 80., density).unwrap().raster_viewport().unwrap();
            let visible = amazing_paint::paint_document_at_viewport(&visible_root, raster, &text, &Marks, &amazing_paint::controls::NoControls).unwrap();
            let red = visible.pixels.chunks_exact(4).filter(|pixel| *pixel == [255, 0, 0, 255]).count();
            assert!(red > hidden_counts[index], "only the formerly invisible range adds emphasis ink after block ends"); assert_eq!(visible.get_pixel(0, 0), Some([0, 0, 255, 255]));
            assert!(visible_glyphs.iter().all(|g| text.glyph_visibility(g.font).unwrap() == GlyphVisibility::Visible));
        }
    }
}
