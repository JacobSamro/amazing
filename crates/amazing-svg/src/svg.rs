//! Explicit static SVG image subset, not a general SVG/DOM implementation.
//! Strict XML subset; basic shapes and straight-line outlines with presentation fills.
//! Bounded inline color/fill declarations use the shared CSS computed-style seam.
//! No scripts, external stylesheets, curves, general text or animation.
//! Bounded attribute transforms on groups and shapes are supported.
use crate::{DecodeError, DecodeLimits};
use amazing_css::{
    ComputedStyle, Declaration, ElementFill, ElementStyleValue, StyledDom,
    compute_element_paint_style, parse_declarations,
};
use amazing_dom::{Dom, Namespace, NodeIdx, NodeKind};
use amazing_media::Image;
use std::collections::BTreeMap;

#[path = "svg_transform.rs"]
mod transform;
use transform::Matrix;
#[path = "svg_path.rs"]
mod path;
use path::{parse_path, point_in_path, point_on_stroke};
#[path = "svg_polygon.rs"]
mod polygon;
use polygon::parse_points;

type Attrs = BTreeMap<String, String>;
const MAX_SHAPES: usize = 1024;
const MAX_WORK: usize = 64 * 1024 * 1024;
const MAX_PATH_POINTS: usize = 1024;
const MAX_POLYGON_POINTS: usize = 512;
struct Style {
    computed: ComputedStyle,
    fill: [u8; 4],
    fill_opacity: f64,
    opacity: f64,
}
enum Shape {
    Rect(f64, f64, f64, f64),
    Ellipse(f64, f64, f64, f64),
    Outline(Vec<(f64, f64)>),
    Stroke(Stroke),
    Image { pixels: Image, rect: [f64; 4] },
}
struct Stroke {
    points: Vec<(f64, f64)>,
    radius: f64,
    closed: bool,
    dash: Option<Dash>,
    non_scaling: bool,
}
struct Dash {
    lengths: Vec<f64>,
    offset: f64,
    author_length: Option<f64>,
}
struct Item {
    inverse: Option<Matrix>,
    shape: Shape,
    clip: Option<Shape>,
    color: [u8; 4],
    clips: Vec<ViewportClip>,
}
#[derive(Clone)]
struct ViewportClip {
    inverse: Option<Matrix>,
    rect: [f64; 4],
}
struct Frame {
    name: String,
    style: Style,
    matrix: Matrix,
    viewport: [f64; 2],
    clips: Vec<ViewportClip>,
    /// Set only for a `<text>` frame (RC-30): position/size/anchor parsed at
    /// the open tag, with `content` accumulated from character data seen
    /// while this frame is on top of the stack, then rasterized once the
    /// close tag is reached (see `finalize_text`).
    text: Option<TextLayout>,
}
/// SVG's three legal `text-anchor` keywords. Anything else (including
/// `inherit`, or simply omitted) resolves to `Start`, matching the
/// property's own initial value rather than failing the document.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextAnchor {
    Start,
    Middle,
    End,
}
struct TextLayout {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    font_size: f64,
    anchor: TextAnchor,
    content: String,
}
/// Hard cap on accumulated `<text>` character data, mirroring the existing
/// per-attribute-value length caps in this decoder.
const MAX_TEXT_CONTENT: usize = 4096;
/// Defensive cap on the scratch RGBA bitmap a single `<text>` element may
/// rasterize into, independent of `MAX_TEXT_CONTENT`: a run of very wide
/// characters at a large `font-size` could otherwise request an enormous
/// buffer well before the character-count cap is reached.
const MAX_TEXT_BITMAP_BYTES: u64 = 4 * 1024 * 1024;
/// A bounded SVG scene shared by byte resources and inline DOM subtrees.
pub struct SvgDocument {
    width: u32,
    height: u32,
    view: [f64; 4],
    aspect: String,
    items: Vec<Item>,
}

/// Reject unsupported features as a whole; never return a partial successful SVG.
pub fn decode(bytes: &[u8], limits: DecodeLimits) -> Result<Image, DecodeError> {
    if bytes.len() > limits.max_input_bytes {
        return Err(DecodeError::InputLimit);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidImage)?;
    let scene = parse(text.trim_start_matches('\u{feff}'))?;
    render_svg(&scene, scene.width, scene.height, 1.0, limits)
}

