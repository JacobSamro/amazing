//! Page resource loading and stylesheet collection.
use crate::cmd::js::resolve_script_url;
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
    use amazing_text::FontSlant;
    use amazing_text_provider::{FontFormat, WebFont};

    let mut text = PageText {
        provider: amazing_text_provider::TextProvider::new(),
        warnings: Vec::new(),
    };
    register_system_text_faces(&mut text.provider);
    for face in amazing_css::parse_font_faces(css) {
        let weight = match face.weight.trim() {
            value if value.eq_ignore_ascii_case("bold") => 700,
            value if value.eq_ignore_ascii_case("normal") => 400,
            value => value.parse::<u16>().ok().filter(|value| (1..=1000).contains(value)).unwrap_or(400),
        };
        let stretch = face
            .stretch
            .trim()
            .strip_suffix('%')
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| (1..=1000).contains(value))
            .unwrap_or(100);
        let slant = if face.style.eq_ignore_ascii_case("italic") {
            FontSlant::Italic
        } else if face.style.to_ascii_lowercase().starts_with("oblique") {
            FontSlant::Oblique
        } else {
            FontSlant::Normal
        };
        let mut errors = Vec::new();
        let mut loaded = false;
        for source in face.sources {
            let format_name = source
                .format
                .as_deref()
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| {
                    source
                        .url
                        .split(['?', '#'])
                        .next()
                        .and_then(|url| url.rsplit('.').next())
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                });
            let format = match format_name.as_str() {
                "woff2" => FontFormat::Woff2,
                "woff" => FontFormat::Woff,
                "opentype" | "truetype" | "ttf" | "otf" => FontFormat::OpenType,
                _ => {
                    errors.push(format!("unsupported format {format_name:?}"));
                    continue;
                },
            };
            let resolved = resolve_font_url(page, &source.url);
            let fetch_options = amazing_net::FetchOptions {
                max_response_bytes: MAX_PAGE_FONT_BYTES,
                ..amazing_net::FetchOptions::default()
            };
            let bytes = match amazing_net::fetch(&resolved, &fetch_options) {
                Ok(response) => response.body,
                Err(error) => {
                    errors.push(format!("{}: {error}", source.url));
                    continue;
                },
            };
            match text.provider.register_web_font(WebFont {
                family: face.family.clone(),
                weight,
                stretch,
                slant,
                format,
                bytes,
            }) {
                Ok(_) => {
                    loaded = true;
                    break;
                },
                Err(error) => errors.push(format!("{}: {error:?}", source.url)),
            }
        }
        if !loaded {
            text.warnings.push(format!(
                "font-face {:?}: {}",
                face.family,
                errors.join("; ")
            ));
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
                match amazing_net::fetch(&resolved, &amazing_net::FetchOptions::default())
                    .map_err(|error| error.to_string())
                    .and_then(|response| {
                        amazing_media::decode_image(&response.body)
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
        out.push((source, sheet));
        return;
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, out);
    }
}

pub(crate) fn collect_author_css(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    page: &str,
    out: &mut Vec<(String, amazing_css::Stylesheet)>,
) -> Result<(), String> {
    if dom.name(node) == Some("style") {
        collect_style_text(dom, node, out);
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
            let response = amazing_net::fetch(&url, &amazing_net::FetchOptions::default())
                .map_err(|error| format!("stylesheet {href}: {error}"))?;
            let source = response
                .text()
                .ok_or_else(|| format!("stylesheet {href} was not text"))?;
            let mut sheet = amazing_css::parse_stylesheet(&source);
            sheet.scope_root = dom.get(node).parent;
            out.push((source, sheet));
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
