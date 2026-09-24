//! Driver adapter from retained layout damage to paint-owned item deltas.
//!
//! LAYOUT-17 can populate this small input after relayout. The adapter resolves
//! the live DOM arena generation at the update boundary; removed items retain
//! their old key because their DOM node may already be gone.

use amazing_dom::Dom;
use amazing_layout::LayoutBox;
use amazing_paint::controls::ControlProvider;
use amazing_paint::display_list_view::{
    DisplayListView, PaintDamage, PaintFallbackReason, PaintItem, PaintNodeKey, RetainedPaintDelta,
    RetainedPaintView, StablePaintItemKey,
};
use amazing_paint::ImageProvider;
use amazing_text::TextSystem;
use std::{collections::BTreeMap, process::ExitCode, time::Duration};

/// The same browser executable serves the IMAGE utility role. The process
/// library enters the platform sandbox before it accepts decoder requests.
pub(crate) fn run_image_utility_child() -> ExitCode {
    use amazing_process::{bootstrap_child, Capabilities, Role};
    let connection = bootstrap_child(Role::Utility, Capabilities(BTreeMap::new()),
        |connection, _, channel| {
            amazing_process::utility::serve_utility(connection, channel);
            Ok(())
        });
    let Ok(connection) = connection else { return ExitCode::FAILURE; };
    while !connection.is_closed() {
        std::thread::sleep(Duration::from_millis(10));
    }
    ExitCode::SUCCESS
}

#[derive(Default)]
pub(crate) struct UtilityImageDecoder {
    host: Option<amazing_process::ProcessHost>,
}

impl UtilityImageDecoder {
    pub(crate) fn decode(&mut self, bytes: &[u8]) -> Result<amazing_media::Image, String> {
        #[cfg(test)]
        {
            // Unit-test executables do not expose the browser's --type entry.
            return if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'<') {
                amazing_svg::decode(bytes, amazing_svg::DecodeLimits::default()).map_err(|e| e.to_string())
            } else {
                amazing_media::decode_image(bytes).map_err(|e| e.to_string())
            };
        }
        #[cfg(not(test))]
        {
            use amazing_process::{Capabilities, ProcessHost, RestartPolicy};
            if self.host.is_none() {
                let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                match ProcessHost::spawn_sandboxed_utility(&exe,
                    Capabilities(BTreeMap::new()), RestartPolicy::Never) {
                    Ok(host) => self.host = Some(host),
                    Err(error) => {
                        #[cfg(target_os = "linux")]
                        if bytes.starts_with(b"BM") || bytes.starts_with(b"P3")
                            || bytes.starts_with(b"P6") || bytes.starts_with(b"<") {
                            // Some Linux build hosts deny user namespaces. Keep
                            // their original basic-image path; expanded codecs
                            // still require the sandboxed utility process.
                            return if bytes.starts_with(b"<") {
                                amazing_svg::decode(bytes, amazing_svg::DecodeLimits::default())
                                    .map_err(|e| e.to_string())
                            } else {
                                amazing_media::decode_image(bytes).map_err(|e| e.to_string())
                            };
                        }
                        return Err(error.to_string());
                    }
                }
            }
            let result = self.host.as_mut().expect("utility was started")
                .decode_untrusted(amazing_process::utility::IMAGE, bytes, Duration::from_secs(5));
            match result {
                Ok(frame) => Ok(amazing_media::Image {
                    width: frame.width, height: frame.height, pixels: frame.bytes().to_vec(),
                }),
                Err(error) => {
                    self.host = None;
                    Err(error.to_string())
                }
            }
        }
    }
}

/// One affected layout item. `item` carries new geometry, property IDs and
/// display position; the adapter fills its stable key from the live DOM.
/// LAYOUT-17 supplies only affected boxes rather than a next full paint view.
pub(crate) enum LayoutPaintDamage<'a> {
    Remove(StablePaintItemKey),
    Upsert {
        box_: &'a LayoutBox,
        item: PaintItem,
        ordinal: u32,
    },
}

/// Capture the initial full scene while the layout tree and DOM are frozen.
/// The returned scene owns its item pixels and outlives this borrowed view.
pub(crate) fn seed_retained_paint(
    root: &LayoutBox,
    width: u32,
    height: u32,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
    controls: &dyn ControlProvider,
    dom: &Dom,
) -> Result<RetainedPaintView, PaintFallbackReason> {
    let view =
        DisplayListView::with_dom_generations(root, width, height, text, images, controls, dom);
    RetainedPaintView::from_view(&view)
}