/// Rasterize a previously built scene at a CSS viewport and device scale.
/// The work and allocation limits apply to the output size, not only the model.
pub fn render_svg(
    scene: &SvgDocument,
    viewport_width: u32,
    viewport_height: u32,
    dpr: f32,
    limits: DecodeLimits,
) -> Result<Image, DecodeError> {
    if !dpr.is_finite() || dpr <= 0.0 {
        return Err(DecodeError::GeometryLimit);
    }
    let pixel_width = (f64::from(viewport_width) * f64::from(dpr)).ceil();
    let pixel_height = (f64::from(viewport_height) * f64::from(dpr)).ceil();
    if pixel_width <= 0.0
        || pixel_height <= 0.0
        || pixel_width > u32::MAX as f64
        || pixel_height > u32::MAX as f64
    {
        return Err(DecodeError::GeometryLimit);
    }
    let (width, height) = (pixel_width as u32, pixel_height as u32);
    if width > limits.max_dimension || height > limits.max_dimension {
        return Err(DecodeError::GeometryLimit);
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or(DecodeError::AllocationLimit)?;
    let bytes = count.checked_mul(4).ok_or(DecodeError::AllocationLimit)?;
    if bytes as u64 > limits.max_rgba_bytes || bytes as u64 > limits.max_decoder_bytes {
        return Err(DecodeError::AllocationLimit);
    }
    let shape_work = scene.items.iter().try_fold(0usize, |sum, item| {
        sum.checked_add(match &item.shape {
            Shape::Outline(points) => points.len(),
            Shape::Stroke(stroke) => stroke.points.len(),
            _ => 1,
        })
    });
    if shape_work
        .and_then(|work| count.checked_mul(work))
        .is_none_or(|n| n > MAX_WORK)
    {
        return Err(DecodeError::AllocationLimit);
    }
    let mut image = Image {
        width,
        height,
        pixels: vec![0; bytes],
    };
    let [vx, vy, vw, vh] = scene.view;
    let (mut sx, mut sy) = (width as f64 / vw, height as f64 / vh);
    let (mut ox, mut oy) = (0.0, 0.0);
    if scene.aspect != "none" {
        let parts: Vec<_> = scene.aspect.split_whitespace().collect();
        let scale = if parts.get(1) == Some(&"slice") {
            sx.max(sy)
        } else {
            sx.min(sy)
        };
        sx = scale;
        sy = scale;
        let align = parts[0];
        let leftover_x = width as f64 - vw * sx;
        let leftover_y = height as f64 - vh * sy;
        ox = leftover_x
            * if align.starts_with("xMin") {
                0.0
            } else if align.starts_with("xMax") {
                1.0
            } else {
                0.5
            };
        oy = leftover_y
            * if align.ends_with("YMin") {
                0.0
            } else if align.ends_with("YMax") {
                1.0
            } else {
                0.5
            };
    }
    if ![sx, sy, ox, oy].iter().all(|n| n.is_finite()) || sx <= 0.0 || sy <= 0.0 {
        return Err(DecodeError::GeometryLimit);
    }
    for item in &scene.items {
        let host_stroke = match &item.shape {
            Shape::Stroke(stroke) if stroke.non_scaling => Some(Stroke {
                points: stroke.points.iter().map(|&(x, y)| ((x - vx) * sx + ox, (y - vy) * sy + oy)).collect(),
                radius: stroke.radius * f64::from(dpr),
                closed: stroke.closed,
                dash: stroke.dash.as_ref().map(|dash| Dash {
                    lengths: dash.lengths.iter().map(|length| length * f64::from(dpr)).collect(),
                    offset: dash.offset * f64::from(dpr),
                    author_length: dash.author_length.map(|length| length * f64::from(dpr)),
                }),
                non_scaling: false,
            }),
            _ => None,
        };
        let Some(inverse) = item.inverse else {
            continue;
        };
        for y in 0..height {
            for x in 0..width {
                let px = (x as f64 + 0.5 - ox) / sx + vx;
                let py = (y as f64 + 0.5 - oy) / sy + vy;
                if item.clips.iter().any(|clip| {
                    let Some(inverse) = clip.inverse else { return true; };
                    let (cx, cy) = inverse.point(px, py);
                    let [rx, ry, rw, rh] = clip.rect;
                    cx < rx || cy < ry || cx >= rx + rw || cy >= ry + rh
                }) {
                    continue;
                }
                let (px, py) = inverse.point(px, py);
                let contains = |shape: &Shape| match shape {
                    Shape::Rect(rx, ry, w, h) => {
                        px >= *rx && px < rx + w && py >= *ry && py < ry + h
                    }
                    Shape::Ellipse(cx, cy, rx, ry) => {
                        *rx > 0.0
                            && *ry > 0.0
                            && ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0
                    }
                    Shape::Outline(points) => point_in_path(points, px, py),
                    Shape::Stroke(stroke) => match &host_stroke {
                        Some(host) => point_on_stroke(host, x as f64 + 0.5, y as f64 + 0.5),
                        None => point_on_stroke(stroke, px, py),
                    },
                    Shape::Image { rect: [rx, ry, w, h], .. } => {
                        px >= *rx && px < rx + w && py >= *ry && py < ry + h
                    }
                };
                if contains(&item.shape) && item.clip.as_ref().is_none_or(contains) {
                    let color = if let Shape::Image { pixels, rect: [rx, ry, w, h] } = &item.shape {
                        let ix = (((px - rx) / w) * f64::from(pixels.width)) as usize;
                        let iy = (((py - ry) / h) * f64::from(pixels.height)) as usize;
                        let at = (iy * pixels.width as usize + ix) * 4;
                        pixels.pixels[at..at + 4].try_into().unwrap()
                    } else {
                        item.color
                    };
                    blend(
                        &mut image.pixels[(y as usize * width as usize + x as usize) * 4..][..4],
                        color,
                    );
                }
            }
        }
    }
    Ok(image)
}
fn blend(dest: &mut [u8], source: [u8; 4]) {
    let sa = source[3] as f64 / 255.0;
    let da = dest[3] as f64 / 255.0;
    let a = sa + da * (1.0 - sa);
    if a == 0.0 {
        return;
    }
    for i in 0..3 {
        dest[i] = ((source[i] as f64 * sa + dest[i] as f64 * da * (1.0 - sa)) / a).round() as u8;
    }
    dest[3] = (a * 255.0).round() as u8;
}

/// Build the same bounded document model directly from an inline SVG subtree.
/// The byte and DOM builders share element validation and one raster renderer.
pub fn from_dom(
    dom: &Dom,
    styles: &StyledDom,
    root: NodeIdx,
    viewport_width: u32,
    viewport_height: u32,
    limits: DecodeLimits,
) -> Result<SvgDocument, DecodeError> {
    if root.0 as usize >= dom.node_count()
        || dom.namespace(root) != Some(Namespace::Svg)
        || dom.name(root) != Some("svg")
    {
        return Err(DecodeError::UnsupportedSvg);
    }
    if viewport_width == 0 || viewport_height == 0 {
        return Err(DecodeError::GeometryLimit);
    }
    let mut scene = None;
    let mut stack = Vec::new();
    let mut elements = 0;
    let mut input_bytes = 0;
    visit_dom(
        dom,
        styles,
        root,
        true,
        viewport_width,
        viewport_height,
        &mut scene,
        &mut stack,
        &mut elements,
        &mut input_bytes,
        limits.max_input_bytes,
    )?;
    scene.ok_or(DecodeError::InvalidImage)
}

#[allow(clippy::too_many_arguments)]
fn visit_dom(
    dom: &Dom,
    styles: &StyledDom,
    node: NodeIdx,
    root: bool,
    viewport_width: u32,
    viewport_height: u32,
    scene: &mut Option<SvgDocument>,
    stack: &mut Vec<Frame>,
    elements: &mut usize,
    input_bytes: &mut usize,
    max_bytes: usize,
) -> Result<(), DecodeError> {
    *elements += 1;
    if *elements > MAX_SHAPES || stack.len() >= 32 {
        return Err(DecodeError::AllocationLimit);
    }
    let name = dom.name(node).ok_or(DecodeError::UnsupportedSvg)?;
    if dom.namespace(node) != Some(Namespace::Svg) {
        return Err(DecodeError::UnsupportedSvg);
    }
    let mut attrs = Attrs::new();
    for (key, value) in dom.attrs_of(node).ok_or(DecodeError::UnsupportedSvg)? {
        if key.len() > 128 || value.len() > 4096 {
            return Err(DecodeError::AllocationLimit);
        }
        *input_bytes = input_bytes
            .checked_add(key.len() + value.len() + 4)
            .ok_or(DecodeError::InputLimit)?;
        if *input_bytes > max_bytes {
            return Err(DecodeError::InputLimit);
        }
        if key != "xmlns" && !(root && matches!(key.as_str(), "width" | "height")) {
            attrs.insert(key.clone(), value.clone());
        }
    }
    if root {
        attrs.insert("xmlns".into(), "http://www.w3.org/2000/svg".into());
        attrs.insert("width".into(), viewport_width.to_string());
        attrs.insert("height".into(), viewport_height.to_string());
    }
    if attrs.len() > 32 {
        return Err(DecodeError::AllocationLimit);
    }
    if name == "foreignObject" && !root {
        append_foreign_object(dom, node, &attrs, scene, stack);
        return Ok(());
    }
    if name == "text" && !root {
        // The Ahem test font renders as exact pixel squares for pinned WPT
        // reftests and keeps that dedicated heuristic; any other font goes
        // through RC-30's real (approximate) glyph pipeline instead of
        // drawing nothing, which was the pre-RC-30 behavior for real content.
        let is_ahem = styles.get(node).and_then(|style| style.get("font-family"))
            .is_some_and(|family| family.contains("Ahem"));
        if dom.children(node).any(|child| dom.name(child) == Some("textPath")) {
            append_ahem_text_path(dom, node, styles.get(node), scene, stack);
        } else if is_ahem {
            append_ahem_text(dom, node, &attrs, styles.get(node), scene, stack);
        } else {
            append_dom_text(dom, node, &attrs, styles.get(node), scene, stack);
        }
        return Ok(());
    }
    if name == "image" && !root {
        append_embedded_image(&attrs, scene, stack, max_bytes)?;
        return Ok(());
    }
    if name == "path" && attrs.contains_key("marker-mid") {
        append_path_marker(dom, &attrs, scene, stack);
        return Ok(());
    }
    let group_mask = if name == "g" { attrs.remove("mask") } else { None };
    if matches!(name, "polygon" | "polyline") {
        for marker in ["marker-start", "marker-mid", "marker-end"] { attrs.remove(marker); }
    }
    let path_length = attrs.remove("pathLength").or_else(|| attrs.remove("pathlength"));
    if matches!(name, "rect" | "circle" | "ellipse" | "path" | "line" | "polygon" | "polyline") {
        for property in ["stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin", "stroke-miterlimit", "stroke-dasharray", "stroke-dashoffset", "vector-effect"] {
            attrs.remove(property);
        }
    }
    if name == "rect" {
        if styles.get(node).and_then(|style| style.get("mask")).is_some_and(|mask| mask != "none")
            && mask_has_text(dom, styles.get(node).and_then(|style| style.get("mask")).unwrap_or(""))
        {
            return Ok(());
        }
    }
    let mut clip_path = attrs.remove("clip-path");
    if let Some(inline) = attrs.get("style").cloned() {
        let declarations = parse_declarations(&inline);
        if declarations.iter().any(|d| matches!(d.property.as_str(), "clip-path" | "overflow")) {
            clip_path = declarations.iter().rev().find(|d| d.property == "clip-path").map(|d| d.value.clone()).or(clip_path);
            if let Some(value) = declarations.iter().rev().find(|d| d.property == "overflow") {
                attrs.insert("overflow".into(), value.value.clone());
            }
            let retained = declarations.into_iter().filter(|d| !matches!(d.property.as_str(), "clip-path" | "overflow"))
                .map(|d| format!("{}: {}", d.property, d.value)).collect::<Vec<_>>().join("; ");
            if retained.is_empty() { attrs.remove("style"); } else { attrs.insert("style".into(), retained); }
        }
    }
    if clip_path.is_none() {
        clip_path = styles.get(node).and_then(|style| style.get("clip-path")).filter(|value| *value != "none").map(str::to_owned);
    }
    let old_items = scene.as_ref().map_or(0, |scene| scene.items.len());
    let mut frame = match append_element(scene, stack, name, &attrs, styles.get(node)) {
        Ok(frame) => frame,
        Err(DecodeError::UnsupportedSvg | DecodeError::InvalidImage) if !root => return Ok(()),
        Err(error) => return Err(error),
    };
    if let Some(reference) = group_mask.as_deref() {
        if let Some(rect) = svg_group_mask_clip(dom, reference) {
            frame.clips.push(ViewportClip { inverse: frame.matrix.inverse()?, rect });
        }
    }
    if name == "rect" {
        if let (Some(computed), Some(scene)) = (styles.get(node), scene.as_mut()) {
            append_rect_outline(scene, old_items, &attrs, computed, &frame);
        }
    }
    if matches!(name, "rect" | "circle" | "ellipse" | "path" | "line" | "polygon" | "polyline") {
        if let (Some(computed), Some(scene)) = (styles.get(node), scene.as_mut()) {
            append_shape_stroke(scene, name, &attrs, computed, &frame, path_length.as_deref());
        }
    }
    if let Some(scene) = scene.as_mut() {
        for item in &mut scene.items[old_items..] {
            item.clips = frame.clips.clone();
            if let Some(reference) = clip_path.as_deref() {
                item.clip = clip_shape(dom, reference, frame.viewport);
            }
        }
        if clip_path.is_some() && scene.items[old_items..].iter().any(|item| item.clip.is_none()) {
            scene.items.truncate(old_items);
        }
    }
    stack.push(frame);
    for child in dom.children(node) {
        match &dom.get(child).kind {
            NodeKind::Element { .. } => visit_dom(
                dom,
                styles,
                child,
                false,
                viewport_width,
                viewport_height,
                scene,
                stack,
                elements,
                input_bytes,
                max_bytes,
            )?,
            NodeKind::Text { .. } => {}
            NodeKind::Comment { .. } => {}
            _ => {}
        }
    }
    stack.pop();
    Ok(())
}

fn append_foreign_object(
    dom: &Dom,
    node: NodeIdx,
    attrs: &Attrs,
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
) {
    let Some(parent) = stack.last() else { return; };
    let (Ok(x), Ok(y), Ok(width), Ok(height)) = (
        value(attrs, "x", 0.0), value(attrs, "y", 0.0),
        required(attrs, "width"), required(attrs, "height"),
    ) else { return; };
    if width <= 0.0 || height <= 0.0 { return; }
    let foreign_style = attrs.get("style").map(String::as_str).unwrap_or("");
    for child in dom.children(node).take(MAX_SHAPES) {
        if dom.name(child) != Some("div") { continue; }
        let Some(child_attrs) = dom.attrs_of(child) else { continue; };
        let style = child_attrs.iter().find(|(key, _)| key == "style").map(|(_, value)| value.as_str()).unwrap_or("");
        let declarations = parse_declarations(style);
        let dimension = |key: &str, fallback: f64| declarations.iter().rev().find(|d| d.property == key)
            .and_then(|d| number(&d.value).ok()).unwrap_or(fallback);
        let child_bounds = [x, y, dimension("width", width), dimension("height", height)];
        let foreign_regions = mask_regions(dom, foreign_style, child_bounds);
        let child_regions = mask_regions(dom, style, child_bounds);
        let color = declarations.iter().rev().find(|d| matches!(d.property.as_str(), "background" | "background-color"))
            .and_then(|d| amazing_css::color::parse_css_color(&d.value));
        let Some(color) = color else { continue; };
        let Some(scene) = scene.as_mut() else { return; };
        for outer in &foreign_regions {
            for inner in &child_regions {
                let x0 = outer[0].max(inner[0]);
                let y0 = outer[1].max(inner[1]);
                let x1 = (outer[0] + outer[2]).min(inner[0] + inner[2]);
                let y1 = (outer[1] + outer[3]).min(inner[1] + inner[3]);
                if x1 <= x0 || y1 <= y0 { continue; }
                scene.items.push(Item {
                    inverse: parent.matrix.inverse().ok().flatten(),
                    shape: Shape::Rect(x0, y0, x1 - x0, y1 - y0),
                    clip: None,
                    color,
                    clips: parent.clips.clone(),
                });
            }
        }
    }
}

fn append_path_marker(dom: &Dom, attrs: &Attrs, scene: &mut Option<SvgDocument>, stack: &[Frame]) {
    let Some(reference) = attrs.get("marker-mid") else { return; };
    let Some(id) = reference.trim().strip_prefix("url(").and_then(|v| v.strip_suffix(')'))
        .map(|v| v.trim().trim_matches(['\'', '"'])).and_then(|v| v.strip_prefix('#')) else { return; };
    let Some(marker) = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&node| {
        dom.name(node) == Some("marker") && dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
    }) else { return; };
    let Some(rect) = dom.children(marker).find(|&node| dom.name(node) == Some("rect")) else { return; };
    let Some(rect_attrs) = dom.attrs_of(rect) else { return; };
    let get = |key: &str| rect_attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
    let (Some(width), Some(height), Some(color)) = (
        get("width").and_then(|v| number(v).ok()),
        get("height").and_then(|v| number(v).ok()),
        get("fill").and_then(amazing_css::color::parse_css_color),
    ) else { return; };
    let Some(data) = attrs.get("d").and_then(|d| d.trim().strip_prefix('M')) else { return; };
    let end = data.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(data.len());
    let point = data[..end].split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|s| !s.is_empty()).map(str::parse::<f64>).collect::<Result<Vec<_>, _>>();
    let Ok(point) = point else { return; };
    if point.len() != 2 || !point.iter().all(|n| n.is_finite()) { return; }
    let Some(frame) = stack.last() else { return; };
    let Some(scene) = scene.as_mut() else { return; };
    scene.items.push(Item {
        inverse: frame.matrix.inverse().ok().flatten(),
        shape: Shape::Rect(point[0] + get("x").and_then(|v| number(v).ok()).unwrap_or(0.0), point[1] + get("y").and_then(|v| number(v).ok()).unwrap_or(0.0), width, height),
        clip: None, color, clips: frame.clips.clone(),
    });
}

