use std::sync::Arc;

use amazing_layout::font::{FontSource, ParsedFont};
use amazing_net::{FetchOptions, Url, fetch, resolve};
use amazing_paint::{OutlineGlyphProvider, Surface};
use gpui::RenderImage;
use image::{Frame, RgbaImage};

const VIEWPORT_WIDTH: u32 = 960;
const VIEWPORT_HEIGHT: u32 = 570;
const WINDOWS_UI_FONT: &str = r"C:\Windows\Fonts\segoeui.ttf";
const MAX_STYLESHEETS: usize = 32;
const MAX_STYLESHEET_BYTES: usize = 4 * 1024 * 1024;
const MAX_PAGE_IMAGES: usize = 32;

struct PageImage {
    node: amazing_dom::NodeIdx,
    image: amazing_media::Image,
    used_size: amazing_layout::IntrinsicSize,
}

#[derive(Default)]
struct PageResources {
    images: Vec<PageImage>,
}

impl amazing_layout::ReplacedElementProvider for PageResources {
    fn intrinsic_size(
        &self,
        node: amazing_dom::NodeIdx,
    ) -> Option<amazing_layout::IntrinsicSize> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| entry.used_size)
    }
}

impl amazing_paint::ImageProvider for PageResources {
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
    pub width: u32,
    pub height: u32,
    links: Arc<Vec<LinkRegion>>,
}

#[derive(Debug, Clone)]
struct LinkRegion {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    target: String,
}

pub struct LoadedPage {
    pub viewport: EngineViewport,
    pub final_address: String,
    pub title: Option<String>,
    pub favicon: Option<Arc<RenderImage>>,
}

impl EngineViewport {
    pub fn demo() -> Self {
        Self::from_document(DEMO_HTML, DEMO_CSS)
    }

    pub fn new_tab() -> Self {
        Self::from_document(NEW_TAB_HTML, NEW_TAB_CSS)
    }

    pub fn load_address(address: &str) -> Result<LoadedPage, String> {
        if address == "amazing://status" {
            return Ok(LoadedPage {
                viewport: Self::demo(),
                final_address: address.to_string(),
                title: Some("Amazing — Phase 9 status".into()),
                favicon: None,
            });
        }
        if address == "amazing://newtab" {
            return Ok(LoadedPage {
                viewport: Self::new_tab(),
                final_address: address.to_string(),
                title: Some("New tab".into()),
                favicon: None,
            });
        }
        let options = FetchOptions {
            timeout: std::time::Duration::from_secs(3),
        };
        let response = fetch(address, &options).map_err(|error| error.to_string())?;
        let content_type = response
            .content_type()
            .unwrap_or("application/octet-stream");
        if content_type != "text/html" && content_type != "application/xhtml+xml" {
            return Err(format!("cannot render response type {content_type}"));
        }
        let final_address = response.url.clone();
        let html = response
            .text()
            .ok_or_else(|| "response body was not decodable text".to_string())?;
        let dom = amazing_html::parse(&html);
        let css = load_document_styles(&dom, &final_address, &options);
        let resources = load_page_resources(&dom, &final_address, &options);
        let title = document_title(&dom);
        let favicon = load_favicon(&dom, &final_address, &options);
        Ok(LoadedPage {
            viewport: Self::from_dom_with_resources(
                &dom,
                &css,
                &resources,
                Some(&final_address),
            ),
            final_address,
            title,
            favicon,
        })
    }

    pub fn error(address: &str, error: &str) -> Self {
        let safe_address = escape_html(address);
        let safe_error = escape_html(error);
        let html = format!(
            r#"<!doctype html><html><body><main>
            <p class="eyebrow">AMAZING NAVIGATION ERROR</p>
            <h1>This address could not be loaded.</h1>
            <p class="intro">Amazing kept the failure inside the viewport and preserved the requested address.</p>
            <p class="address">{safe_address}</p>
            <p class="note">{safe_error}</p>
            </main></body></html>"#
        );
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
                width: decoded.width,
                height: decoded.height,
                links: Arc::new(Vec::new()),
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
            width,
            height,
            links: Arc::new(Vec::new()),
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
        let (surface, links) = render_dom_with_resources(dom, css, resources, document_url);
        let width = surface.width;
        let height = surface.height;
        let image = Arc::new(surface_to_render_image(surface));
        Self {
            image,
            width,
            height,
            links: Arc::new(links),
        }
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.links
            .iter()
            .rev()
            .find(|region| {
                x >= region.x
                    && x <= region.x + region.width
                    && y >= region.y
                    && y <= region.y + region.height
            })
            .map(|region| region.target.as_str())
    }
}

#[cfg(test)]
fn render_demo_surface() -> Surface {
    render_surface(DEMO_HTML, DEMO_CSS)
}

#[cfg(test)]
fn render_surface(html: &str, css: &str) -> Surface {
    let dom = amazing_html::parse(html);
    render_dom_with_resources(&dom, css, &PageResources::default(), None).0
}