/// Convert layout's bounded damage and update only those paint chunks. A
/// missing/reused DOM generation yields a full-rebuild reason before mutation.
pub(crate) fn apply_layout_paint_damage(
    retained: &mut RetainedPaintView,
    damage: &[LayoutPaintDamage<'_>],
    dom: &Dom,
    text: &dyn TextSystem,
    images: &dyn ImageProvider,
) -> Result<RetainedPaintDelta, PaintFallbackReason> {
    let mut paint_damage = Vec::with_capacity(damage.len());
    for entry in damage {
        match entry {
            LayoutPaintDamage::Remove(key) => paint_damage.push(PaintDamage::Remove(*key)),
            LayoutPaintDamage::Upsert {
                box_,
                item,
                ordinal,
            } => {
                let node = box_.node.ok_or(PaintFallbackReason::AmbiguousGeneration)?;
                let generation = dom
                    .node_generation(node)
                    .ok_or(PaintFallbackReason::AmbiguousGeneration)?;
                let mut item = *item;
                item.key = Some(StablePaintItemKey {
                    node: PaintNodeKey { node, generation },
                    phase: item.kind,
                    ordinal: *ordinal,
                });
                paint_damage.push(PaintDamage::Upsert { item, box_ });
            }
        }
    }
    retained.apply_damage(&paint_damage, text, images)
}

/// HTML image-source adapter shared by the driver loaders. The DOM walk is
/// deliberately bounded to the img's preceding siblings in its picture.
pub(crate) fn select_image_source(
    dom: &Dom,
    img: amazing_dom::NodeIdx,
    environment: amazing_media::responsive::SourceEnvironment,
    media_matches: impl Fn(&str) -> bool,
    supports_type: impl Fn(&str) -> bool,
) -> Option<amazing_media::responsive::SelectedSource> {
    use amazing_media::responsive::{ImageSources, PictureSource, select_source_with_sizes};
    if dom.name(img) != Some("img") { return None; }
    let attrs = dom.attrs_of(img)?;
    let attr = |name| attribute(attrs, name);
    let mut picture = Vec::new();
    if let Some(parent) = dom.get(img).parent.filter(|parent| dom.name(*parent) == Some("picture")) {
        for sibling in dom.children(parent) {
            if sibling == img { break; }
            if dom.name(sibling) != Some("source") { continue; }
            let Some(source_attrs) = dom.attrs_of(sibling) else { continue; };
            picture.push(PictureSource {
                srcset: attribute(source_attrs, "srcset").unwrap_or(""),
                sizes: attribute(source_attrs, "sizes"),
                media: attribute(source_attrs, "media"),
                mime_type: attribute(source_attrs, "type"),
            });
        }
    }
    select_source_with_sizes(ImageSources {
        src: attr("src"),
        srcset: attr("srcset"),
        sizes: attr("sizes"),
        picture: &picture,
    }, environment, media_matches, supports_type, |sizes, env, _| {
        amazing_css::parse_sizes_attribute(sizes.unwrap_or(""), &amazing_css::MediaEnvironment {
            viewport_width_px: env.viewport_width,
            device_pixel_ratio: env.device_pixel_ratio,
            ..Default::default()
        }, env.allow_auto_sizes.then_some(env.slot_width).flatten())
    })
}

fn attribute<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}

/// Decision returned to a loader after source selection and layout geometry.
/// A deferred lazy image must be revisited when viewport/scroll changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageFetchDecision { Start, Deferred, Unchanged }

pub(crate) fn schedule_image_fetch(
    request: &mut amazing_media::loading::ImageRequest,
    selected_url: Option<&str>,
    loading: amazing_media::loading::Loading,
    image_rect: Option<amazing_media::loading::ViewportRect>,
    viewport: amazing_media::loading::ViewportRect,
    root_margin: f32,
) -> ImageFetchDecision {
    let changed = request.select(selected_url);
    if selected_url.is_none() || (!changed && request.fetch_started) { return ImageFetchDecision::Unchanged; }
    if amazing_media::loading::should_fetch(loading, image_rect, viewport, root_margin) {
        request.fetch_started = true;
        ImageFetchDecision::Start
    } else {
        ImageFetchDecision::Deferred
    }
}

/// The JS binding's `decode()` promise observes this state, independently of
/// the `decoding` rendering hint. Stale generations reject with EncodingError.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageDecodeDecision { Resolve, Wait(u64), Reject }

pub(crate) fn decode_image_decision(
    request: &amazing_media::loading::ImageRequest,
) -> ImageDecodeDecision {
    if request.current_src.is_none() { return ImageDecodeDecision::Reject; }
    if request.complete {
        if request.decoded { ImageDecodeDecision::Resolve } else { ImageDecodeDecision::Reject }
    } else {
        ImageDecodeDecision::Wait(request.generation)
    }
}

/// Select picture sources with the same media evaluator as CSS @media rules.
pub(crate) fn matches_image_media(query: &str, viewport_width: f32) -> bool {
    let environment = amazing_css::MediaEnvironment {
        viewport_width_px: viewport_width,
        ..Default::default()
    };
    amazing_css::evaluate_media_query(&amazing_css::MediaQueryList::parse(query), &environment)
}

/// The process utility currently supports only BMP/PPM/SVG. Once IPC's image
/// decoder swap lands, callers can request the complete pure-Rust codec set.
pub(crate) fn supports_image_type(mime: &str, full_web_decoders: bool) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/bmp" | "image/x-ms-bmp" | "image/x-portable-pixmap" | "image/svg+xml" => true,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/x-icon"
            if full_web_decoders => true,
        // AVIF needs dav1d and remains a separate codec decision.
        _ => false,
    }
}