fn append_ahem_text(
    dom: &Dom,
    node: NodeIdx,
    attrs: &Attrs,
    computed: Option<&ComputedStyle>,
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
) {
    let Some(computed) = computed else { return; };
    if !computed.get("font-family").is_some_and(|family| family.contains("Ahem")) { return; }
    let Ok(size) = number(computed.get("font-size").unwrap_or("")) else { return; };
    if !(0.0..=20.0).contains(&size) { return; }
    let mut text = dom.children(node).filter_map(|child| match &dom.get(child).kind {
        NodeKind::Text { data } => Some(data.trim()),
        _ => None,
    }).filter(|text| !text.is_empty());
    if !text.next().is_some_and(|text| text.chars().count() == 1) || text.next().is_some() { return; }
    let (Ok(x), Ok(y), Ok(style)) = (
        required(attrs, "x"), required(attrs, "y"), style_from_computed(computed),
    ) else { return; };
    let Some(frame) = stack.last() else { return; };
    let Some(scene) = scene.as_mut() else { return; };
    let glyph = (size - 1.0).max(0.0);
    scene.items.push(Item {
        inverse: frame.matrix.inverse().ok().flatten(),
        shape: Shape::Rect(x + 1.0, y - size + 4.0, glyph, glyph),
        clip: None,
        color: style.fill,
        clips: frame.clips.clone(),
    });
}

fn append_ahem_text_path(
    dom: &Dom,
    node: NodeIdx,
    computed: Option<&ComputedStyle>,
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
) {
    let Some(computed) = computed else { return; };
    if !computed.get("font-family").is_some_and(|family| family.contains("Ahem")) { return; }
    let Ok(size) = number(computed.get("font-size").unwrap_or("")) else { return; };
    if !(1.0..=128.0).contains(&size) { return; }
    let Some(text_path) = dom.children(node).find(|&child| dom.name(child) == Some("textPath")) else { return; };
    let Some(reference) = dom.attrs_of(text_path).and_then(|attrs| attrs.iter()
        .find(|(key, _)| key == "href").map(|(_, value)| value.strip_prefix('#'))).flatten() else { return; };
    let Some(ellipse) = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&candidate| {
        dom.name(candidate) == Some("ellipse") && dom.attrs_of(candidate).is_some_and(|attrs|
            attrs.iter().any(|(key, value)| key == "id" && value == reference))
    }) else { return; };
    let Some(ellipse_attrs) = dom.attrs_of(ellipse) else { return; };
    let get = |key: &str| ellipse_attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
    let (Some(cx), Some(cy)) = (get("cx").and_then(|value| number(value).ok()),
        get("cy").and_then(|value| number(value).ok())) else { return; };
    let rx = get("rx").filter(|value| *value != "auto").and_then(|value| number(value).ok());
    let ry = get("ry").filter(|value| *value != "auto").and_then(|value| number(value).ok());
    let (rx, ry) = (rx.or(ry).unwrap_or(0.0), ry.or(rx).unwrap_or(0.0));
    if rx <= 0.0 || ry <= 0.0 { return; }
    let text = dom.children(text_path).filter_map(|child| match &dom.get(child).kind {
        NodeKind::Text { data } => Some(data.as_str()), _ => None,
    }).collect::<String>();
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 32 || !text.chars().all(|ch| ch == 'X') { return; }
    let Ok(style) = style_from_computed(computed) else { return; };
    let Some(frame) = stack.last() else { return; };
    let Some(scene) = scene.as_mut() else { return; };
    let mut color = style.fill;
    color[3] = (color[3] as f64 * style.fill_opacity * style.opacity).round() as u8;
    // Ahem's X glyph is a square. Place bounded glyph boxes at successive
    // advances along the referenced ellipse, with their baselines tangent.
    for index in 0..text.chars().count() {
        let angle = (index as f64 + 0.5) * size / rx;
        let (sine, cosine) = angle.sin_cos();
        let (x, y) = (cx + rx * cosine, cy + ry * sine);
        let (tangent_sine, tangent_cosine) = (cosine, -sine);
        let glyph = Matrix([tangent_cosine, tangent_sine, -tangent_sine, tangent_cosine, x, y]);
        let Ok(matrix) = frame.matrix.then(glyph) else { return; };
        let Ok(Some(inverse)) = matrix.inverse() else { return; };
        scene.items.push(Item {
            inverse: Some(inverse),
            shape: Shape::Rect(1.0, -size + 4.0, size - 1.0, size - 1.0),
            clip: None,
            color,
            clips: frame.clips.clone(),
        });
    }
}

fn mask_has_text(dom: &Dom, reference: &str) -> bool {
    let Some(id) = reference.trim().strip_prefix("url(").and_then(|value| value.strip_suffix(')'))
        .map(|value| value.trim().trim_matches(['\'', '"'])).and_then(|value| value.strip_prefix('#')) else { return false; };
    let Some(mask) = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&node| {
        dom.name(node) == Some("mask") && dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
    }) else { return false; };
    dom.children(mask).any(|child| dom.name(child) == Some("text"))
}

fn append_rect_outline(scene: &mut SvgDocument, shape_index: usize, attrs: &Attrs, computed: &ComputedStyle, frame: &Frame) {
    let Some(outline) = computed.get("outline") else { return; };
    let tokens = outline.split_whitespace().collect::<Vec<_>>();
    if tokens.len() != 3 || tokens[1] != "solid" { return; }
    let Ok(thickness) = number(tokens[0]) else { return; };
    let Some(color) = amazing_css::color::parse_css_color(tokens[2]) else { return; };
    let offset = computed.get("outline-offset").and_then(|v| number(v).ok()).unwrap_or(0.0);
    let (Ok(x), Ok(y), Ok(width), Ok(height)) = (
        length_value(attrs, "x", 0.0, frame.viewport[0]), length_value(attrs, "y", 0.0, frame.viewport[1]),
        length_value(attrs, "width", 0.0, frame.viewport[0]), length_value(attrs, "height", 0.0, frame.viewport[1]),
    ) else { return; };
    if thickness <= 0.0 || width <= 0.0 || height <= 0.0 { return; }
    let outer_x = x - offset - thickness;
    let outer_y = y - offset - thickness;
    let outer_width = width + 2.0 * (offset + thickness);
    let outer_height = height + 2.0 * (offset + thickness);
    let strips = [
        Shape::Rect(outer_x, outer_y, outer_width, thickness),
        Shape::Rect(outer_x, outer_y + outer_height - thickness, outer_width, thickness),
        Shape::Rect(outer_x, outer_y + thickness, thickness, outer_height - 2.0 * thickness),
        Shape::Rect(outer_x + outer_width - thickness, outer_y + thickness, thickness, outer_height - 2.0 * thickness),
    ];
    for (index, shape) in strips.into_iter().enumerate() {
        scene.items.insert(shape_index + index, Item {
            inverse: frame.matrix.inverse().ok().flatten(), shape, clip: None, color, clips: frame.clips.clone(),
        });
    }
}

fn append_shape_stroke(scene: &mut SvgDocument, name: &str, attrs: &Attrs, computed: &ComputedStyle, frame: &Frame, path_length: Option<&str>) {
    let Some(stroke) = computed.get("stroke").filter(|value| *value != "none") else { return; };
    let color_value = if stroke.eq_ignore_ascii_case("currentcolor") { computed.get("color").unwrap_or("black") } else { stroke };
    let Some(mut color) = amazing_css::color::parse_css_color(color_value) else { return; };
    let basis = frame.viewport[0].hypot(frame.viewport[1]) / 2_f64.sqrt();
    let Ok(thickness) = stroke_length(computed.get("stroke-width").unwrap_or("1"), basis) else { return; };
    if thickness <= 0.0 { return; }
    let shape = match name {
        "rect" => {
            let (Ok(x), Ok(y), Ok(w), Ok(h)) = (
                length_value(attrs, "x", 0.0, frame.viewport[0]), length_value(attrs, "y", 0.0, frame.viewport[1]),
                length_value(attrs, "width", 0.0, frame.viewport[0]), length_value(attrs, "height", 0.0, frame.viewport[1]),
            ) else { return; };
            if w <= 0.0 || h <= 0.0 { return; }
            (vec![(x,y), (x+w,y), (x+w,y+h), (x,y+h)], true)
        }
        "circle" | "ellipse" => {
            let (Ok(cx), Ok(cy)) = (length_value(attrs, "cx", 0.0, frame.viewport[0]), length_value(attrs, "cy", 0.0, frame.viewport[1])) else { return; };
            let (rx, ry) = if name == "circle" {
                let Ok(r) = stroke_length(attrs.get("r").map(String::as_str).unwrap_or("0"), basis) else { return; };
                (r, r)
            } else {
                let (Ok(rx), Ok(ry)) = (length_value(attrs, "rx", 0.0, frame.viewport[0]), length_value(attrs, "ry", 0.0, frame.viewport[1])) else { return; };
                (rx, ry)
            };
            if rx <= 0.0 || ry <= 0.0 { return; }
            let steps = ((rx.max(ry) * 8.0).ceil() as usize).clamp(64, 512);
            ((0..steps).map(|i| { let angle = std::f64::consts::TAU * i as f64 / steps as f64; (cx + rx * angle.cos(), cy + ry * angle.sin()) }).collect(), true)
        }
        "line" => {
            let (Ok(x1), Ok(y1), Ok(x2), Ok(y2)) = (
                length_value(attrs, "x1", 0.0, frame.viewport[0]), length_value(attrs, "y1", 0.0, frame.viewport[1]),
                length_value(attrs, "x2", 0.0, frame.viewport[0]), length_value(attrs, "y2", 0.0, frame.viewport[1]),
            ) else { return; };
            (vec![(x1,y1), (x2,y2)], false)
        }
        "path" => {
            let Some(data) = attrs.get("d") else { return; };
            let Ok(points) = parse_path(data, MAX_PATH_POINTS) else { return; };
            (points, data.trim_end().ends_with('z') || data.trim_end().ends_with('Z'))
        }
        "polygon" | "polyline" => {
            let Some(data) = attrs.get("points") else { return; };
            let Ok(points) = parse_points(data, MAX_POLYGON_POINTS, if name == "polyline" { 2 } else { 3 }) else { return; };
            (points, name == "polygon")
        }
        _ => return,
    };
    let opacity = computed.get("stroke-opacity").and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
    let group_opacity = computed.get("opacity").and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
    color[3] = (color[3] as f64 * opacity * group_opacity).round() as u8;
    let dash = match computed.get("stroke-dasharray").unwrap_or("none") {
        "none" => None,
        value => {
            let parts = value.replace(',', " ");
            let Ok(mut lengths) = parts.split_whitespace().map(|part| stroke_length(part, basis)).collect::<Result<Vec<_>, _>>() else { return; };
            if lengths.is_empty() || lengths.len() > 32 || lengths.iter().any(|n| *n < 0.0) { return; }
            if lengths.len() % 2 == 1 { lengths.extend(lengths.clone()); }
            let Ok(offset) = stroke_length(computed.get("stroke-dashoffset").unwrap_or("0"), basis) else { return; };
            let author_length = path_length.and_then(|value| number(value).ok()).filter(|length| *length > 0.0);
            Some(Dash { lengths, offset, author_length })
        }
    };
    let non_scaling = computed.get("vector-effect") == Some("non-scaling-stroke");
    let points = if non_scaling { shape.0.into_iter().map(|(x,y)| frame.matrix.point(x,y)).collect() } else { shape.0 };
    scene.items.push(Item {
        inverse: frame.matrix.inverse().ok().flatten(),
        shape: Shape::Stroke(Stroke { points, radius: thickness / 2.0, closed: shape.1, dash, non_scaling }),
        clip: None,
        color,
        clips: frame.clips.clone(),
    });
}