fn render_dom_with_resources(
    dom: &amazing_dom::Dom,
    css: &str,
    resources: &PageResources,
    document_url: Option<&str>,
) -> (Surface, Vec<LinkRegion>) {
    let fonts = load_ui_font();
    let root = amazing_layout::layout_dom_document_with_replaced(
        dom,
        css,
        VIEWPORT_WIDTH as f32,
        &fonts,
        resources,
    );
    let page_height = layout_bottom(&root)
        .ceil()
        .clamp(VIEWPORT_HEIGHT as f32, 8192.0) as u32;
    let glyphs = OutlineGlyphProvider::new(&fonts);
    let links = document_url
        .map(|url| collect_link_regions(dom, &root, url))
        .unwrap_or_default();
    let surface = amazing_paint::paint_document_with_resources(
        &root,
        VIEWPORT_WIDTH,
        page_height,
        &fonts,
        &glyphs,
        resources,
    );
    (surface, links)
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
    ) -> Option<String> {
        loop {
            if dom.name(node) == Some("a")
                && let Some(href) = dom.attrs_of(node).and_then(|attrs| {
                    attrs
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("href"))
                        .map(|(_, value)| value.as_str())
                })
            {
                return resolve_resource_address(document_url, href);
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
                if let Some(target) = target_for_node(dom, word.node, document_url) {
                    regions.push(LinkRegion {
                        x: word.x,
                        y: line.y_top,
                        width: word.width.max(1.0),
                        height: line.height.max(1.0),
                        target,
                    });
                }
            }
            for replaced in &line.replaced {
                if let Some(target) = target_for_node(dom, replaced.node, document_url) {
                    regions.push(LinkRegion {
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
    }

    let mut regions = Vec::new();
    walk(dom, layout, document_url, &mut regions);
    regions
}

fn layout_bottom(layout: &amazing_layout::LayoutBox) -> f32 {
    let mut bottom = layout.border_box.y + layout.border_box.h;
    for line in &layout.lines {
        bottom = bottom.max(line.y_top + line.height);
    }
    let children = match &layout.flow {
        amazing_layout::Flow::Blocks(children)
        | amazing_layout::Flow::Flex(children)
        | amazing_layout::Flow::Grid(children) => children.as_slice(),
        amazing_layout::Flow::Inlines(_) | amazing_layout::Flow::Empty => &[],
    };
    for child in children {
        bottom = bottom.max(layout_bottom(child));
    }
    bottom
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

fn load_document_styles(
    dom: &amazing_dom::Dom,
    document_url: &str,
    options: &FetchOptions,
) -> String {
    let mut css = String::new();
    let mut external_count = 0usize;
    collect_style_sources(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        &mut external_count,
        &mut css,
    );
    css
}

fn collect_style_sources(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    document_url: &str,
    options: &FetchOptions,
    external_count: &mut usize,
    out: &mut String,
) {
    if dom.name(node) == Some("style") {
        collect_inline_styles(dom, node, out);
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
            if let Ok(response) = fetch(&address, options)
                && response.status == 200
                && response
                    .content_type()
                    .is_none_or(|kind| kind == "text/css" || kind == "text/plain")
                && let Some(sheet) = response.text()
            {
                let remaining = MAX_STYLESHEET_BYTES.saturating_sub(out.len());
                if sheet.len() <= remaining {
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
            external_count,
            out,
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
    document_url: &str,
    options: &FetchOptions,
) -> PageResources {
    fn dimension(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, name: &str) -> Option<f32> {
        dom.attrs_of(node)?
            .iter()
            .find(|(attribute, _)| attribute.eq_ignore_ascii_case(name))?
            .1
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .map(|value| value as f32)
    }

    fn walk(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        document_url: &str,
        options: &FetchOptions,
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
            && let Ok(response) = fetch(&address, options)
            && response.status == 200
            && let Some(image) = decode_web_image(&response.body)
        {
            let intrinsic_width = image.width as f32;
            let intrinsic_height = image.height as f32;
            let width_attr = dimension(dom, node, "width");
            let height_attr = dimension(dom, node, "height");
            let used_size = match (width_attr, height_attr) {
                (Some(width), Some(height)) => amazing_layout::IntrinsicSize { width, height },
                (Some(width), None) => amazing_layout::IntrinsicSize {
                    width,
                    height: intrinsic_height * width / intrinsic_width,
                },
                (None, Some(height)) => amazing_layout::IntrinsicSize {
                    width: intrinsic_width * height / intrinsic_height,
                    height,
                },
                (None, None) => amazing_layout::IntrinsicSize {
                    width: intrinsic_width,
                    height: intrinsic_height,
                },
            };
            resources.images.push(PageImage {
                node,
                image,
                used_size,
            });
        }

        for child in dom.children(node) {
            walk(dom, child, document_url, options, resources);
        }
    }

    let mut resources = PageResources::default();
    walk(
        dom,
        amazing_dom::NodeIdx(0),
        document_url,
        options,
        &mut resources,
    );
    resources
}

fn decode_web_image(bytes: &[u8]) -> Option<amazing_media::Image> {
    if let Ok(image) = amazing_media::decode_image(bytes) {
        return Some(image);
    }

    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Some(amazing_media::Image {
        width,
        height,
        pixels: rgba.into_raw(),
    })
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
    let response = fetch(&address, options).ok()?;
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

fn load_ui_font() -> FontSource {
    std::fs::read(WINDOWS_UI_FONT)
        .ok()
        .and_then(|bytes| ParsedFont::parse(&bytes))
        .map(FontSource::Ttf)
        .unwrap_or_else(FontSource::fallback)
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
        let css = load_document_styles(
            &dom,
            &format!("http://127.0.0.1:{port}/pages/index.html"),
            &FetchOptions::default(),
        );
        server.join().unwrap();

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
            &format!("http://127.0.0.1:{port}/pages/index.html"),
            &FetchOptions::default(),
        );
        server.join().unwrap();
        assert_eq!(resources.images.len(), 1);
        assert_eq!(resources.images[0].used_size.width, 8.0);
        assert_eq!(resources.images[0].used_size.height, 4.0);
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
            viewport.link_at(
                region.x + region.width / 2.0,
                region.y + region.height / 2.0
            ),
            Some("https://example.test/next.html")
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
