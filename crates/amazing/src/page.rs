//! Page resource loading and stylesheet collection.
use crate::cmd::js::resolve_script_url;
use std::collections::{HashMap, HashSet};
#[path = "font_loader.rs"]
mod font_loader;
#[path = "page_border_images.rs"]
mod page_border_images;
#[path = "page_background_images.rs"]
mod page_background_images;

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
    url: String,
    html_image: bool,
    density: f32,
    loading: amazing_media::loading::Loading,
    decoding: amazing_media::loading::Decoding,
    request: amazing_media::loading::ImageRequest,
}

#[derive(Clone, Debug)]
pub(crate) struct PageImageState {
    pub(crate) current_src: String,
    pub(crate) complete: bool,
    pub(crate) natural_width: u32,
    pub(crate) natural_height: u32,
}

pub(crate) struct PageImageSnapshot {
    pub(crate) node: amazing_dom::NodeIdx,
    pub(crate) state: PageImageState,
    pub(crate) generation: u64,
    pub(crate) decode: crate::runtime_host_replaced_content::ImageDecodeDecision,
}

#[derive(Default)]
pub(crate) struct PageMedia {
    background_sources:HashMap<(amazing_dom::NodeIdx,String,Option<String>),String>,
    images: Vec<PageImage>,
    decoded: HashMap<String, amazing_media::Image>,
    border_sources: HashMap<(String, Option<String>), String>,
    decoder: crate::runtime_host_replaced_content::UtilityImageDecoder,
    pub(crate) warnings: Vec<String>,
}

impl PageMedia {
    pub(crate) fn image_snapshots(&self) -> Vec<PageImageSnapshot> {
        self.images.iter().filter(|entry| entry.html_image).filter_map(|entry| Some(PageImageSnapshot {
            node: entry.node,
            state: self.image_state(entry.node)?,
            generation: entry.request.generation,
            decode: crate::runtime_host_replaced_content::decode_image_decision(&entry.request),
        })).collect()
    }
    pub(crate) fn has_pending_lazy(&self) -> bool {
        self.images.iter().any(|entry| entry.loading == amazing_media::loading::Loading::Lazy && !entry.request.fetch_started)
    }

    pub(crate) fn image_state(&self, node: amazing_dom::NodeIdx) -> Option<PageImageState> {
        let entry = self.images.iter().find(|entry| entry.node == node && entry.html_image)?;
        let image = self.decoded.get(&entry.url);
        let density = entry.density.max(f32::MIN_POSITIVE);
        Some(PageImageState {
            current_src: entry.url.clone(),
            complete: entry.request.complete,
            natural_width: image.map_or(0, |image| (image.width as f32 / density).round() as u32),
            natural_height: image.map_or(0, |image| (image.height as f32 / density).round() as u32),
        })
    }
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
    // Shared across every @font-face rule on the page: real pages routinely
    // declare several icon-font aliases (Font Awesome 5/6 Free/Brands, plain
    // "FontAwesome") whose `src:` lists repeat the exact same URL, and
    // without this each repeat paid its own network round trip -- see the
    // cache field's doc comment in `font_loader.rs` (IA-LIVE-2).
    let mut font_fetch_cache = std::collections::HashMap::new();
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
                cache: &mut font_fetch_cache,
            };
            let source_labels = descriptor.sources.iter().map(|source| match source {
                amazing_text_provider::FontFaceSource::Local(name) => format!("local({name})"),
                amazing_text_provider::FontFaceSource::Url { url, .. } => font_loader::font_source_label(url),
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

    #[cfg(target_os = "windows")]
    {
        for (path, family) in [
            (r"C:\Windows\Fonts\times.ttf", "serif"),
            (r"C:\Windows\Fonts\times.ttf", "Times"),
            (r"C:\Windows\Fonts\times.ttf", "Times New Roman"),
            (r"C:\Windows\Fonts\arial.ttf", "sans-serif"),
            (r"C:\Windows\Fonts\arial.ttf", "Arial"),
            (r"C:\Windows\Fonts\segoeui.ttf", "system-ui"),
            (r"C:\Windows\Fonts\consola.ttf", "monospace"),
            (r"C:\Windows\Fonts\consola.ttf", "Consolas"),
            // TEXT-21: CSS Fonts 4's standard/system-UI generic families are
            // generic keywords even when quoted as a string. Windows has no
            // distinct "rounded" design either, so ui-rounded rides with
            // Segoe UI alongside ui-sans-serif/system-ui.
            (r"C:\Windows\Fonts\times.ttf", "ui-serif"),
            (r"C:\Windows\Fonts\segoeui.ttf", "ui-sans-serif"),
            (r"C:\Windows\Fonts\consola.ttf", "ui-monospace"),
            (r"C:\Windows\Fonts\segoeui.ttf", "ui-rounded"),
            // font-variant-emoji / UTR #51: a forced emoji presentation
            // (U+FE0F) needs a colour-capable face to select over the
            // declared family's own monochrome glyph for the base code
            // point.
            (r"C:\Windows\Fonts\seguiemj.ttf", "emoji"),
        ] {
            font_loader::register_system_face(provider, path, family, 400, FontSlant::Normal);
        }
    }

    // Windows has fixed, well-known font file paths; Linux does not, and this
    // std-only engine has no fontconfig binding. Without this, generic
    // families (serif/sans-serif/monospace) and every common named family
    // (Arial, Times, Courier, ...) resolved to zero registered faces on
    // Linux, so every glyph rasterized as a full-coverage/.notdef box (see
    // TEXT-17). Walk the standard font directories instead and classify by
    // filename, preferring well-known, broadly-hinted families so the
    // default look is stable across sandboxes with different font sets.
    #[cfg(not(target_os = "windows"))]
    font_loader::register_linux_system_faces(provider);

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
            .and_then(|entry| self.intrinsic_size_by_url(&entry.url).map(|size| amazing_layout::IntrinsicSize {
                width: size.width / entry.density.max(f32::MIN_POSITIVE),
                height: size.height / entry.density.max(f32::MIN_POSITIVE),
            }).or_else(|| (entry.loading == amazing_media::loading::Loading::Lazy)
                .then_some(amazing_layout::IntrinsicSize { width: 300.0, height: 150.0 })))
    }

    fn intrinsic_size_by_url(&self, url: &str) -> Option<amazing_layout::IntrinsicSize> {
        self.decoded.get(url).map(|image| amazing_layout::IntrinsicSize {
            width: image.width as f32,
            height: image.height as f32,
        })
    }
}

