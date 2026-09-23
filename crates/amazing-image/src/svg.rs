//! Explicit static SVG image subset, not a general SVG/DOM implementation.
//! Strict XML subset; rectangles, circles and ellipses with presentation fills.
//! Bounded inline color/fill declarations use the shared CSS declaration parser.
//! No scripts, stylesheets, references, paths, text, strokes or animation.
//! Bounded attribute transforms on groups and shapes are supported.
use crate::{DecodeError, DecodeLimits};
use amazing_media::Image;
use std::collections::BTreeMap;

#[path = "svg_transform.rs"]
mod transform;
use transform::Matrix;

type Attrs = BTreeMap<String, String>;
const MAX_SHAPES: usize = 1024;
const MAX_WORK: usize = 64 * 1024 * 1024;
#[derive(Clone, Copy)]
struct Style {
    fill: [u8; 4],
    fill_current: bool,
    color: [u8; 4],
    fill_opacity: f64,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            fill: [0, 0, 0, 255],
            fill_current: false,
            color: [0, 0, 0, 255],
            fill_opacity: 1.0,
        }
    }
}
#[derive(Clone, Copy)]
enum Shape {
    Rect(f64, f64, f64, f64),
    Ellipse(f64, f64, f64, f64),
}
struct Item {
    inverse: Option<Matrix>,
    shape: Shape,
    color: [u8; 4],
}
struct Scene {
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
    if scene.width == 0
        || scene.height == 0
        || scene.width > limits.max_dimension
        || scene.height > limits.max_dimension
    {
        return Err(DecodeError::GeometryLimit);
    }
    let count = (scene.width as usize)
        .checked_mul(scene.height as usize)
        .ok_or(DecodeError::AllocationLimit)?;
    let bytes = count.checked_mul(4).ok_or(DecodeError::AllocationLimit)?;
    if bytes as u64 > limits.max_rgba_bytes || bytes as u64 > limits.max_decoder_bytes {
        return Err(DecodeError::AllocationLimit);
    }
    if count
        .checked_mul(scene.items.len())
        .is_none_or(|n| n > MAX_WORK)
    {
        return Err(DecodeError::AllocationLimit);
    }
    let mut image = Image {
        width: scene.width,
        height: scene.height,
        pixels: vec![0; bytes],
    };
    let [vx, vy, vw, vh] = scene.view;
    let (mut sx, mut sy) = (scene.width as f64 / vw, scene.height as f64 / vh);
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
        let leftover_x = scene.width as f64 - vw * sx;
        let leftover_y = scene.height as f64 - vh * sy;
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
    for item in scene.items {
        let Some(inverse) = item.inverse else {
            continue;
        };
        for y in 0..scene.height {
            for x in 0..scene.width {
                let px = (x as f64 + 0.5 - ox) / sx + vx;
                let py = (y as f64 + 0.5 - oy) / sy + vy;
                let (px, py) = inverse.point(px, py);
                let inside = match item.shape {
                    Shape::Rect(rx, ry, w, h) => px >= rx && px < rx + w && py >= ry && py < ry + h,
                    Shape::Ellipse(cx, cy, rx, ry) => {
                        rx > 0.0
                            && ry > 0.0
                            && ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0
                    }
                };
                if inside {
                    blend(
                        &mut image.pixels[(y as usize * scene.width as usize + x as usize) * 4..]
                            [..4],
                        item.color,
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
fn parse(mut text: &str) -> Result<Scene, DecodeError> {
    let mut scene = None;
    let mut stack: Vec<(String, Style, Matrix)> = Vec::new();
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
            return Err(DecodeError::UnsupportedSvg);
        }
        let end = tag_end(text)?;
        let tag = &text[1..end];
        text = &text[end + 1..];
        if let Some(close) = tag.strip_prefix('/') {
            if stack.pop().is_none_or(|(name, _, _)| name != close.trim()) {
                return Err(DecodeError::InvalidImage);
            }
            continue;
        }
        let self_closed = tag.ends_with('/');
        let tag = tag.strip_suffix('/').unwrap_or(tag).trim_end();
        let (name, attrs) = attributes(tag)?;
        let attrs = presentation_attrs(attrs)?;
        elements += 1;
        if elements > MAX_SHAPES || stack.len() >= 32 {
            return Err(DecodeError::AllocationLimit);
        }
        let inherited = stack.last().map(|(_, s, _)| *s).unwrap_or_default();
        let style = style(&attrs, inherited)?;
        let parent_matrix = stack.last().map(|(_, _, m)| *m).unwrap_or(Matrix::IDENTITY);
        let local_matrix = attrs
            .get("transform")
            .map_or(Ok(Matrix::IDENTITY), |v| transform::parse(v))?;
        let matrix = parent_matrix.then(local_matrix)?;
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
                ],
            )?;
            let width = required(&attrs, "width")?;
            let height = required(&attrs, "height")?;
            if width <= 0.0 || height <= 0.0 || width > u32::MAX as f64 || height > u32::MAX as f64
            {
                return Err(DecodeError::GeometryLimit);
            }
            // Fractional intrinsic viewport support is deferred rather than rounded silently.
            if width.fract() != 0.0 || height.fract() != 0.0 {
                return Err(DecodeError::UnsupportedSvg);
            }
            let view = if let Some(v) = attrs.get("viewBox") {
                let numbers: Vec<_> = v
                    .split(|c: char| c.is_ascii_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(scalar)
                    .collect::<Result<_, _>>()?;
                if numbers.len() != 4 || numbers[2] <= 0.0 || numbers[3] <= 0.0 {
                    return Err(DecodeError::InvalidImage);
                }
                [numbers[0], numbers[1], numbers[2], numbers[3]]
            } else {
                [0.0, 0.0, width, height]
            };
            let aspect = attrs
                .get("preserveAspectRatio")
                .map(String::as_str)
                .unwrap_or("xMidYMid meet");
            validate_aspect(aspect)?;
            if scalar_value(&attrs, "opacity", 1.0)? != 1.0 {
                return Err(DecodeError::UnsupportedSvg);
            }
            scene = Some(Scene {
                width: width as u32,
                height: height as u32,
                view,
                aspect: aspect.to_owned(),
                items: Vec::new(),
            });
        } else {
            if !stack
                .last()
                .is_some_and(|(name, _, _)| matches!(name.as_str(), "svg" | "g"))
            {
                return Err(DecodeError::InvalidImage);
            }
            let shape = match name.as_str() {
                "g" => {
                    check_attrs(&attrs, &[])?;
                    if scalar_value(&attrs, "opacity", 1.0)? != 1.0 {
                        return Err(DecodeError::UnsupportedSvg);
                    }
                    None
                }
                "rect" => {
                    check_attrs(&attrs, &["x", "y", "width", "height"])?;
                    let w = value(&attrs, "width", 0.0)?;
                    let h = value(&attrs, "height", 0.0)?;
                    nonnegative(w, h)?;
                    Some(Shape::Rect(
                        value(&attrs, "x", 0.0)?,
                        value(&attrs, "y", 0.0)?,
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
                _ => return Err(DecodeError::UnsupportedSvg),
            };
            if let Some(shape) = shape {
                let mut color = if style.fill_current {
                    style.color
                } else {
                    style.fill
                };
                let opacity = scalar_value(&attrs, "opacity", 1.0)?.clamp(0.0, 1.0);
                color[3] = (color[3] as f64 * style.fill_opacity * opacity).round() as u8;
                scene.as_mut().unwrap().items.push(Item {
                    shape,
                    color,
                    inverse: matrix.inverse()?,
                });
            }
        }
        if !self_closed {
            stack.push((name, style, matrix));
        }
    }
    if !stack.is_empty() {
        return Err(DecodeError::InvalidImage);
    }
    scene.ok_or(DecodeError::InvalidImage)
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
fn scalar_value(a: &Attrs, key: &str, default: f64) -> Result<f64, DecodeError> {
    a.get(key).map_or(Ok(default), |s| scalar(s))
}
fn required(a: &Attrs, key: &str) -> Result<f64, DecodeError> {
    number(a.get(key).ok_or(DecodeError::UnsupportedSvg)?)
}
fn value(a: &Attrs, key: &str, default: f64) -> Result<f64, DecodeError> {
    a.get(key).map_or(Ok(default), |s| number(s))
}
fn style(a: &Attrs, mut parent: Style) -> Result<Style, DecodeError> {
    if let Some(color) = a.get("color") {
        parent.color =
            amazing_css::color::parse_css_color(color).ok_or(DecodeError::UnsupportedSvg)?;
    }
    if let Some(fill) = a.get("fill") {
        parent.fill_current = fill.eq_ignore_ascii_case("currentcolor");
        parent.fill = if fill.eq_ignore_ascii_case("none") {
            [0; 4]
        } else if parent.fill_current {
            parent.color
        } else {
            amazing_css::color::parse_css_color(fill).ok_or(DecodeError::UnsupportedSvg)?
        };
    }
    parent.fill_opacity = scalar_value(a, "fill-opacity", parent.fill_opacity)?.clamp(0.0, 1.0);
    Ok(parent)
}

/// Presentation attributes are lower priority than declarations in `style`.
/// The CSS crate owns declaration tokenization, comments, escapes and !important.
/// Only values this static renderer can compute and paint are admitted here.
fn presentation_attrs(mut attrs: Attrs) -> Result<Attrs, DecodeError> {
    let Some(inline) = attrs.remove("style") else {
        return Ok(attrs);
    };
    let declarations = amazing_css::parse_declarations(&inline);
    if declarations.is_empty() && !inline.trim().is_empty() {
        return Err(DecodeError::UnsupportedSvg);
    }
    let mut winners: BTreeMap<String, (String, bool)> = BTreeMap::new();
    for declaration in declarations {
        if !matches!(
            declaration.property.as_str(),
            "fill" | "color" | "fill-opacity" | "opacity"
        ) {
            return Err(DecodeError::UnsupportedSvg);
        }
        let previous_important = winners
            .get(&declaration.property)
            .is_some_and(|(_, important)| *important);
        if !previous_important || declaration.important {
            winners.insert(
                declaration.property,
                (declaration.value, declaration.important),
            );
        }
    }
    for (property, (value, _)) in winners {
        attrs.insert(property, value);
    }
    Ok(attrs)
}
fn check_attrs(a: &Attrs, extra: &[&str]) -> Result<(), DecodeError> {
    for key in a.keys() {
        if ![
            "id",
            "fill",
            "color",
            "fill-opacity",
            "opacity",
            "transform",
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
