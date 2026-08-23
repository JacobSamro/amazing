use std::sync::Arc;

use amazing_layout::font::{FontSource, ParsedFont};
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

    pub fn for_address(address: &str) -> Self {
        if address == "amazing://status" {
            return Self::demo();
        }
        let safe_address = escape_html(address);
        let html = format!(
            r#"<!doctype html><html><body><main>
            <p class="eyebrow">AMAZING NAVIGATION</p>
            <h1>Navigation request accepted.</h1>
            <p class="intro">The address bar updated the browser model and asked the Amazing pipeline for a new page surface.</p>
            <p class="address">{safe_address}</p>
            <p class="note">Network fetch and response-driven document loading are the next handoff. This page is still parsed, laid out, and painted by Amazing.</p>
            </main></body></html>"#
        );
        Self::from_document(&html, NAVIGATION_CSS)
    }

    fn from_document(html: &str, css: &str) -> Self {
        let surface = render_surface(html, css);
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

fn render_surface(html: &str, css: &str) -> Surface {
    let fonts = load_ui_font();
    let root = amazing_layout::layout_document(html, css, VIEWPORT_WIDTH as f32, &fonts);
    let glyphs = OutlineGlyphProvider::new(&fonts);
    paint_document_with(&root, VIEWPORT_WIDTH, VIEWPORT_HEIGHT, &fonts, &glyphs)
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
    let mut bgra = surface.pixels;
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = RgbaImage::from_raw(surface.width, surface.height, bgra)
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
        let viewport = EngineViewport::for_address("https://example.test/");
        assert_eq!((viewport.width, viewport.height), (960, 570));
    }
}