impl amazing_paint::ImageProvider for PageMedia {
    fn image_density(&self, node: amazing_dom::NodeIdx) -> f32 {
        self.images.iter().find(|entry| entry.node == node).map(|entry| entry.density).unwrap_or(1.0)
    }
    fn background_image(&self,node:amazing_dom::NodeIdx,url:&str)->Option<&amazing_media::Image> {
        self.background_sources.get(&(node,url.to_owned(),None)).and_then(|url|self.decoded.get(url))
            .or_else(||self.decoded.get(url))
    }
    fn background_image_with_base(&self,node:amazing_dom::NodeIdx,url:&str,base:Option<&str>)->Option<&amazing_media::Image> {
        self.background_sources.get(&(node,url.to_owned(),base.map(str::to_owned))).and_then(|resolved|self.decoded.get(resolved))
            .or_else(||self.decoded.get(url))
    }
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .and_then(|entry| self.image_by_url(&entry.url))
    }

    fn image_by_url(&self, url: &str) -> Option<&amazing_media::Image> {
        self.decoded.get(url)
    }
    fn border_image(&self, _node: Option<amazing_dom::NodeIdx>, url: &str, base: Option<&str>) -> Option<&amazing_media::Image> {
        self.border_sources.get(&(url.to_owned(),base.map(str::to_owned))).and_then(|resolved|self.decoded.get(resolved))
    }
}

/// Paint access to the live document and its computed styles for inline SVG.
pub(crate) struct PagePaint<'a> {
    pub(crate) media: &'a PageMedia,
    pub(crate) dom: &'a amazing_dom::Dom,
    pub(crate) styles: &'a amazing_css::StyledDom,
}