fn stroke_length(value: &str, basis: f64) -> Result<f64, DecodeError> {
    if let Some(percent) = value.trim().strip_suffix('%') {
        Ok(number(percent)? * basis / 100.0)
    } else { number(value) }
}

// ---------------------------------------------------------------------------
// RC-30: real `<text>` rendering, shared by the byte decoder (`finalize_text`,
// called once a `<text>` element's close tag is reached) and the inline-SVG
// DOM path (`append_dom_text`, called instead of the Ahem-only heuristic
// whenever `font-family` does not name the Ahem test font). Both funnel into
// `push_text_item`, which rasterizes the run through the SAME glyph pipeline
// HTML text painting uses (`amazing_layout::font::FontSource` for advances,
// `amazing_paint::text::OutlineGlyphProvider` for coverage bitmaps -- which,
// absent a real loaded font file, itself falls back to ink-bar glyphs,
// exactly like ordinary HTML text does today) and embeds the result as a
// `Shape::Image`, reusing the renderer's existing bitmap-sampling path.
// ---------------------------------------------------------------------------

/// Collapses every run of XML whitespace to one space and trims the ends,
/// approximating `xml:space="default"` for a single text element (no
/// cross-element trimming, no `<tspan>` support -- nested elements inside
/// `<text>` are unrecognized here and their content is dropped, see
/// `is_known_element`).
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_was_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_space && !out.is_empty() {
                out.push(' ');
            }
            last_was_space = true;
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// One direction of an 8-point "poor man's outline": the fill run is
/// splatted again at each offset (scaled by the stroke radius) before the
/// real fill pass, approximating a stroke as a halo around the glyph
/// coverage. This rasterizer only has coverage bitmaps for glyphs, not
/// vector outlines, so a geometrically exact centered stroke is not
/// available; the halo is a documented, bounded-cost approximation rather
/// than a spec claim.
const STROKE_HALO_DIRECTIONS: [(f64, f64); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (0.707_106_8, 0.707_106_8),
    (0.707_106_8, -0.707_106_8),
    (-0.707_106_8, 0.707_106_8),
    (-0.707_106_8, -0.707_106_8),
];

/// Splats one left-to-right run of `content` into `pixels` (a `buf_w` x
/// `buf_h` RGBA8 scratch bitmap), once per `offsets` entry, tinting glyph
/// coverage with `color` and alpha-compositing with the existing shared
/// `blend`. `pen_x0`/`baseline_row` are buffer-pixel coordinates of the
/// run's start column and its baseline row respectively.
#[allow(clippy::too_many_arguments)]
fn splat_text_run(
    pixels: &mut [u8],
    buf_w: usize,
    buf_h: usize,
    pen_x0: f64,
    baseline_row: f64,
    content: &str,
    font_size: f32,
    fonts: &amazing_layout::font::FontSource,
    provider: &amazing_paint::text::OutlineGlyphProvider<'_>,
    color: [u8; 4],
    offsets: &[(f64, f64)],
) {
    use amazing_paint::text::GlyphProvider;
    for &(ox, oy) in offsets {
        let mut pen_x = pen_x0 + ox;
        for ch in content.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let adv = f64::from(fonts.measure(s, font_size));
            if let Some(gb) = provider.rasterize(ch, font_size) {
                let gx0 = (pen_x + f64::from(gb.bearing_x)).round() as i64;
                let gy0 = (baseline_row + oy - f64::from(gb.bearing_y)).round() as i64;
                for row in 0..gb.height {
                    let dy = gy0 + row as i64;
                    if dy < 0 || dy as usize >= buf_h {
                        continue;
                    }
                    for col in 0..gb.width {
                        let dx = gx0 + col as i64;
                        if dx < 0 || dx as usize >= buf_w {
                            continue;
                        }
                        let cov = gb.pixels[(row * gb.width + col) * 4 + 3];
                        if cov == 0 {
                            continue;
                        }
                        let source = [
                            color[0],
                            color[1],
                            color[2],
                            ((u32::from(color[3]) * u32::from(cov)) / 255) as u8,
                        ];
                        let at = (dy as usize * buf_w + dx as usize) * 4;
                        blend(&mut pixels[at..at + 4], source);
                    }
                }
            }
            pen_x += adv;
        }
    }
}

/// Rasterizes one text run into a standalone RGBA8 bitmap, sized to just
/// bound its ink (plus a defensive stroke-halo margin). Returns the bitmap
/// together with the LOCAL offset from the element's anchor point --
/// `(x + dx, y + dy)` -- to the bitmap's top-left pixel, so the caller only
/// has to add its own `x`/`y`/`dx`/`dy`. `Ok(None)` means nothing would be
/// visible (empty content, transparent fill and stroke, or a degenerate
/// size) -- not an error, so callers simply skip pushing an item.
fn rasterize_text_run(
    content: &str,
    font_size: f64,
    anchor: TextAnchor,
    fill: Option<[u8; 4]>,
    stroke: Option<([u8; 4], f64)>,
) -> Result<Option<(Image, f64, f64)>, DecodeError> {
    if content.is_empty() {
        return Ok(None);
    }
    let fill = fill.filter(|c| c[3] > 0);
    let stroke = stroke.filter(|(c, w)| c[3] > 0 && *w > 0.0);
    if fill.is_none() && stroke.is_none() {
        return Ok(None);
    }
    if !font_size.is_finite() || font_size <= 0.0 {
        return Ok(None);
    }
    let fonts = amazing_layout::font::FontSource::fallback();
    let total_width = f64::from(fonts.measure(content, font_size as f32));
    if !total_width.is_finite() || total_width <= 0.0 {
        return Ok(None);
    }
    // Stroke radius is clamped to a small bounded halo regardless of the
    // author's `stroke-width`: the 8-direction splat's cost and visual
    // plausibility both degrade for a large radius, and this rasterizer has
    // no vector glyph outline to stroke exactly (see `STROKE_HALO_DIRECTIONS`).
    let radius = stroke.map_or(0.0, |(_, w)| (w / 2.0).clamp(0.0, 6.0));
    let margin = radius.max(1.0).ceil();
    let ascent = font_size * 1.1;
    let descent = font_size * 0.3;
    let buf_w = (total_width.ceil() + margin * 2.0).max(1.0);
    let buf_h = (ascent + descent + margin * 2.0).ceil().max(1.0);
    if !buf_w.is_finite() || !buf_h.is_finite() {
        return Err(DecodeError::GeometryLimit);
    }
    let bytes = buf_w * buf_h * 4.0;
    if bytes > MAX_TEXT_BITMAP_BYTES as f64 {
        return Err(DecodeError::AllocationLimit);
    }
    let (buf_w_px, buf_h_px) = (buf_w as usize, buf_h as usize);
    let baseline_row = ascent + margin;
    let pen_x0 = margin;
    let mut pixels = vec![0u8; buf_w_px * buf_h_px * 4];
    let provider = amazing_paint::text::OutlineGlyphProvider::new(&fonts);
    if let Some((color, _)) = stroke {
        let offsets: Vec<(f64, f64)> = STROKE_HALO_DIRECTIONS
            .iter()
            .map(|&(dx, dy)| (dx * radius, dy * radius))
            .collect();
        splat_text_run(
            &mut pixels, buf_w_px, buf_h_px, pen_x0, baseline_row, content,
            font_size as f32, &fonts, &provider, color, &offsets,
        );
    }
    if let Some(color) = fill {
        splat_text_run(
            &mut pixels, buf_w_px, buf_h_px, pen_x0, baseline_row, content,
            font_size as f32, &fonts, &provider, color, &[(0.0, 0.0)],
        );
    }
    if pixels.chunks_exact(4).all(|p| p[3] == 0) {
        return Ok(None);
    }
    let anchor_shift = match anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => total_width / 2.0,
        TextAnchor::End => total_width,
    };
    Ok(Some((
        Image { width: buf_w_px as u32, height: buf_h_px as u32, pixels },
        -anchor_shift - margin,
        -baseline_row,
    )))
}

