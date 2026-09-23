//! Static image reftest artifact. Same engine layout/paint APIs as the shell;
//! deterministic fallback fonts, local resources only, no script execution.
use amazing_dom::{Dom, NodeIdx, NodeKind};
use amazing_layout::{IntrinsicSize, ReplacedElementProvider};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
struct Entry {
    image: amazing_media::Image,
    size: IntrinsicSize,
}
struct Resources(HashMap<u32, Entry>, HashMap<String, amazing_media::Image>);
impl ReplacedElementProvider for Resources {
    fn intrinsic_size(&self, n: NodeIdx) -> Option<IntrinsicSize> {
        self.0.get(&n.0).map(|v| v.size)
    }
}
impl amazing_paint::ImageProvider for Resources {
    fn background_image(&self, _n: NodeIdx, url: &str) -> Option<&amazing_media::Image> { self.1.get(url) }
    fn image(&self, n: NodeIdx) -> Option<&amazing_media::Image> {
        self.0.get(&n.0).map(|v| &v.image)
    }
}
struct PaintResources<'a> {
    resources: &'a Resources,
    dom: &'a Dom,
    styles: &'a amazing_css::StyledDom,
}
impl amazing_paint::ImageProvider for PaintResources<'_> {
    fn image(&self, node: NodeIdx) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::image(self.resources, node)
    }
    fn background_image(&self, node: NodeIdx, url: &str) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::background_image(self.resources, node, url)
    }
    fn svg(&self, node: NodeIdx, width: u32, height: u32, dpr: f32) -> Option<amazing_media::Image> {
        if self.dom.namespace(node) != Some(amazing_dom::Namespace::Svg)
            || self.dom.name(node) != Some("svg")
        {
            return None;
        }
        let limits = amazing_svg::DecodeLimits::default();
        let document = amazing_svg::from_dom(self.dom, self.styles, node, width, height, limits).ok()?;
        amazing_svg::render_svg(&document, width, height, dpr, limits).ok()
    }
    fn device_scale(&self) -> f32 { 1.0 }
    fn presentation(&self, node: NodeIdx) -> amazing_paint::image::ImagePresentation {
        self.styles.get(node)
            .map(amazing_paint::image::ImagePresentation::from_style)
            .unwrap_or_default()
    }
}
fn attr<'a>(dom: &'a Dom, n: NodeIdx, key: &str) -> Option<&'a str> {
    dom.attrs_of(n)?
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
fn resolve(page: &Path, source: &str) -> PathBuf {
    if source.starts_with('/') {
        for parent in page.ancestors() {
            if parent.join(".git").exists() {
                return parent.join(source.trim_start_matches('/'));
            }
        }
    }
    page.parent()
        .unwrap()
        .join(source.split('#').next().unwrap_or(source))
}
fn read_resource(page: &Path, source: &str) -> Option<Vec<u8>> {
    const MAX_BYTES: u64 = 16 * 1024 * 1024;
    if source.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
        let url = amazing_net::ParsedUrl::parse(source).ok()?;
        let data = amazing_fetch::process_data_url(&url)?;
        return (data.body.len() as u64 <= MAX_BYTES).then_some(data.body);
    }
    let path = resolve(page, source);
    if std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    std::fs::read(path).ok()
}
fn visit(dom: &Dom, n: NodeIdx, f: &mut impl FnMut(NodeIdx)) {
    f(n);
    for child in dom.children(n) {
        visit(dom, child, f)
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 6 || args[1] != "render-wpt" {
        return Err("expected render-wpt HTML PPM WIDTH HEIGHT".into());
    }
    let page = Path::new(&args[2]);
    let html = std::fs::read_to_string(page)?;
    let (width, height): (u32, u32) = (args[4].parse()?, args[5].parse()?);
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err("viewport limit".into());
    }
    let dom = amazing_html::parse(&html);
    let mut css = String::new();
    visit(&dom, NodeIdx(0), &mut |n| {
        if dom.name(n) == Some("style") {
            for c in dom.children(n) {
                if let NodeKind::Text { data } = &dom.get(c).kind {
                    css.push_str(data);
                    css.push('\n');
                }
            }
        } else if dom.name(n) == Some("link") && attr(&dom, n, "rel") == Some("stylesheet") {
            if let Some(href) = attr(&dom, n, "href") {
                if let Some(text) = read_resource(page, href).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()) {
                    css.push_str(&text);
                    css.push('\n');
                }
            }
        }
    });
    let mut resources = Resources(HashMap::new(), HashMap::new());
    let mut retained = 0usize;
    visit(&dom, NodeIdx(0), &mut |n| {
        if dom.name(n) != Some("img") || resources.0.len() >= 256 {
            return;
        }
        let Some(src) = attr(&dom, n, "src") else {
            return;
        };
        let Some(bytes) = read_resource(page, src) else {
            return;
        };
        let image = if std::env::var_os("AMAZING_IMAGE_BASELINE").is_some() {
            amazing_media::decode_image(&bytes).ok()
        } else {
            amazing_image::decode(&bytes, Default::default()).ok()
        };
        let Some(image) = image else {
            return;
        };
        if retained + image.pixels.len() > 128 * 1024 * 1024 {
            return;
        }
        retained += image.pixels.len();
        let dimension = |name| {
            attr(&dom, n, name)
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|n| n.is_finite() && *n >= 0.0)
        };
        let (w, h) = (dimension("width"), dimension("height"));
        let Some(size) = amazing_media::ImageSize::resolve(image.width, image.height, w, h) else {
            return;
        };
        let size = IntrinsicSize {
            width: size.width,
            height: size.height,
        };
        resources.0.insert(n.0, Entry { image, size });
    });
    let styles = amazing_css::compute(&dom, &[amazing_css::parse_stylesheet(&css)]);
    let mut attempted_backgrounds = std::collections::HashSet::new();
    visit(&dom, NodeIdx(0), &mut |n| {
        let Some(style) = styles.get(n) else { return; };
        for layer in amazing_paint::background::BackgroundPlan::from_style(style).layers {
            let Some(url) = layer.url else { continue; };
            if attempted_backgrounds.len() >= 256 || !attempted_backgrounds.insert(url.clone()) { continue; }
            let Some(image) = read_resource(page, &url).and_then(|bytes| amazing_image::decode(&bytes, Default::default()).ok()) else { continue; };
            if retained.saturating_add(image.pixels.len()) > 128 * 1024 * 1024 { continue; }
            retained += image.pixels.len();
            resources.1.insert(url, image);
        }
    });
    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_dom_document_with_replaced_in_viewport(
        &dom,
        &css,
        width as f32,
        Some(height as f32),
        &fonts,
        &resources,
    );
    let glyphs = amazing_paint::RectInkProvider;
    let paint_resources = PaintResources { resources: &resources, dom: &dom, styles: &styles };
    let surface = amazing_paint::paint_document_with_resources(
        &root, width, height, &fonts, &glyphs, &paint_resources,
    );
    surface.save_ppm(&args[3])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_image_and_stylesheet_sources_do_not_become_file_paths() {
        let page = Path::new("C:/unavailable/page.html");
        let image = read_resource(page, "data:image/x-portable-pixmap,P6%0A1%201%0A255%0A%FF%00%00").unwrap();
        let decoded = amazing_image::decode(&image, Default::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (1, 1));
        assert_eq!(read_resource(page, "data:text/css;base64,Ym9keXtjb2xvcjpyZWR9").unwrap(), b"body{color:red}");
    }
}