impl amazing_paint::ImageProvider for PagePaint<'_> {
    fn image_density(&self, node: amazing_dom::NodeIdx) -> f32 {
        amazing_paint::ImageProvider::image_density(self.media, node)
    }
    fn background_image(&self,node:amazing_dom::NodeIdx,url:&str)->Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::background_image(self.media,node,url)
    }
    fn background_image_with_base(&self,node:amazing_dom::NodeIdx,url:&str,base:Option<&str>)->Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::background_image_with_base(self.media,node,url,base)
    }
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::image(self.media, node)
    }

    fn image_by_url(&self, url: &str) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::image_by_url(self.media, url)
    }

    fn border_image(&self, node: Option<amazing_dom::NodeIdx>, url: &str, base: Option<&str>) -> Option<&amazing_media::Image> {
        amazing_paint::ImageProvider::border_image(self.media,node,url,base)
    }

    fn presentation(&self, node: amazing_dom::NodeIdx) -> amazing_paint::image::ImagePresentation {
        self.styles.get(node).map(amazing_paint::image::ImagePresentation::from_style).unwrap_or_default()
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

fn load_image(media: &mut PageMedia, url: &str, attempted: &mut HashSet<String>) {
        if !attempted.insert(url.to_owned()) {
            return;
        }
        let bytes = if url.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:")) {
            amazing_net::ParsedUrl::parse(url)
                .map_err(|error| format!("{error:?}"))
                .and_then(|parsed| amazing_fetch::process_data_url(&parsed)
                    .ok_or_else(|| "invalid data URL".to_owned()))
                .and_then(|data| {
                    if data.body.len() > 16 * 1024 * 1024 {
                        return Err("resource size limit".to_owned());
                    }
                    Ok(data.body)
                })
        } else {
            load_resource(url, 16 * 1024 * 1024)
        };
        let image = bytes.and_then(|bytes| media.decoder.decode(&bytes));
        match image {
            Ok(image) => { media.decoded.insert(url.to_owned(), image); }
            Err(error) => media.warnings.push(format!("image {url:?}: {error}")),
        }
}

pub(crate) fn load_page_media(
    dom: &amazing_dom::Dom,
    page: &str,
    styled: &amazing_css::StyledDom,
) -> PageMedia {
    load_page_media_in_viewport(dom, page, styled, 800.0, 1.0)
}

