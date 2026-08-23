use std::sync::Arc;

use amazing_layout::font::{FontSource, ParsedFont};
use amazing_net::{FetchOptions, fetch};
use amazing_paint::{OutlineGlyphProvider, Surface, paint_document_with};
use gpui::RenderImage;
use image::{Frame, RgbaImage};

const VIEWPORT_WIDTH: u32 = 960;
const VIEWPORT_HEIGHT: u32 = 570;
const WINDOWS_UI_FONT: &str = r"C:\Windows\Fonts\segoeui.ttf";

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

pub struct EngineViewport {
    pub image: Arc<RenderImage>,
    pub width: u32,
    pub height: u32,
}

impl EngineViewport {
    pub fn demo() -> Self {
        Self::from_document(DEMO_HTML, DEMO_CSS)
    }

    pub fn load_address(address: &str) -> Result<(Self, String), String> {
        if address == "amazing://status" {
            return Ok((Self::demo(), address.to_string()));
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
        let mut css = String::new();
        collect_inline_styles(&dom, amazing_dom::NodeIdx(0), &mut css);
        Ok((Self::from_dom(&dom, &css), final_address))
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
        })
    }

    fn from_document(html: &str, css: &str) -> Self {
        let dom = amazing_html::parse(html);
        Self::from_dom(&dom, css)
    }

    fn from_dom(dom: &amazing_dom::Dom, css: &str) -> Self {
        let surface = render_dom(dom, css);
        let width = surface.width;
        let height = surface.height;
        let image = Arc::new(surface_to_render_image(surface));
        Self {
            image,
            width,
            height,
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
    render_dom(&dom, css)
}

fn render_dom(dom: &amazing_dom::Dom, css: &str) -> Surface {
    let fonts = load_ui_font();
    let root = amazing_layout::layout_dom_document(dom, css, VIEWPORT_WIDTH as f32, &fonts);
    let page_height = layout_bottom(&root)
        .ceil()
        .clamp(VIEWPORT_HEIGHT as f32, 8192.0) as u32;
    let glyphs = OutlineGlyphProvider::new(&fonts);
    paint_document_with(&root, VIEWPORT_WIDTH, page_height, &fonts, &glyphs)
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
    fn tall_documents_keep_their_painted_height() {
        let viewport = EngineViewport::from_document(
            "<main><p>top</p><div></div><p>bottom</p></main>",
            "main { padding: 20px; } div { display: block; height: 900px; }",
        );
        assert_eq!(viewport.width, 960);
        assert!(viewport.height > 900, "height was {}", viewport.height);
    }
}