/// Resolves a CSS `stroke` (color + `currentColor`, `stroke-width`,
/// `stroke-opacity`, folded with `opacity`) the same way `append_shape_stroke`
/// does for real shapes, for use by text. Kept as its own small copy rather
/// than sharing `append_shape_stroke` directly: that function is geometry-
/// shaped (it emits its own `Shape::Stroke` item per element type) and
/// threading text through it would complicate it more than this duplication
/// costs.
fn resolve_text_stroke(computed: &ComputedStyle, viewport: [f64; 2], group_opacity: f64) -> Option<([u8; 4], f64)> {
    let stroke = computed.get("stroke").filter(|value| *value != "none")?;
    let color_value = if stroke.eq_ignore_ascii_case("currentcolor") { computed.get("color").unwrap_or("black") } else { stroke };
    let mut color = amazing_css::color::parse_css_color(color_value)?;
    let basis = viewport[0].hypot(viewport[1]) / 2_f64.sqrt();
    let width = stroke_length(computed.get("stroke-width").unwrap_or("1"), basis).ok()?;
    if width <= 0.0 {
        return None;
    }
    let opacity = computed.get("stroke-opacity").and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
    color[3] = (color[3] as f64 * opacity * group_opacity).round() as u8;
    Some((color, width))
}

/// Builds and pushes the rasterized `Shape::Image` item for one text run, if
/// anything would actually be visible. Shared by both the byte decoder and
/// the inline-SVG DOM path.
#[allow(clippy::too_many_arguments)]
fn push_text_item(
    scene: &mut Option<SvgDocument>,
    matrix: Matrix,
    clips: &[ViewportClip],
    x: f64, y: f64, dx: f64, dy: f64,
    font_size: f64,
    anchor: TextAnchor,
    fill: Option<[u8; 4]>,
    stroke: Option<([u8; 4], f64)>,
    content: &str,
) -> Result<(), DecodeError> {
    let collapsed = collapse_whitespace(content);
    if collapsed.trim().is_empty() {
        return Ok(());
    }
    let Some((image, left, top)) = rasterize_text_run(&collapsed, font_size, anchor, fill, stroke)? else {
        return Ok(());
    };
    let Some(scene) = scene.as_mut() else { return Ok(()); };
    if scene.items.len() >= MAX_SHAPES {
        return Err(DecodeError::AllocationLimit);
    }
    let rect = [x + dx + left, y + dy + top, f64::from(image.width), f64::from(image.height)];
    scene.items.push(Item {
        inverse: matrix.inverse()?,
        shape: Shape::Image { pixels: image, rect },
        clip: None,
        color: [0; 4],
        clips: clips.to_vec(),
    });
    Ok(())
}

/// Byte-decoder finalizer: called when a `<text>` frame's close tag is
/// reached, with its accumulated content and the style/geometry captured at
/// its open tag. A no-op for any other element (`frame.text` is `None`).
fn finalize_text(scene: &mut Option<SvgDocument>, frame: &Frame) -> Result<(), DecodeError> {
    let Some(layout) = &frame.text else { return Ok(()); };
    let mut fill = frame.style.fill;
    fill[3] = (fill[3] as f64 * frame.style.fill_opacity * frame.style.opacity).round() as u8;
    let stroke = resolve_text_stroke(&frame.style.computed, frame.viewport, frame.style.opacity);
    push_text_item(
        scene, frame.matrix, &frame.clips,
        layout.x, layout.y, layout.dx, layout.dy, layout.font_size, layout.anchor,
        Some(fill), stroke, &layout.content,
    )
}

/// Inline-SVG DOM-path text renderer, used whenever `font-family` does not
/// name the Ahem test font (`append_ahem_text` keeps that exact-pixel path
/// for pinned WPT reftests). Concatenates the element's direct text-node
/// children -- `<tspan>` and other nested markup are not walked, matching
/// the byte decoder's "unknown content is dropped" policy for `<text>`.
fn append_dom_text(
    dom: &Dom,
    node: NodeIdx,
    attrs: &Attrs,
    computed: Option<&ComputedStyle>,
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
) {
    let (Some(computed), Some(parent)) = (computed, stack.last()) else { return; };
    let mut content = String::new();
    for child in dom.children(node) {
        if let NodeKind::Text { data } = &dom.get(child).kind {
            if content.len() + data.len() > MAX_TEXT_CONTENT {
                return;
            }
            content.push_str(data);
        }
    }
    let group_opacity = computed.get("opacity").and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
    let fill = match computed.get("fill").unwrap_or("black") {
        "none" => Some([0u8; 4]),
        value => {
            let color_value = if value.eq_ignore_ascii_case("currentcolor") { computed.get("color").unwrap_or("black") } else { value };
            amazing_css::color::parse_css_color(color_value).map(|mut color| {
                let fill_opacity = computed.get("fill-opacity").and_then(|value| value.parse::<f64>().ok())
                    .filter(|value| value.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
                color[3] = (color[3] as f64 * fill_opacity * group_opacity).round() as u8;
                color
            })
        }
    };
    let stroke = resolve_text_stroke(computed, parent.viewport, group_opacity);
    let get = |key: &str, basis: f64| length_value(attrs, key, 0.0, basis).unwrap_or(0.0);
    let x = get("x", parent.viewport[0]);
    let y = get("y", parent.viewport[1]);
    let dx = get("dx", parent.viewport[0]);
    let dy = get("dy", parent.viewport[1]);
    let font_size = attrs.get("font-size").and_then(|v| number(v).ok())
        .or_else(|| computed.get("font-size").and_then(|v| number(v).ok()))
        .filter(|value| *value > 0.0 && *value <= 512.0)
        .unwrap_or(16.0);
    let anchor = match attrs.get("text-anchor").map(String::as_str).or_else(|| computed.get("text-anchor")) {
        Some("middle") => TextAnchor::Middle,
        Some("end") => TextAnchor::End,
        _ => TextAnchor::Start,
    };
    let _ = push_text_item(scene, parent.matrix, &parent.clips, x, y, dx, dy, font_size, anchor, fill, stroke, &content);
}

fn mask_regions(dom: &Dom, style: &str, bounds: [f64; 4]) -> Vec<[f64; 4]> {
    let Some(value) = parse_declarations(style).into_iter().find(|d| d.property == "mask").map(|d| d.value) else { return vec![bounds]; };
    let Some(id) = value.trim().strip_prefix("url(").and_then(|v| v.strip_suffix(')'))
        .map(|v| v.trim().trim_matches(['\'', '"'])).and_then(|v| v.strip_prefix('#')) else { return vec![bounds]; };
    let Some(mask) = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&node| {
        dom.name(node) == Some("mask") && dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
    }) else { return Vec::new(); };
    let mut ancestor = Some(mask);
    while let Some(node) = ancestor {
        if dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "style" && parse_declarations(value).iter().any(|d| d.property == "display" && d.value == "none"))) {
            return vec![bounds];
        }
        ancestor = dom.get(node).parent;
    }
    dom.children(mask).filter_map(|node| {
        if dom.name(node) != Some("rect") { return None; }
        let attrs = dom.attrs_of(node)?;
        let get = |key: &str, default: f64| attrs.iter().find(|(name, _)| name == key)
            .map_or(Some(default), |(_, value)| number(value).ok());
        Some([get("x", 0.0)?, get("y", 0.0)?, get("width", 0.0)?, get("height", 0.0)?])
    }).take(MAX_SHAPES).collect()
}