pub(crate) fn load_page_media_in_viewport(
    dom: &amazing_dom::Dom,
    page: &str,
    styled: &amazing_css::StyledDom,
    viewport_width: f32,
    device_pixel_ratio: f32,
) -> PageMedia {

    fn collect_image(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        page: &str,
        media: &mut PageMedia,
        attempted: &mut HashSet<String>,
        viewport_width: f32,
        device_pixel_ratio: f32,
        detached: bool,
    ) {
        let environment = amazing_media::responsive::SourceEnvironment {
            viewport_width,
            device_pixel_ratio,
            slot_width: None,
            allow_auto_sizes: false,
        };
        let selected = crate::runtime_host_replaced_content::select_image_source(
            dom, node, environment,
            |query| crate::runtime_host_replaced_content::matches_image_media(query, viewport_width),
            |mime| crate::runtime_host_replaced_content::supports_image_type(mime, true),
        );
        if let Some(selected) = selected {
            let resolved = resolve_script_url(page, &selected.url);
            let attrs = dom.attrs_of(node).unwrap_or(&[]);
            let attr = |name: &str| attrs.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str());
            let loading = if detached {
                // A detached image is never lazy: it has no viewport to wait for.
                amazing_media::loading::Loading::Eager
            } else {
                amazing_media::loading::Loading::from_attribute(attr("loading"))
            };
            let decoding = amazing_media::loading::Decoding::from_attribute(attr("decoding"));
            let mut request = amazing_media::loading::ImageRequest::default();
            request.select(Some(&resolved));
            if loading == amazing_media::loading::Loading::Eager {
                request.fetch_started = true;
                load_image(media, &resolved, attempted);
                request.finish(request.generation, media.decoded.contains_key(&resolved));
            }
            media.images.push(PageImage { node, url: resolved, html_image: true, density: selected.density,
                loading, decoding, request });
        }
    }

    fn walk(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        page: &str,
        styled: &amazing_css::StyledDom,
        media: &mut PageMedia,
        attempted: &mut HashSet<String>,
        viewport_width: f32,
        device_pixel_ratio: f32,
    ) {
        page_border_images::collect(node, page, styled, media, attempted);
        page_background_images::collect(node,page,styled,media,attempted);
        if dom.name(node) == Some("img") {
            collect_image(dom, node, page, media, attempted, viewport_width, device_pixel_ratio, false);
        } else if let Some(attribute) = match dom.name(node) {
            Some("embed") => Some("src"),
            Some("object") => Some("data"),
            Some("video") => Some("poster"),
            _ => None,
        } {
            if let Some(source) = dom.attrs_of(node).unwrap_or(&[]).iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(attribute))
                .map(|(_, value)| value.as_str())
                .filter(|value| !value.trim().is_empty()) {
                let resolved = resolve_script_url(page, source);
                let mut request = amazing_media::loading::ImageRequest::default();
                request.select(Some(&resolved));
                request.fetch_started = true;
                load_image(media, &resolved, attempted);
                request.finish(request.generation, media.decoded.contains_key(&resolved));
                media.images.push(PageImage {
                    node, url: resolved, html_image: false, density: 1.0,
                    loading: amazing_media::loading::Loading::Eager,
                    decoding: amazing_media::loading::Decoding::Sync,
                    request,
                });
            }
        }
        for pseudo in [
            amazing_css::selectors::PseudoElement::Marker,
            amazing_css::selectors::PseudoElement::Before,
            amazing_css::selectors::PseudoElement::After,
        ] {
            if let Some(content) = styled.generated_content(node, pseudo) {
                for run in &content.runs {
                    if let amazing_css::generated::ContentRun::Image(url) = run {
                        // CSS image functions need their own rasterizer. The
                        // decoded resource cache handles URL images only.
                        let css_image_function = url.split_once('(').is_some_and(|(name, _)| {
                            matches!(name.to_ascii_lowercase().as_str(),
                                "linear-gradient" | "repeating-linear-gradient" |
                                "radial-gradient" | "repeating-radial-gradient" |
                                "conic-gradient" | "repeating-conic-gradient" |
                                "image" | "image-set" | "cross-fade" | "element" | "paint")
                        });
                        if !css_image_function {
                            load_image(media, url, attempted);
                        }
                    }
                }
            }
        }
        for child in dom.children(node) {
            walk(dom, child, page, styled, media, attempted, viewport_width, device_pixel_ratio);
        }
    }

    let mut media = PageMedia::default();
    walk(dom, amazing_dom::NodeIdx(0), page, styled, &mut media, &mut HashSet::new(), viewport_width, device_pixel_ratio);
    // `new Image()` and other script-detached elements load even though the
    // document walk cannot reach them, and their decode() promises must settle.
    // Visit every live arena slot for images the walk did not already collect.
    let collected = media.images.iter().map(|entry| entry.node).collect::<HashSet<_>>();
    let mut detached_attempted = media.decoded.keys().cloned().collect::<HashSet<_>>();
    for index in 0..dom.node_count() {
        let node = amazing_dom::NodeIdx(index as u32);
        if dom.name(node) != Some("img") || collected.contains(&node) {
            continue;
        }
        collect_image(dom, node, page, &mut media, &mut detached_attempted, viewport_width, device_pixel_ratio, true);
    }
    media
}

