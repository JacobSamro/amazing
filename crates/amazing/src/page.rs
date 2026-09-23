//! Page resource loading and stylesheet collection.
use crate::cmd::js::resolve_script_url;
#[path = "font_loader.rs"]
mod font_loader;

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
struct PageImage {
    node: amazing_dom::NodeIdx,
    image: amazing_media::Image,
}

#[derive(Default)]
pub(crate) struct PageMedia {
    images: Vec<PageImage>,
    pub(crate) warnings: Vec<String>,
}

pub(crate) struct PageText {
    pub(crate) provider: amazing_text_provider::TextProvider,
    pub(crate) warnings: Vec<String>,
}

const MAX_PAGE_FONT_BYTES: usize = 8 * 1024 * 1024;

pub(crate) fn load_page_text(css: &str, page: &str) -> PageText {
    load_page_text_from_sheets(&[(css.to_owned(), page.to_owned())], page)
}

pub(crate) fn load_page_text_from_sheets(sheets: &[(String, String)], page: &str) -> PageText {
    let mut text = PageText {
        provider: amazing_text_provider::TextProvider::new(),
        warnings: Vec::new(),
    };
    register_system_text_faces(&mut text.provider);
    let expanded = font_loader::expand_font_sheets(sheets, &mut text.warnings);
    for (source, stylesheet_url) in &expanded {
        for face in amazing_css::parse_font_faces(source) {
            let family = face.family.clone();
            let descriptor = match font_loader::descriptor_from_css(face) {
                Ok(value) => value,
                Err(error) => {
                    text.warnings.push(format!("font-face {family:?}: {error}"));
                    continue;
                }
            };
            let mut loader = font_loader::PageFontLoader {
                stylesheet_url,
                document_url: page,
                warnings: Vec::new(),
            };
            let source_labels = descriptor.sources.iter().map(|source| match source {
                amazing_text_provider::FontFaceSource::Local(name) => format!("local({name})"),
                amazing_text_provider::FontFaceSource::Url { url, .. } => url.clone(),
            }).collect::<Vec<_>>().join(", ");
            if let Err(error) = text.provider.begin_font_load(descriptor.display) {
                text.warnings.push(format!("font-face {family:?}: {error:?}"));
                continue;
            }
            let result = text.provider.register_font_face(descriptor, &mut loader);
            text.provider.end_font_load();
            match result {
                Ok(Some(_)) => {},
                Ok(None) => text.warnings.push(format!("font-face {family:?}: no usable source ({source_labels})")),
                Err(error) => text.warnings.push(format!("font-face {family:?}: {error:?}")),
            }
            text.warnings.extend(loader.warnings);
        }
    }
    text
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

fn resolve_font_url(page: &str, source: &str) -> String {
    if std::env::var_os("AMAZING_WPT_ROOT").is_some() {
        return resolve_script_url(page, source);
    }
    if source.starts_with('/')
        && !page.starts_with("http://")
        && !page.starts_with("https://")
        && !page.starts_with("file:")
    {
        let relative = source.trim_start_matches(['/', '\\']);
        let mut ancestor = std::path::Path::new(page).parent();
        while let Some(directory) = ancestor {
            let candidate = directory.join(relative);
            if directory.join(".git").exists() && candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
            ancestor = directory.parent();
        }
    }
    resolve_script_url(page, source)
}

impl amazing_layout::ReplacedElementProvider for PageMedia {
    fn intrinsic_size(&self, node: amazing_dom::NodeIdx) -> Option<amazing_layout::IntrinsicSize> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| amazing_layout::IntrinsicSize {
                width: entry.image.width as f32,
                height: entry.image.height as f32,
            })
    }
}

impl amazing_paint::ImageProvider for PageMedia {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| &entry.image)
    }
}

/// Paint access to the live document and its computed styles for inline SVG.
pub(crate) struct PagePaint<'a> {
    pub(crate) media: &'a PageMedia,
    pub(crate) dom: &'a amazing_dom::Dom,
    pub(crate) styles: &'a amazing_css::StyledDom,
}

impl amazing_paint::ImageProvider for PagePaint<'_> {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::image(self.media, node)
    }

    fn svg(&self, node: amazing_dom::NodeIdx, width: u32, height: u32, dpr: f32) -> Option<amazing_media::Image> {
        if self.dom.namespace(node) != Some(amazing_dom::Namespace::Svg)
            || self.dom.name(node) != Some("svg")
        {
            return None;
        }
        let limits = amazing_svg::DecodeLimits::default();
        let document = amazing_svg::from_dom(self.dom, self.styles, node, width, height, limits).ok()?;
        amazing_svg::render_svg(&document, width, height, dpr, limits).ok()
    }

    fn device_scale(&self) -> f32 {
        1.0
    }
}

pub(crate) fn load_page_media(dom: &amazing_dom::Dom, page: &str) -> PageMedia {
    fn walk(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, page: &str, media: &mut PageMedia) {
        if dom.name(node) == Some("img") {
            let source = dom
                .attrs_of(node)
                .and_then(|attrs| attrs.iter().find(|(name, _)| name == "src"))
                .map(|(_, value)| value.as_str())
                .filter(|value| !value.trim().is_empty());
            if let Some(source) = source {
                let resolved = resolve_script_url(page, source);
                match load_resource(&resolved, 16 * 1024 * 1024)
                    .and_then(|bytes| {
                        amazing_media::decode_image(&bytes)
                            .map_err(|error| error.to_string())
                    }) {
                    Ok(image) => {
                        media.images.push(PageImage {
                            node,
                            image,
                        });
                    }
                    Err(error) => media
                        .warnings
                        .push(format!("image {source:?} ({resolved}): {error}")),
                }
            }
        }
        for child in dom.children(node) {
            walk(dom, child, page, media);
        }
    }

    let mut media = PageMedia::default();
    walk(dom, amazing_dom::NodeIdx(0), page, &mut media);
    media
}