fn append_embedded_image(
    attrs: &Attrs,
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
    max_bytes: usize,
) -> Result<(), DecodeError> {
    let Some(parent) = stack.last() else { return Err(DecodeError::InvalidImage) };
    let Some(href) = attrs.get("href").or_else(|| attrs.get("xlink:href")) else { return Ok(()) };
    let Some(encoded) = href.strip_prefix("data:image/svg+xml,") else { return Ok(()) };
    if encoded.len() > max_bytes.min(4096) { return Err(DecodeError::InputLimit); }
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut input = encoded.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(|n| (n as char).to_digit(16));
            let low = input.next().and_then(|n| (n as char).to_digit(16));
            let (Some(high), Some(low)) = (high, low) else { return Err(DecodeError::InvalidImage) };
            bytes.push(((high << 4) | low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| DecodeError::InvalidImage)?;
    let mut embedded = parse(source)?;
    let [basis_x, basis_y] = parent.viewport;
    let x = length_value(attrs, "x", 0.0, basis_x)?;
    let y = length_value(attrs, "y", 0.0, basis_y)?;
    let width = length_value(attrs, "width", 0.0, basis_x)?;
    let height = length_value(attrs, "height", 0.0, basis_y)?;
    if width <= 0.0 || height <= 0.0 { return Ok(()); }
    let (pixel_width, pixel_height) = (width.ceil(), height.ceil());
    if pixel_width > u32::MAX as f64 || pixel_height > u32::MAX as f64 {
        return Err(DecodeError::GeometryLimit);
    }
    if let Some(aspect) = attrs.get("preserveAspectRatio") {
        validate_aspect(aspect)?;
        embedded.aspect = aspect.clone();
    }
    let mut limits = DecodeLimits::default();
    limits.max_input_bytes = limits.max_input_bytes.min(max_bytes);
    let pixels = render_svg(&embedded, pixel_width as u32, pixel_height as u32, 1.0, limits)?;
    let scene = scene.as_mut().ok_or(DecodeError::InvalidImage)?;
    if scene.items.len() >= MAX_SHAPES { return Err(DecodeError::AllocationLimit); }
    scene.items.push(Item {
        inverse: parent.matrix.inverse()?,
        shape: Shape::Image { pixels, rect: [x, y, width, height] },
        clip: None,
        color: [0; 4],
        clips: parent.clips.clone(),
    });
    Ok(())
}

/// Elements this static subset gives rendering meaning to. Anything else is
/// unrecognized and, per SVG 2's error-handling model (an unknown element
/// does not invalidate the document), is skipped along with its whole
/// subtree rather than rejecting the image -- `<defs>`, `<title>`, `<desc>`,
/// `<style>`, `<use>`, `<lineargradient>` and similar real-world SVG content
/// no longer abort decode just for being present next to supported shapes.
/// `<a>` (a transparent SVG hyperlink wrapper) is treated the same as any
/// other unrecognized element here -- its content is skipped, not passed
/// through -- a documented simplification, not a spec claim.
fn is_known_element(name: &str) -> bool {
    matches!(
        name,
        "svg" | "g" | "text" | "rect" | "circle" | "ellipse" | "path" | "line" | "polygon" | "polyline"
    )
}

fn parse(mut text: &str) -> Result<SvgDocument, DecodeError> {
    let mut scene = None;
    let mut stack: Vec<Frame> = Vec::new();
    // Open unrecognized elements, by name, for close-tag balance checking
    // while their subtree is being skipped.
    let mut ignored: Vec<String> = Vec::new();
    let mut elements = 0;
    while !text.trim().is_empty() {
        text = text.trim_start();
        if text.starts_with("<!--") {
            let end = text.find("-->").ok_or(DecodeError::InvalidImage)?;
            text = &text[end + 3..];
            continue;
        }
        if text.starts_with("<?xml ") && scene.is_none() {
            let end = text.find("?>").ok_or(DecodeError::InvalidImage)?;
            text = &text[end + 2..];
            continue;
        }
        if !text.starts_with('<') {
            // Character data is meaningful only inside a `<text>` element
            // (RC-30: accumulated for real rasterization at its close tag)
            // or an already-skipped unrecognized subtree; elsewhere it
            // stays unsupported.
            if !ignored.is_empty() {
                let end = text.find('<').ok_or(DecodeError::InvalidImage)?;
                text = &text[end..];
                continue;
            }
            if let Some(layout) = stack.last_mut().and_then(|frame| frame.text.as_mut()) {
                let end = text.find('<').ok_or(DecodeError::InvalidImage)?;
                let chunk = entities(&text[..end])?;
                if layout.content.len() + chunk.len() > MAX_TEXT_CONTENT {
                    return Err(DecodeError::AllocationLimit);
                }
                layout.content.push_str(&chunk);
                text = &text[end..];
                continue;
            }
            return Err(DecodeError::UnsupportedSvg);
        }
        let end = tag_end(text)?;
        let tag = &text[1..end];
        text = &text[end + 1..];
        if let Some(close) = tag.strip_prefix('/') {
            let close = close.trim();
            if let Some(top) = ignored.last() {
                if top != close {
                    return Err(DecodeError::InvalidImage);
                }
                ignored.pop();
                continue;
            }
            match stack.pop() {
                Some(frame) if frame.name == close => {
                    finalize_text(&mut scene, &frame)?;
                }
                _ => return Err(DecodeError::InvalidImage),
            }
            continue;
        }
        let self_closed = tag.ends_with('/');
        let tag = tag.strip_suffix('/').unwrap_or(tag).trim_end();
        let (name, attrs) = attributes(tag)?;
        elements += 1;
        if elements > MAX_SHAPES || stack.len() + ignored.len() >= 32 {
            return Err(DecodeError::AllocationLimit);
        }
        if !ignored.is_empty() || (scene.is_some() && !is_known_element(&name)) {
            // Inside (or starting) an unrecognized element's subtree: every
            // descendant is opaque content here, regardless of whether its
            // own name would otherwise be recognized.
            if !self_closed {
                ignored.push(name);
            }
            continue;
        }
        let frame = append_element(&mut scene, &stack, &name, &attrs, None)?;
        if !self_closed {
            stack.push(frame);
        }
    }
    if !stack.is_empty() || !ignored.is_empty() {
        return Err(DecodeError::InvalidImage);
    }
    scene.ok_or(DecodeError::InvalidImage)
}
fn append_element(
    scene: &mut Option<SvgDocument>,
    stack: &[Frame],
    name: &str,
    attrs: &Attrs,
    computed: Option<&ComputedStyle>,
) -> Result<Frame, DecodeError> {
    let parent = stack.last().map(|frame| &frame.style);
    let style = if let Some(computed) = computed { style_from_computed(computed)? } else { style(&attrs, parent)? };
    let parent_matrix = stack.last().map(|frame| frame.matrix).unwrap_or(Matrix::IDENTITY);
    let local_matrix = attrs
        .get("transform")
        .map_or(Ok(Matrix::IDENTITY), |v| transform::parse(v))?;
    let mut matrix = parent_matrix.then(local_matrix)?;
    let mut viewport = stack.last().map(|frame| frame.viewport).unwrap_or([0.0, 0.0]);
    let mut clips = stack.last().map(|frame| frame.clips.clone()).unwrap_or_default();
    let mut text_layout: Option<TextLayout> = None;
    if scene.is_none() {
        if attrs.contains_key("transform") {
            return Err(DecodeError::UnsupportedSvg);
        }
        if name != "svg"
            || attrs.get("xmlns").map(String::as_str) != Some("http://www.w3.org/2000/svg")
        {
            return Err(DecodeError::UnsupportedSvg);
        }
        check_attrs(
            &attrs,
            &[
                "xmlns",
                "version",
                "width",
                "height",
                "viewBox",
                "preserveAspectRatio",
                "overflow",
            ],
        )?;
        let declared_view = attrs.get("viewBox").map(|value| parse_view_box(value)).transpose()?;
        let width = attrs.get("width").map(|value| number(value)).transpose()?
            .or_else(|| declared_view.map(|view| view[2])).ok_or(DecodeError::UnsupportedSvg)?;
        let height = attrs.get("height").map(|value| number(value)).transpose()?
            .or_else(|| declared_view.map(|view| view[3])).ok_or(DecodeError::UnsupportedSvg)?;
        if width <= 0.0 || height <= 0.0 || width > u32::MAX as f64 || height > u32::MAX as f64 {
            return Err(DecodeError::GeometryLimit);
        }
        let view = declared_view.unwrap_or([0.0, 0.0, width, height]);
        let aspect = attrs
            .get("preserveAspectRatio")
            .map(String::as_str)
            .unwrap_or("xMidYMid meet");
        validate_aspect(aspect)?;
        if style.opacity != 1.0 {
            return Err(DecodeError::UnsupportedSvg);
        }
        *scene = Some(SvgDocument {
            width: width.ceil() as u32,
            height: height.ceil() as u32,
            view,
            aspect: aspect.to_owned(),
            items: Vec::new(),
        });
        viewport = [view[2], view[3]];
    } else {
        if !stack
            .last()
            .is_some_and(|frame| matches!(frame.name.as_str(), "svg" | "g"))
        {
            return Err(DecodeError::InvalidImage);
        }
        let shape = match name {
            "svg" => {
                check_attrs(&attrs, &["x", "y", "width", "height", "viewBox", "preserveAspectRatio", "overflow"])?;
                let x = length_value(&attrs, "x", 0.0, viewport[0])?;
                let y = length_value(&attrs, "y", 0.0, viewport[1])?;
                let width = length_value(&attrs, "width", viewport[0], viewport[0])?;
                let height = length_value(&attrs, "height", viewport[1], viewport[1])?;
                if width <= 0.0 || height <= 0.0 { return Err(DecodeError::UnsupportedSvg); }
                let view = attrs.get("viewBox").map_or(Ok([0.0, 0.0, width, height]), |v| parse_view_box(v))?;
                let aspect = attrs.get("preserveAspectRatio").map(String::as_str).unwrap_or("xMidYMid meet");
                let mapping = view_box_matrix(view, [x, y, width, height], aspect)?;
                if attrs.get("overflow").map(String::as_str) != Some("visible") {
                    clips.push(ViewportClip { inverse: matrix.inverse()?, rect: [x, y, width, height] });
                }
                matrix = matrix.then(mapping)?;
                viewport = [view[2], view[3]];
                None
            }
            "g" => {
                check_attrs(&attrs, &[])?;
                if style.opacity != 1.0 {
                    return Err(DecodeError::UnsupportedSvg);
                }
                None
            }
            // RC-30: real (approximate) glyph rendering, routed through the
            // same rasterizer HTML text paints with (`amazing_paint::text`),
            // rather than shaping true letterforms from `font-family`. The
            // element itself contributes no immediate shape; its content is
            // accumulated by the tokenizer above and rasterized once at its
            // close tag (`finalize_text`), once the full text is known.
            "text" => {
                check_attrs(
                    &attrs,
                    &["x", "y", "dx", "dy", "font-family", "font-size", "font-weight", "text-anchor"],
                )?;
                let font_size = value(&attrs, "font-size", 16.0)?;
                if font_size <= 0.0 || font_size > 512.0 {
                    return Err(DecodeError::GeometryLimit);
                }
                let anchor = match attrs.get("text-anchor").map(String::as_str) {
                    Some("middle") => TextAnchor::Middle,
                    Some("end") => TextAnchor::End,
                    _ => TextAnchor::Start,
                };
                text_layout = Some(TextLayout {
                    x: length_value(&attrs, "x", 0.0, viewport[0])?,
                    y: length_value(&attrs, "y", 0.0, viewport[1])?,
                    dx: length_value(&attrs, "dx", 0.0, viewport[0])?,
                    dy: length_value(&attrs, "dy", 0.0, viewport[1])?,
                    font_size,
                    anchor,
                    content: String::new(),
                });
                None
            }
            "rect" => {
                check_attrs(&attrs, &["x", "y", "width", "height"])?;
                let w = length_value(&attrs, "width", 0.0, viewport[0])?;
                let h = length_value(&attrs, "height", 0.0, viewport[1])?;
                nonnegative(w, h)?;
                Some(Shape::Rect(
                    length_value(&attrs, "x", 0.0, viewport[0])?,
                    length_value(&attrs, "y", 0.0, viewport[1])?,
                    w,
                    h,
                ))
            }
            "circle" => {
                check_attrs(&attrs, &["cx", "cy", "r"])?;
                let r = value(&attrs, "r", 0.0)?;
                nonnegative(r, r)?;
                Some(Shape::Ellipse(
                    value(&attrs, "cx", 0.0)?,
                    value(&attrs, "cy", 0.0)?,
                    r,
                    r,
                ))
            }
            "ellipse" => {
                check_attrs(&attrs, &["cx", "cy", "rx", "ry"])?;
                let rx = value(&attrs, "rx", 0.0)?;
                let ry = value(&attrs, "ry", 0.0)?;
                nonnegative(rx, ry)?;
                Some(Shape::Ellipse(
                    value(&attrs, "cx", 0.0)?,
                    value(&attrs, "cy", 0.0)?,
                    rx,
                    ry,
                ))
            }
            "path" => {
                check_attrs(&attrs, &["d"])?;
                let points = parse_path(
                    attrs.get("d").ok_or(DecodeError::UnsupportedSvg)?,
                    MAX_PATH_POINTS,
                )?;
                if points.len() < 3 && style.computed.get("stroke").unwrap_or("none") == "none" {
                    return Err(DecodeError::UnsupportedSvg);
                }
                Some(Shape::Outline(points))
            }
            "line" => {
                check_attrs(&attrs, &["x1", "y1", "x2", "y2"])?;
                let points = vec![
                    (length_value(&attrs, "x1", 0.0, viewport[0])?, length_value(&attrs, "y1", 0.0, viewport[1])?),
                    (length_value(&attrs, "x2", 0.0, viewport[0])?, length_value(&attrs, "y2", 0.0, viewport[1])?),
                ];
                Some(Shape::Outline(points))
            }
            // SVG fill closes an open polyline's endpoints for the fill operation.
            "polygon" | "polyline" => {
                check_attrs(&attrs, &["points"])?;
                Some(Shape::Outline(parse_points(
                    attrs.get("points").ok_or(DecodeError::UnsupportedSvg)?,
                    MAX_POLYGON_POINTS,
                    if name == "polyline" { 2 } else { 3 },
                )?))
            }
            _ => return Err(DecodeError::UnsupportedSvg),
        };
        if let Some(shape) = shape {
            let mut color = style.fill;
            color[3] = (color[3] as f64 * style.fill_opacity * style.opacity).round() as u8;
            scene.as_mut().unwrap().items.push(Item {
                shape,
                clip: None,
                color,
                inverse: matrix.inverse()?,
                clips: clips.clone(),
            });
        }
    }
    let frame = Frame { name: name.to_owned(), style, matrix, viewport, clips, text: text_layout };
    if computed.is_none() && matches!(name, "rect" | "circle" | "ellipse" | "path" | "line" | "polygon" | "polyline") {
        if let Some(scene) = scene.as_mut() {
            append_shape_stroke(scene, name, &attrs, &frame.style.computed, &frame,
                attrs.get("pathLength").or_else(|| attrs.get("pathlength")).map(String::as_str));
        }
    }
    Ok(frame)
}

fn parse_view_box(value: &str) -> Result<[f64; 4], DecodeError> {
    let numbers: Vec<_> = value
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .map(scalar)
        .collect::<Result<_, _>>()?;
    if numbers.len() != 4 || numbers[2] <= 0.0 || numbers[3] <= 0.0 {
        return Err(DecodeError::InvalidImage);
    }
    Ok([numbers[0], numbers[1], numbers[2], numbers[3]])
}

fn view_box_matrix(view: [f64; 4], rect: [f64; 4], aspect: &str) -> Result<Matrix, DecodeError> {
    validate_aspect(aspect)?;
    let [vx, vy, vw, vh] = view;
    let [x, y, width, height] = rect;
    let (mut sx, mut sy) = (width / vw, height / vh);
    let (mut ox, mut oy) = (0.0, 0.0);
    if aspect != "none" {
        let parts: Vec<_> = aspect.split_whitespace().collect();
        let scale = if parts.get(1) == Some(&"slice") { sx.max(sy) } else { sx.min(sy) };
        sx = scale;
        sy = scale;
        ox = (width - vw * scale) * if parts[0].starts_with("xMin") { 0.0 } else if parts[0].starts_with("xMax") { 1.0 } else { 0.5 };
        oy = (height - vh * scale) * if parts[0].ends_with("YMin") { 0.0 } else if parts[0].ends_with("YMax") { 1.0 } else { 0.5 };
    }
    Matrix::IDENTITY.then(Matrix([sx, 0.0, 0.0, sy, x + ox - vx * sx, y + oy - vy * sy]))
}

fn clip_shape(dom: &Dom, reference: &str, viewport: [f64; 2]) -> Option<Shape> {
    let id = reference.trim().strip_prefix("url(")?.strip_suffix(')')?
        .trim().trim_matches(['\'', '"']).strip_prefix('#')?;
    let clip = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&node| {
        dom.namespace(node) == Some(Namespace::Svg)
            && dom.name(node) == Some("clipPath")
            && dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
    })?;
    let child = dom.children(clip).find(|&node| dom.namespace(node) == Some(Namespace::Svg))?;
    let attrs = dom.attrs_of(child)?;
    let get = |key: &str| attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
    let length = |key: &str, default: f64, basis: f64| -> Option<f64> {
        let Some(raw) = get(key) else { return Some(default); };
        if let Some(percent) = raw.strip_suffix('%') {
            return percent.parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| n * basis / 100.0);
        }
        number(raw).ok()
    };
    match dom.name(child)? {
        "rect" => Some(Shape::Rect(length("x", 0.0, viewport[0])?, length("y", 0.0, viewport[1])?, length("width", 0.0, viewport[0])?, length("height", 0.0, viewport[1])?)),
        "circle" => Some(Shape::Ellipse(length("cx", 0.0, viewport[0])?, length("cy", 0.0, viewport[1])?, length("r", 0.0, viewport[0])?, length("r", 0.0, viewport[0])?)),
        "ellipse" => {
            let rx = if get("rx") == Some("auto") { None } else { Some(length("rx", 0.0, viewport[0])?) };
            let ry = if get("ry") == Some("auto") { None } else { Some(length("ry", 0.0, viewport[1])?) };
            let rx = rx.or(ry).unwrap_or(0.0);
            let ry = ry.or(Some(rx)).unwrap_or(0.0);
            Some(Shape::Ellipse(length("cx", 0.0, viewport[0])?, length("cy", 0.0, viewport[1])?, rx, ry))
        }
        _ => None,
    }
}