impl PageMedia {
    /// Fetch lazy images whose laid-out boxes intersect the visible viewport.
    /// The caller relayouts if an image gains intrinsic dimensions.
    pub(crate) fn load_visible_lazy(
        &mut self,
        layout: &amazing_layout::LayoutBox,
        viewport: amazing_media::loading::ViewportRect,
    ) -> usize {
        fn rect_for(layout: &amazing_layout::LayoutBox, node: amazing_dom::NodeIdx) -> Option<amazing_layout::Rect> {
            for line in &layout.lines {
                if let Some(fragment) = line.replaced.iter().find(|fragment| fragment.node == node) {
                    return Some(amazing_layout::Rect { x: fragment.x, y: fragment.y, w: fragment.width, h: fragment.height });
                }
            }
            if layout.node == Some(node) { return Some(layout.content_box); }
            let flow = match &layout.flow {
                amazing_layout::Flow::Blocks(c) | amazing_layout::Flow::Flex(c) | amazing_layout::Flow::Grid(c) => c.as_slice(),
                _ => &[],
            };
            flow.iter().chain(&layout.positioned).find_map(|child| rect_for(child, node))
        }
        let mut ready = Vec::new();
        for (index, entry) in self.images.iter_mut().enumerate() {
            if entry.loading != amazing_media::loading::Loading::Lazy { continue; }
            let rect = rect_for(layout, entry.node).map(|rect| amazing_media::loading::ViewportRect {
                x: rect.x, y: rect.y, width: rect.w, height: rect.h,
            });
            if crate::runtime_host_replaced_content::schedule_image_fetch(
                &mut entry.request, Some(&entry.url), entry.loading, rect, viewport, 0.0,
            ) == crate::runtime_host_replaced_content::ImageFetchDecision::Start {
                ready.push(index);
            }
        }
        let mut attempted = self.decoded.keys().cloned().collect::<HashSet<_>>();
        for index in &ready {
            let url = self.images[*index].url.clone();
            load_image(self, &url, &mut attempted);
            let decoded = self.decoded.contains_key(&url);
            let generation = self.images[*index].request.generation;
            self.images[*index].request.finish(generation, decoded);
        }
        ready.len()
    }
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
        let styled = amazing_css::compute(&dom, &[]);
        let media = load_page_media(&dom, "C:/pages/test.html", &styled);
        assert_eq!(media.images.len(), 1, "{:?}", media.warnings);
        let image = amazing_paint::ImageProvider::image(&media, media.images[0].node).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        let mut sheets = Vec::new();
        collect_author_css(&dom, amazing_dom::NodeIdx(0), "C:/pages/test.html", &mut sheets).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].0, "body{color:red}");
        assert_eq!(sheets[0].2.base_url.as_deref(), Some("data:text/css,body%7Bcolor%3Ared%7D"));
    }

    #[test]
    fn image_resources_for_replaced_elements_share_paint_decoding() {
        let source = "data:image/x-portable-pixmap,P6%0A1%201%0A255%0A%FF%00%00";
        let dom = amazing_html::parse(&format!(
            "<img src='{source}'><embed src='{source}'><object data='{source}'></object><video poster='{source}'></video>"
        ));
        let styled = amazing_css::compute(&dom, &[]);
        let media = load_page_media(&dom, "https://example.test/page", &styled);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        assert_eq!(media.images.len(), 4);
        assert_eq!(media.image_snapshots().len(), 1);
        for entry in &media.images {
            let image = amazing_paint::ImageProvider::image(&media, entry.node).unwrap();
            assert_eq!((image.width, image.height), (1, 1));
        }
    }

    #[test]
    fn picture_and_density_candidates_select_intrinsic_css_size() {
        let base = "data:image/x-portable-pixmap,P6%0A2%201%0A255%0A%FF%00%00%00%FF%00";
        let dom = amazing_html::parse(&format!(
            "<picture><source type='image/avif' srcset='bad.avif'><source type='image/x-portable-pixmap' srcset='{base} 2x'><img src='missing.ppm'></picture>"
        ));
        let styled = amazing_css::compute(&dom, &[]);
        let media = load_page_media_in_viewport(&dom, "https://example.test/page", &styled, 800.0, 2.0);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        let entry = &media.images[0];
        assert_eq!(entry.url, base);
        assert_eq!(media.intrinsic_size(entry.node).unwrap().width, 1.0);
        assert_eq!(media.image_state(entry.node).unwrap().natural_width, 1);
    }

    #[test]
    fn lazy_image_loads_only_after_its_box_enters_viewport() {
        let url = "data:image/x-portable-pixmap,P6%0A1%201%0A255%0A%FF%00%00";
        let dom = amazing_html::parse(&format!("<body><div style='height:900px'></div><img loading=lazy width=20 height=20 src='{url}'></body>"));
        let styled = amazing_css::compute(&dom, &[]);
        let mut media = load_page_media_in_viewport(&dom, "https://example.test/page", &styled, 800.0, 1.0);
        assert!(!media.image_state(media.images[0].node).unwrap().complete);
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_styled_document(&dom, &styled, &text, &media, 800.0, Some(600.0));
        assert_eq!(media.load_visible_lazy(&layout, amazing_media::loading::ViewportRect { x:0.0,y:0.0,width:800.0,height:600.0 }), 0);
        assert_eq!(media.load_visible_lazy(&layout, amazing_media::loading::ViewportRect { x:0.0,y:800.0,width:800.0,height:600.0 }), 1);
        assert!(media.image_state(media.images[0].node).unwrap().complete);
        assert_eq!(media.image_state(media.images[0].node).unwrap().natural_width, 1);
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
            let styled = amazing_css::compute(&dom, &[]);
            let media = load_page_media(&dom, &page.to_string_lossy(), &styled);
            assert!(media.warnings.is_empty(), "{:?}", media.warnings);
            assert_eq!(media.images.len(), 1);
            let size = media.intrinsic_size(media.images[0].node).unwrap();
            assert_eq!((size.width, size.height), (2.0, 1.0), "{attributes}");
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn generated_images_share_the_decoded_url_cache_with_img_nodes() {
        let url = "data:image/x-portable-pixmap,P6%0A2%201%0A255%0A%FF%00%00%00%FF%00";
        let dom = amazing_html::parse(&format!("<img src='{url}'><p>text</p>"));
        let sheet = amazing_css::parse_stylesheet(&format!("p::before {{ content: url('{url}') }}"));
        let styled = amazing_css::compute(&dom, &[sheet]);
        let media = load_page_media(&dom, "https://example.test/page", &styled);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        assert_eq!(media.decoded.len(), 1);
        let img = media.images[0].node;
        let node_image = amazing_paint::ImageProvider::image(&media, img).unwrap();
        let generated_image = amazing_paint::ImageProvider::image_by_url(&media, url).unwrap();
        assert!(std::ptr::eq(node_image, generated_image));
        assert_eq!(media.intrinsic_size(img), media.intrinsic_size_by_url(url));
        assert_eq!(media.intrinsic_size_by_url(url).unwrap().width, 2.0);
        let paint = PagePaint { media: &media, dom: &dom, styles: &styled };
        assert!(std::ptr::eq(
            amazing_paint::ImageProvider::image_by_url(&paint, url).unwrap(),
            generated_image,
        ));
    }

    #[test]
    fn generated_url_pixels_reach_the_rendered_page() {
        let url = "data:image/x-portable-pixmap,P6%0A2%202%0A255%0A%FF%00%00%FF%00%00%FF%00%00%FF%00%00";
        let dom = amazing_html::parse("<body></body>");
        let sheet = amazing_css::parse_stylesheet(&format!(
            "body {{ margin: 0 }} body::before {{ content: url('{url}') }}"
        ));
        let styled = amazing_css::compute(&dom, std::slice::from_ref(&sheet));
        let media = load_page_media(&dom, "https://example.test/page", &styled);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport(
            &dom, &[sheet], 100.0, Some(100.0), &text, &media,
        );
        let fragment = layout.styled_replaced_fragments().next().unwrap();
        assert_eq!(fragment.generated_url.as_deref(), Some(url));
        assert_eq!((fragment.content_box.w, fragment.content_box.h), (2.0, 2.0));
        let paint = PagePaint { media: &media, dom: &dom, styles: &styled };
        let surface = amazing_paint::paint_document_with_text_resources(&layout, 100, 100, &text, &paint);
        assert_eq!(surface.get_pixel(
            (fragment.content_box.x + 1.0) as u32,
            (fragment.content_box.y + 1.0) as u32,
        ), Some([255, 0, 0, 255]));
    }

    #[test]
    fn data_svg_marker_uses_generated_image_cache_and_paints() {
        let url = "data:image/svg+xml,%3Csvg%20xmlns=%22http://www.w3.org/2000/svg%22%20width=%222%22%20height=%222%22%3E%3Crect%20width=%222%22%20height=%222%22%20fill=%22red%22/%3E%3C/svg%3E";
        let dom = amazing_html::parse("<body><ul><li>item</li></ul></body>");
        let sheet = amazing_css::parse_stylesheet(&format!(
            "body {{ margin: 0 }} li {{ list-style-position: inside; list-style-image: url('{url}') }}"
        ));
        let styled = amazing_css::compute(&dom, std::slice::from_ref(&sheet));
        let media = load_page_media(&dom, "https://example.test/page", &styled);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        assert_eq!(media.decoded.len(), 1);
        let image = amazing_paint::ImageProvider::image_by_url(&media, url).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
        let li = (0..dom.node_count()).map(|index| amazing_dom::NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("li")).unwrap();
        assert_eq!(styled.generated_content(li, amazing_css::selectors::PseudoElement::Marker).unwrap().runs,
            vec![amazing_css::generated::ContentRun::Image(url.to_owned())]);
        let text = amazing_text::FallbackTextSystem::new();
        let layout = amazing_layout::layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport(
            &dom, &[sheet], 100.0, Some(100.0), &text, &media,
        );
        assert_eq!(media.intrinsic_size_by_url(url).unwrap().width, 2.0);
        let paint = PagePaint { media: &media, dom: &dom, styles: &styled };
        let surface = amazing_paint::paint_document_with_text_resources(&layout, 100, 100, &text, &paint);
        assert!(surface.pixels.chunks_exact(4).any(|pixel| pixel == [255, 0, 0, 255]),
            "list marker did not paint");
    }
}