pub(crate) fn collect_style_text(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    page: &str,
    out: &mut Vec<(String, amazing_css::Stylesheet)>,
) {
    if dom.name(node) == Some("style") {
        let mut source = String::new();
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                source.push_str(data);
            }
        }
        let mut sheet = amazing_css::parse_stylesheet(&source);
        sheet.scope_root = dom.get(node).parent;
        sheet.base_url = Some(page.to_owned());
        out.push((source, sheet));
        return;
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, page, out);
    }
}

pub(crate) fn collect_author_css(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    page: &str,
    out: &mut Vec<(String, String, amazing_css::Stylesheet)>,
) -> Result<(), String> {
    if dom.name(node) == Some("style") {
        let mut source = String::new();
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                source.push_str(data);
            }
        }
        let mut sheet = amazing_css::parse_stylesheet(&source);
        sheet.scope_root = dom.get(node).parent;
        sheet.base_url = Some(page.to_owned());
        out.push((source, page.to_owned(), sheet));
        return Ok(());
    }
    if dom.name(node) == Some("link") {
        let attrs = dom.attrs_of(node).unwrap_or(&[]);
        let rel = attrs
            .iter()
            .find(|(name, _)| name == "rel")
            .map(|(_, value)| value.to_ascii_lowercase())
            .unwrap_or_default();
        if rel
            .split_ascii_whitespace()
            .any(|token| token == "stylesheet")
            && let Some(href) = attrs
                .iter()
                .find(|(name, _)| name == "href")
                .map(|(_, value)| value)
        {
            let url = resolve_script_url(page, href);
            let (bytes, final_url) = if url.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
                (load_resource(&url, 16 * 1024 * 1024)
                    .map_err(|error| format!("stylesheet {href}: {error}"))?, url.clone())
            } else {
                let options = amazing_net::FetchOptions {
                    max_response_bytes: 16 * 1024 * 1024,
                    ..amazing_net::FetchOptions::default()
                };
                let response = amazing_net::fetch(&url, &options)
                    .map_err(|error| format!("stylesheet {href}: {error}"))?;
                (response.body, response.url)
            };
            let source = String::from_utf8_lossy(&bytes).into_owned();
            let mut sheet = amazing_css::parse_stylesheet(&source);
            sheet.scope_root = dom.get(node).parent;
            sheet.base_url = Some(final_url.clone());
            out.push((source, final_url, sheet));
        }
    }
    for child in dom.children(node) {
        collect_author_css(dom, child, page, out)?;
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use amazing_layout::ReplacedElementProvider;

    #[test]
    fn data_urls_supply_image_and_stylesheet_bytes() {
        let dom = amazing_html::parse("<link rel='stylesheet' href='data:text/css,body%7Bcolor%3Ared%7D'><img src='data:image/x-portable-pixmap,P6%0A1%201%0A255%0A%FF%00%00'>");
        let media = load_page_media(&dom, "C:/pages/test.html");
        assert_eq!(media.images.len(), 1, "{:?}", media.warnings);
        assert_eq!((media.images[0].image.width, media.images[0].image.height), (1, 1));
        let mut sheets = Vec::new();
        collect_author_css(&dom, amazing_dom::NodeIdx(0), "C:/pages/test.html", &mut sheets).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].0, "body{color:red}");
        assert_eq!(sheets[0].2.base_url.as_deref(), Some("data:text/css,body%7Bcolor%3Ared%7D"));
    }

    #[test]
    fn inline_stylesheet_uses_document_url_as_base() {
        let dom = amazing_html::parse("<style>p { color: red }</style>");
        let mut sheets = Vec::new();
        collect_author_css(&dom, amazing_dom::NodeIdx(0), "https://example.test/pages/p.html", &mut sheets).unwrap();
        assert_eq!(sheets[0].2.base_url.as_deref(), Some("https://example.test/pages/p.html"));
    }

    #[test]
    fn image_intrinsic_size_ignores_html_and_css_dimensions() {
        let directory = std::env::temp_dir().join(format!(
            "amazing-natural-image-size-{}", std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("pixel.ppm"), b"P6\n2 1\n255\n\xff\x00\x00\x00\xff\x00").unwrap();
        let page = directory.join("page.html");
        for attributes in [
            "", "width=20", "height=30", "width=20 height=30",
            "width=20 height=30 style='width:4px;height:auto'",
        ] {
            let dom = amazing_html::parse(&format!("<img src='pixel.ppm' {attributes}>"));
            let media = load_page_media(&dom, &page.to_string_lossy());
            assert!(media.warnings.is_empty(), "{:?}", media.warnings);
            assert_eq!(media.images.len(), 1);
            let size = media.intrinsic_size(media.images[0].node).unwrap();
            assert_eq!((size.width, size.height), (2.0, 1.0), "{attributes}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