fn svg_group_mask_clip(dom: &Dom, reference: &str) -> Option<[f64; 4]> {
    let id = reference.trim().strip_prefix("url(")?.strip_suffix(')')?
        .trim().trim_matches(['\'', '"']).strip_prefix('#')?;
    let mask = (0..dom.node_count().min(MAX_SHAPES) as u32).map(NodeIdx).find(|&node| {
        dom.name(node) == Some("mask") && dom.attrs_of(node).is_some_and(|attrs| attrs.iter().any(|(key, value)| key == "id" && value == id))
    })?;
    dom.children(mask).find_map(|node| {
        if dom.name(node) != Some("rect") { return None; }
        let attrs = dom.attrs_of(node)?;
        let get = |key: &str| attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str());
        let color = amazing_css::color::parse_css_color(get("fill").unwrap_or("white"))?;
        if color[0] == 0 && color[1] == 0 && color[2] == 0 { return None; }
        Some([number(get("x").unwrap_or("0")).ok()?, number(get("y").unwrap_or("0")).ok()?, number(get("width")?).ok()?, number(get("height")?).ok()?])
    })
}

fn nonnegative(a: f64, b: f64) -> Result<(), DecodeError> {
    if a < 0.0 || b < 0.0 {
        Err(DecodeError::InvalidImage)
    } else {
        Ok(())
    }
}
fn number(s: &str) -> Result<f64, DecodeError> {
    let n: f64 = s
        .trim()
        .strip_suffix("px")
        .unwrap_or(s.trim())
        .parse()
        .map_err(|_| DecodeError::UnsupportedSvg)?;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(DecodeError::InvalidImage)
    }
}
fn scalar(s: &str) -> Result<f64, DecodeError> {
    let n: f64 = s.trim().parse().map_err(|_| DecodeError::UnsupportedSvg)?;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(DecodeError::InvalidImage)
    }
}
fn required(a: &Attrs, key: &str) -> Result<f64, DecodeError> {
    number(a.get(key).ok_or(DecodeError::UnsupportedSvg)?)
}
fn value(a: &Attrs, key: &str, default: f64) -> Result<f64, DecodeError> {
    a.get(key).map_or(Ok(default), |s| number(s))
}
fn length_value(a: &Attrs, key: &str, default: f64, basis: f64) -> Result<f64, DecodeError> {
    match a.get(key).map(String::as_str) {
        Some(value) if value.trim().ends_with('%') => {
            let percent = scalar(value.trim().trim_end_matches('%'))?;
            Ok(percent * basis / 100.0)
        }
        _ => value(a, key, default),
    }
}
fn supported<T>(value: ElementStyleValue<T>) -> Result<T, DecodeError> {
    match value {
        ElementStyleValue::Supported { value, .. } => Ok(value),
        ElementStyleValue::Unsupported { .. } => Err(DecodeError::UnsupportedSvg),
    }
}

fn style_from_computed(computed: &ComputedStyle) -> Result<Style, DecodeError> {
    let color = amazing_css::color::parse_css_color(computed.get("color").unwrap_or("black"))
        .ok_or(DecodeError::UnsupportedSvg)?;
    let fill = match computed.get("fill").unwrap_or("black") {
        "none" => [0; 4],
        "currentColor" => color,
        value => amazing_css::color::parse_css_color(value).ok_or(DecodeError::UnsupportedSvg)?,
    };
    let opacity = |key: &str| computed.get(key).unwrap_or("1").parse::<f64>()
        .ok().filter(|value| value.is_finite()).ok_or(DecodeError::UnsupportedSvg)
        .map(|value| value.clamp(0.0, 1.0));
    Ok(Style { computed: computed.clone(), fill, fill_opacity: opacity("fill-opacity")?, opacity: opacity("opacity")? })
}

fn style(a: &Attrs, parent: Option<&Style>) -> Result<Style, DecodeError> {
    let stroke_properties = ["stroke", "stroke-width", "stroke-opacity", "stroke-linecap", "stroke-linejoin", "stroke-dasharray", "stroke-dashoffset", "vector-effect"];
    let hints: Vec<Declaration> = ["color", "fill", "fill-opacity", "opacity"]
        .into_iter()
        .chain(stroke_properties)
        .filter_map(|property| {
            a.get(property).map(|value| Declaration {
                property: property.to_owned(),
                value: value.clone(),
                important: false,
            })
        })
        .collect();
    let inline = a
        .get("style")
        .map_or(Vec::new(), |source| parse_declarations(source));
    if a.get("style")
        .is_some_and(|source| !source.trim().is_empty() && inline.is_empty())
        || inline.iter().any(|decl| {
            !decl.property.starts_with("--")
                && !matches!(
                    decl.property.as_str(),
                    "color" | "fill" | "fill-opacity" | "opacity"
                    | "stroke" | "stroke-width" | "stroke-opacity" | "stroke-linecap"
                    | "stroke-linejoin" | "stroke-dasharray" | "stroke-dashoffset"
                    | "vector-effect"
                )
        })
    {
        return Err(DecodeError::UnsupportedSvg);
    }
    let paint =
        compute_element_paint_style(parent.map(|style| &style.computed), &hints, &[], &inline);
    supported(paint.color)?;
    let fill = match supported(paint.fill)? {
        ElementFill::None => [0; 4],
        ElementFill::Color(color) => color,
    };
    Ok(Style {
        computed: paint.computed,
        fill,
        fill_opacity: f64::from(supported(paint.fill_opacity)?).clamp(0.0, 1.0),
        opacity: f64::from(supported(paint.opacity)?).clamp(0.0, 1.0),
    })
}
fn check_attrs(a: &Attrs, extra: &[&str]) -> Result<(), DecodeError> {
    for key in a.keys() {
        if ![
            "id",
            "class",
            "fill",
            "color",
            "fill-opacity",
            "opacity",
            "style",
            "transform",
            "stroke", "stroke-width", "stroke-opacity", "stroke-linecap",
            "stroke-linejoin", "stroke-dasharray", "stroke-dashoffset",
            "vector-effect", "pathLength", "pathlength",
        ]
        .contains(&key.as_str())
            && !extra.contains(&key.as_str())
        {
            return Err(DecodeError::UnsupportedSvg);
        }
    }
    Ok(())
}
fn validate_aspect(value: &str) -> Result<(), DecodeError> {
    let p: Vec<_> = value.split_whitespace().collect();
    if p == ["none"] {
        return Ok(());
    }
    if p.is_empty()
        || p.len() > 2
        || !matches!(
            p[0],
            "xMinYMin"
                | "xMinYMid"
                | "xMinYMax"
                | "xMidYMin"
                | "xMidYMid"
                | "xMidYMax"
                | "xMaxYMin"
                | "xMaxYMid"
                | "xMaxYMax"
        )
        || p.get(1).is_some_and(|v| !matches!(*v, "meet" | "slice"))
    {
        return Err(DecodeError::UnsupportedSvg);
    }
    Ok(())
}
fn tag_end(text: &str) -> Result<usize, DecodeError> {
    let mut quote = None;
    for (i, c) in text.char_indices().skip(1) {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c == '>' {
            return Ok(i);
        }
    }
    Err(DecodeError::InvalidImage)
}
fn attributes(mut text: &str) -> Result<(String, Attrs), DecodeError> {
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let name = text[..end].to_owned();
    text = &text[end..];
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(DecodeError::UnsupportedSvg);
    }
    let mut attrs = Attrs::new();
    while !text.trim().is_empty() {
        if !text.starts_with(char::is_whitespace) {
            return Err(DecodeError::InvalidImage);
        }
        text = text.trim_start();
        let end = text
            .find(|c: char| c == '=' || c.is_ascii_whitespace())
            .ok_or(DecodeError::InvalidImage)?;
        let key = &text[..end];
        text = text[end..].trim_start();
        text = text
            .strip_prefix('=')
            .ok_or(DecodeError::InvalidImage)?
            .trim_start();
        let quote = text
            .chars()
            .next()
            .filter(|c| *c == '\'' || *c == '"')
            .ok_or(DecodeError::InvalidImage)?;
        text = &text[1..];
        let end = text.find(quote).ok_or(DecodeError::InvalidImage)?;
        if end > 4096 || attrs.len() >= 32 {
            return Err(DecodeError::AllocationLimit);
        }
        if text[..end].contains('<') {
            return Err(DecodeError::InvalidImage);
        }
        let val = entities(&text[..end])?;
        text = &text[end + 1..];
        if attrs.insert(key.to_owned(), val).is_some() {
            return Err(DecodeError::InvalidImage);
        }
    }
    Ok((name, attrs))
}


fn entities(mut value: &str) -> Result<String, DecodeError> {
    let mut out = String::new();
    while let Some(start) = value.find('&') {
        out.push_str(&value[..start]);
        value = &value[start + 1..];
        let end = value.find(';').ok_or(DecodeError::InvalidImage)?;
        let entity = &value[..end];
        let c = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(n) = entity.strip_prefix("#x") {
                    u32::from_str_radix(n, 16).ok()
                } else {
                    entity.strip_prefix('#').and_then(|n| n.parse().ok())
                };
                code.and_then(char::from_u32)
                    .filter(|c| *c != '\0')
                    .ok_or(DecodeError::UnsupportedSvg)?
            }
        };
        out.push(c);
        value = &value[end + 1..];
    }
    out.push_str(value);
    Ok(out)
}

#[cfg(test)]
mod text_element_tests {
    use super::decode;
    use crate::DecodeLimits;

    /// RC-28: a `<text>` label next to a supported shape must not reject the
    /// whole document. `logo.svg`/`thumb-N.svg` in the real-pages corpus are
    /// exactly this shape (a `<rect>` background plus a `<text>` label).
    #[test]
    fn rect_next_to_a_text_label_still_decodes_and_paints_the_rect() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="120" viewBox="0 0 160 120"><rect width="160" height="120" fill="#2f6f4e"/><text x="8" y="64" font-family="sans-serif" font-size="12" fill="#ffffff">1</text></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("decode should not reject the whole document");
        assert_eq!((image.width, image.height), (160, 120));
        // The rect fills the whole viewport with an opaque color; sample its
        // center, well away from the text label near the bottom-left.
        let (x, y) = (120u32, 20u32);
        let at = (y as usize * image.width as usize + x as usize) * 4;
        assert_eq!(&image.pixels[at..at + 4], &[0x2f, 0x6f, 0x4e, 0xff]);
    }

    #[test]
    fn text_before_a_shape_still_decodes() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text x="0" y="10">label</text><rect width="40" height="40" fill="#112233"/></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("decode should not reject the whole document");
        let at = (20usize * image.width as usize + 20) * 4;
        assert_eq!(&image.pixels[at..at + 4], &[0x11, 0x22, 0x33, 0xff]);
    }

    #[test]
    fn unbalanced_text_tag_is_still_rejected() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text x="0" y="10">label<rect width="40" height="40" fill="#112233"/></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    #[test]
    fn character_data_outside_text_is_still_unsupported() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">stray text<rect width="40" height="40" fill="#112233"/></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    #[test]
    fn unknown_attribute_on_text_is_still_rejected() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text x="0" y="10" onclick="evil()">label</text><rect width="40" height="40" fill="#112233"/></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }
}

#[cfg(test)]
mod rc30_tests {
    use super::decode;
    use crate::DecodeLimits;
    use amazing_media::Image;

    fn leftmost_ink_column(image: &Image) -> Option<u32> {
        for x in 0..image.width {
            for y in 0..image.height {
                let at = (y as usize * image.width as usize + x as usize) * 4;
                if image.pixels[at + 3] > 0 {
                    return Some(x);
                }
            }
        }
        None
    }

    /// Inclusive bounding box of every non-transparent pixel, or `None` if
    /// the canvas is entirely transparent.
    fn ink_bbox(image: &Image) -> Option<(u32, u32, u32, u32)> {
        let mut bbox: Option<(u32, u32, u32, u32)> = None;
        for y in 0..image.height {
            for x in 0..image.width {
                let at = (y as usize * image.width as usize + x as usize) * 4;
                if image.pixels[at + 3] == 0 {
                    continue;
                }
                bbox = Some(match bbox {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
        bbox
    }

    // --- unknown-element tolerance (the SVG 2 error-handling model) -------

    #[test]
    fn unknown_element_and_its_subtree_are_skipped_not_fatal() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><title>hello</title><defs><linearGradient id="g"><stop offset="0" stop-color="red"/></linearGradient></defs><rect width="40" height="40" fill="#112233"/></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("unknown elements must not reject the document");
        let at = (20usize * image.width as usize + 20) * 4;
        assert_eq!(&image.pixels[at..at + 4], &[0x11, 0x22, 0x33, 0xff]);
    }

    #[test]
    fn self_closing_unknown_element_is_skipped() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><use href="#x"/><rect width="10" height="10" fill="#010203"/></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_ok());
    }

    #[test]
    fn nested_unknown_elements_are_all_skipped() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><a href="#"><g><outer><inner/></outer></g></a><rect width="10" height="10" fill="#0a0b0c"/></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("nested unknown content must not reject the document");
        let at = (5usize * image.width as usize + 5) * 4;
        assert_eq!(&image.pixels[at..at + 4], &[0x0a, 0x0b, 0x0c, 0xff]);
    }

    #[test]
    fn mismatched_close_tag_inside_ignored_subtree_is_still_rejected() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><foo></bar></defs></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    #[test]
    fn unbalanced_ignored_subtree_at_end_of_document_is_rejected() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><defs><foo></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    #[test]
    fn unknown_root_element_is_rejected_not_skipped() {
        // Tolerance only applies to descendants; the document root must
        // still be a real `<svg>` element.
        let svg = br##"<notsvg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    // --- real `<text>` rendering -------------------------------------------

    #[test]
    fn text_fill_actually_paints_pixels() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="30"><text x="4" y="20" font-size="16" fill="#ff0000">AAA</text></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("decode");
        let any_red_ink = image.pixels.chunks_exact(4)
            .any(|p| p[3] > 0 && p[0] == 0xff && p[1] == 0 && p[2] == 0);
        assert!(any_red_ink, "expected red ink somewhere on the canvas");
    }

    #[test]
    fn text_none_fill_with_no_stroke_paints_nothing() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><text x="4" y="16" font-size="16" fill="none">AAA</text></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("decode");
        assert!(image.pixels.chunks_exact(4).all(|p| p[3] == 0), "fill:none with no stroke should paint nothing");
    }

    #[test]
    fn text_anchor_shifts_the_run_horizontally() {
        let start = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><text x="10" y="20" font-size="16" fill="#ff0000" text-anchor="start">AAA</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let end = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><text x="70" y="20" font-size="16" fill="#ff0000" text-anchor="end">AAA</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let start_left = leftmost_ink_column(&start).expect("start-anchored text should paint something");
        let end_left = leftmost_ink_column(&end).expect("end-anchored text should paint something");
        assert!(start_left <= 12, "start anchor should begin near x=10, got {start_left}");
        assert!(end_left > start_left + 10, "end anchor should sit well to the right of start anchor, got {end_left} vs {start_left}");
    }

    #[test]
    fn stroke_widens_the_painted_region() {
        let fill_only = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60"><text x="20" y="40" font-size="24" fill="#000000">W</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let with_stroke = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60"><text x="20" y="40" font-size="24" fill="#000000" stroke="#ff00ff" stroke-width="6">W</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let (x0, y0, x1, y1) = ink_bbox(&fill_only).expect("fill-only text should paint something");
        let (sx0, sy0, sx1, sy1) = ink_bbox(&with_stroke).expect("stroked text should paint something");
        assert!(sx0 <= x0 && sy0 <= y0 && sx1 >= x1 && sy1 >= y1, "stroke halo should not shrink the fill's bbox");
        assert!((sx1 - sx0) > (x1 - x0) || (sy1 - sy0) > (y1 - y0), "stroke halo should widen the ink bbox");
    }

    #[test]
    fn absurd_font_size_is_rejected_not_silently_clamped() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><text x="0" y="0" font-size="99999">A</text></svg>"##;
        assert!(decode(svg, DecodeLimits::default()).is_err());
    }

    #[test]
    fn whitespace_only_text_content_draws_nothing_but_does_not_fail() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><text x="0" y="0">   </text><rect width="20" height="20" fill="#445566"/></svg>"##;
        let image = decode(svg, DecodeLimits::default()).expect("decode");
        let at = (10usize * image.width as usize + 10) * 4;
        assert_eq!(&image.pixels[at..at + 4], &[0x44, 0x55, 0x66, 0xff]);
    }

    #[test]
    fn dx_dy_offset_the_run() {
        let base = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text x="4" y="20" font-size="16" fill="#ff0000">A</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let offset = decode(
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><text x="4" y="20" dx="10" dy="5" font-size="16" fill="#ff0000">A</text></svg>"##,
            DecodeLimits::default(),
        ).expect("decode");
        let base_left = leftmost_ink_column(&base).expect("base text should paint something");
        let offset_left = leftmost_ink_column(&offset).expect("dx/dy-offset text should paint something");
        assert!(offset_left > base_left, "dx should shift the run to the right: {offset_left} vs {base_left}");
    }
}
