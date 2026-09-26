//! Rasterization of CSS generated `<gradient>` images. CSS owns tokenization,
//! color parsing and preservation of the generated image value; this module
//! resolves the image function against its concrete replaced-fragment size.

use amazing_css::syntax::{serialize, tokenize, Token};
use amazing_media::Image;

#[derive(Clone, Copy)]
enum Kind {
    Linear(f32, f32),
    /// Center and per-axis radius in raster pixels. A circle has equal radii;
    /// an ellipse may differ. The default is the farthest-corner radius.
    Radial {
        center: (f32, f32),
        radius: (f32, f32),
    },
    /// Start angle in degrees and center in raster pixels.
    Conic {
        start: f32,
        center: (f32, f32),
    },
}

#[derive(Clone, Copy)]
struct Stop {
    color: [u8; 4],
    position: f32,
}

/// A gradient has no intrinsic dimensions. The generated fragment supplies
/// the concrete raster size, with a bounded allocation like other paint layers.
pub(crate) fn raster(css: &str, width: u32, height: u32, font_size: f32) -> Option<Image> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels == 0 || pixels > 4 * 1024 * 1024 {
        return None;
    }
    let tokens = tokenize(css);
    let Some(Token::Function(name)) = tokens.first() else {
        return None;
    };
    if tokens.get(1) != Some(&Token::OpenParen) || tokens.last() != Some(&Token::CloseParen) {
        return None;
    }
    let (mut kind, repeating) = match name.to_ascii_lowercase().as_str() {
        "linear-gradient" => (Kind::Linear(0.0, 1.0), false),
        "repeating-linear-gradient" => (Kind::Linear(0.0, 1.0), true),
        "radial-gradient" => (default_radial(width, height), false),
        "repeating-radial-gradient" => (default_radial(width, height), true),
        "conic-gradient" => (
            Kind::Conic {
                start: 0.0,
                center: (width as f32 * 0.5, height as f32 * 0.5),
            },
            false,
        ),
        "repeating-conic-gradient" => (
            Kind::Conic {
                start: 0.0,
                center: (width as f32 * 0.5, height as f32 * 0.5),
            },
            true,
        ),
        _ => return None,
    };
    let parts = comma_parts(&tokens[2..tokens.len() - 1])?;
    // An optional leading geometry part precedes the color stops. If the first
    // part begins with a color it is already a stop; otherwise it must parse as
    // valid geometry or the whole gradient is rejected (never guessed).
    let first = if color_prefix(parts[0]).is_some() {
        0
    } else {
        kind = geometry(parts[0], kind, width, height, font_size)?;
        1
    };
    // CSS Images 3 allows a gradient with just one color stop (and no
    // explicit geometry-only case): `linear-gradient(green)` is valid and
    // renders as a solid fill of that color (`sample` below already
    // degenerates correctly to a single color when `stops.len() == 1`).
    if parts.len() - first < 1 {
        return None;
    }
    let conic = matches!(kind, Kind::Conic { .. });
    let extent = match kind {
        Kind::Linear(dx, dy) => (dx.abs() * width as f32 + dy.abs() * height as f32).max(1.0),
        Kind::Radial { radius, .. } => radius.0.max(radius.1).max(1.0),
        Kind::Conic { .. } => 360.0,
    };
    let mut stops = Vec::new();
    for part in &parts[first..] {
        let (color, positions) = color_stop(part, extent, font_size, conic)?;
        if positions.is_empty() {
            stops.push((color, None));
        } else {
            for position in positions {
                stops.push((color, Some(position)));
            }
        }
    }
    if stops.is_empty() {
        return None;
    }
    resolve_stops(&mut stops);
    let stops: Vec<Stop> = stops
        .into_iter()
        .map(|(color, position)| Stop {
            color,
            position: position.unwrap_or(0.0),
        })
        .collect();
    let mut output = Vec::with_capacity(pixels * 4);
    for y in 0..height {
        for x in 0..width {
            let cx = x as f32 + 0.5;
            let cy = y as f32 + 0.5;
            let t = match kind {
                Kind::Linear(dx, dy) => {
                    let px = cx - width as f32 * 0.5;
                    let py = cy - height as f32 * 0.5;
                    0.5 + (px * dx + py * dy) / extent
                }
                Kind::Radial { center, radius } => {
                    if radius.0 <= 0.0 || radius.1 <= 0.0 {
                        return None;
                    }
                    let dx = (cx - center.0) / radius.0;
                    let dy = (cy - center.1) / radius.1;
                    dx.hypot(dy)
                }
                Kind::Conic { start, center } => {
                    let angle = (cx - center.0).atan2(-(cy - center.1)).to_degrees();
                    (angle - start).rem_euclid(360.0) / 360.0
                }
            };
            output.extend_from_slice(&sample(&stops, t, repeating));
        }
    }
    Some(Image {
        width,
        height,
        pixels: output,
    })
}

fn comma_parts(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            Token::Comma if depth == 0 => {
                parts.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    parts.push(&tokens[start..]);
    Some(parts)
}

fn default_radial(width: u32, height: u32) -> Kind {
    let center = (width as f32 * 0.5, height as f32 * 0.5);
    Kind::Radial {
        center,
        radius: farthest_corner(center, width as f32, height as f32),
    }
}

fn farthest_corner(center: (f32, f32), width: f32, height: f32) -> (f32, f32) {
    let dx = center.0.abs().max((width - center.0).abs());
    let dy = center.1.abs().max((height - center.1).abs());
    let distance = (dx * dx + dy * dy).sqrt();
    (distance, distance)
}

fn geometry(tokens: &[Token], kind: Kind, width: u32, height: u32, font_size: f32) -> Option<Kind> {
    let text = serialize(tokens).trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    match kind {
        Kind::Linear(..) => {
            let vector = match text.as_str() {
                "to top" => (0.0, -1.0),
                "to bottom" => (0.0, 1.0),
                "to left" => (-1.0, 0.0),
                "to right" => (1.0, 0.0),
                "to top left" | "to left top" => (-0.70710677, -0.70710677),
                "to top right" | "to right top" => (0.70710677, -0.70710677),
                "to bottom left" | "to left bottom" => (-0.70710677, 0.70710677),
                "to bottom right" | "to right bottom" => (0.70710677, 0.70710677),
                _ => {
                    let angle = angle_degrees(&text)?;
                    (angle.to_radians().sin(), -angle.to_radians().cos())
                }
            };
            Some(Kind::Linear(vector.0, vector.1))
        }
        Kind::Radial { .. } => parse_radial(&text, width, height, font_size),
        Kind::Conic { .. } => parse_conic(&text, width, height),
    }
}

/// Splits a geometry part into its shape/size portion and an optional `at
/// <position>` tail. `at` is a keyword; a color stop never contains a bare
/// `at` token.
fn split_at(text: &str) -> (&str, Option<&str>) {
    if let Some(rest) = text.strip_prefix("at ") {
        ("", Some(rest.trim()))
    } else if let Some(index) = text.rfind(" at ") {
        (text[..index].trim(), Some(text[index + 4..].trim()))
    } else {
        (text, None)
    }
}

fn parse_radial(text: &str, width: u32, height: u32, font_size: f32) -> Option<Kind> {
    let (shape_size, position) = split_at(text);
    let center = resolve_position(position, width, height)?;
    let (w, h) = (width as f32, height as f32);
    let mut circle = false;
    let mut explicit: Vec<f32> = Vec::new();
    let mut keyword: Option<&str> = None;
    for token in shape_size.split_whitespace() {
        match token {
            "circle" => circle = true,
            "ellipse" => circle = false,
            "closest-side" | "farthest-side" | "closest-corner" | "farthest-corner" => {
                if keyword.replace(token).is_some() {
                    return None;
                }
            }
            other => explicit.push(length_px(other, font_size)?),
        }
    }
    if keyword.is_some() && !explicit.is_empty() {
        return None;
    }
    if explicit.len() > 2 {
        return None;
    }
    let radius = if !explicit.is_empty() {
        if circle || explicit.len() == 1 {
            (explicit[0], explicit[0])
        } else {
            (explicit[0], explicit[1])
        }
    } else {
        let (dx_min, dy_min) = (center.0.min(w - center.0), center.1.min(h - center.1));
        let (dx_max, dy_max) = (center.0.max(w - center.0), center.1.max(h - center.1));
        match keyword.unwrap_or("farthest-corner") {
            "closest-side" => (dx_min, dy_min),
            "farthest-side" => (dx_max, dy_max),
            "closest-corner" => {
                let distance = (dx_min * dx_min + dy_min * dy_min).sqrt();
                (distance, distance)
            }
            _ => {
                let distance = (dx_max * dx_max + dy_max * dy_max).sqrt();
                (distance, distance)
            }
        }
    };
    if !radius.0.is_finite() || !radius.1.is_finite() || radius.0 <= 0.0 || radius.1 <= 0.0 {
        return None;
    }
    Some(Kind::Radial { center, radius })
}

fn parse_conic(text: &str, width: u32, height: u32) -> Option<Kind> {
    let (from_part, position) = split_at(text);
    let center = resolve_position(position, width, height)?;
    let from_part = from_part.trim();
    let start = if from_part.is_empty() {
        0.0
    } else {
        angle_degrees(from_part.strip_prefix("from ")?.trim())?
    };
    start.is_finite().then_some(Kind::Conic { start, center })
}

/// Resolves an `at <position>` tail in raster pixels. `None` means the default
/// box center; an empty tail is invalid.
fn resolve_position(position: Option<&str>, width: u32, height: u32) -> Option<(f32, f32)> {
    let Some(position) = position else {
        return Some((width as f32 * 0.5, height as f32 * 0.5));
    };
    if position.is_empty() {
        return None;
    }
    let (horizontal, vertical) = crate::image::position(position)?;
    let x = horizontal.fraction * width as f32 + horizontal.offset;
    let y = vertical.fraction * height as f32 + vertical.offset;
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

fn length_px(value: &str, font_size: f32) -> Option<f32> {
    let radius = if let Some(n) = value.strip_suffix("px") {
        n.trim().parse::<f32>().ok()?
    } else if let Some(n) = value.strip_suffix("rem") {
        n.trim().parse::<f32>().ok()? * 16.0
    } else if let Some(n) = value.strip_suffix("em") {
        n.trim().parse::<f32>().ok()? * font_size
    } else {
        return None;
    };
    (radius.is_finite() && radius > 0.0).then_some(radius)
}

fn angle_degrees(value: &str) -> Option<f32> {
    let value = value.trim();
    let (number, factor) = if let Some(n) = value.strip_suffix("deg") {
        (n, 1.0)
    } else if let Some(n) = value.strip_suffix("grad") {
        (n, 0.9)
    } else if let Some(n) = value.strip_suffix("rad") {
        (n, 180.0 / std::f32::consts::PI)
    } else if let Some(n) = value.strip_suffix("turn") {
        (n, 360.0)
    } else {
        return None;
    };
    let number: f32 = number.trim().parse().ok()?;
    number.is_finite().then_some(number * factor)
}

fn color_prefix(tokens: &[Token]) -> Option<usize> {
    let tokens = trim(tokens);
    (1..=tokens.len()).find(|end| {
        amazing_css::color::parse_css_color(serialize(&tokens[..*end]).trim()).is_some()
    })
}

fn color_stop(
    tokens: &[Token],
    extent: f32,
    font_size: f32,
    conic: bool,
) -> Option<([u8; 4], Vec<f32>)> {
    let tokens = trim(tokens);
    let end = color_prefix(tokens)?;
    let color = amazing_css::color::parse_css_color(serialize(&tokens[..end]).trim())?;
    let rest = trim(&tokens[end..]);
    if rest.is_empty() {
        return Some((color, Vec::new()));
    }
    let mut positions = Vec::new();
    for token in rest.iter().filter(|token| **token != Token::Whitespace) {
        let position = match token {
            Token::Percentage(value) => (*value as f32) / 100.0,
            Token::Dimension(value, unit) if conic && unit.eq_ignore_ascii_case("deg") => {
                (*value as f32) / 360.0
            }
            Token::Dimension(value, unit) if conic && unit.eq_ignore_ascii_case("turn") => {
                *value as f32
            }
            Token::Dimension(value, unit) if !conic && unit.eq_ignore_ascii_case("px") => {
                (*value as f32) / extent
            }
            Token::Dimension(value, unit) if !conic && unit.eq_ignore_ascii_case("em") => {
                (*value as f32) * font_size / extent
            }
            Token::Number(value) if *value == 0.0 => 0.0,
            _ => return None,
        };
        if !position.is_finite() || positions.len() == 2 {
            return None;
        }
        positions.push(position);
    }
    Some((color, positions))
}

fn trim(mut tokens: &[Token]) -> &[Token] {
    while tokens.first() == Some(&Token::Whitespace) {
        tokens = &tokens[1..];
    }
    while tokens.last() == Some(&Token::Whitespace) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

fn resolve_stops(stops: &mut [([u8; 4], Option<f32>)]) {
    let last = stops.len() - 1;
    stops[0].1.get_or_insert(0.0);
    stops[last].1.get_or_insert(1.0);
    let mut anchor = 0;
    for index in 1..stops.len() {
        let Some(mut end) = stops[index].1 else {
            continue;
        };
        end = end.max(stops[anchor].1.unwrap_or(0.0));
        stops[index].1 = Some(end);
        let start = stops[anchor].1.unwrap_or(0.0);
        for middle in anchor + 1..index {
            stops[middle].1 =
                Some(start + (end - start) * (middle - anchor) as f32 / (index - anchor) as f32);
        }
        anchor = index;
    }
}

fn sample(stops: &[Stop], mut t: f32, repeating: bool) -> [u8; 4] {
    let start = stops[0].position;
    let end = stops[stops.len() - 1].position;
    if repeating && end > start {
        t = start + (t - start).rem_euclid(end - start);
    }
    if t <= start {
        return stops[0].color;
    }
    if t >= end {
        return stops[stops.len() - 1].color;
    }
    for pair in stops.windows(2) {
        if t <= pair[1].position {
            let span = pair[1].position - pair[0].position;
            let weight = if span > 0.0 {
                (t - pair[0].position) / span
            } else {
                1.0
            };
            let alpha = pair[0].color[3] as f32 * (1.0 - weight) + pair[1].color[3] as f32 * weight;
            if alpha <= 0.0 {
                return [0, 0, 0, 0];
            }
            let mut result = [0; 4];
            for (channel, component) in result.iter_mut().take(3).enumerate() {
                let mixed =
                    pair[0].color[channel] as f32 * pair[0].color[3] as f32 * (1.0 - weight)
                        + pair[1].color[channel] as f32 * pair[1].color[3] as f32 * weight;
                *component = (mixed / alpha).round().clamp(0.0, 255.0) as u8;
            }
            result[3] = alpha.round().clamp(0.0, 255.0) as u8;
            return result;
        }
    }
    stops[stops.len() - 1].color
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_gradient_kinds_and_repeats_rasterize() {
        let cases = [
            "linear-gradient(red, blue)",
            "repeating-linear-gradient(red 0px, blue 4px)",
            "radial-gradient(red, blue)",
            "repeating-radial-gradient(red 0em, blue .4em)",
            "conic-gradient(red, blue)",
            "repeating-conic-gradient(red 0deg, blue 90deg)",
        ];
        for case in cases {
            let image = raster(case, 12, 12, 16.0).expect(case);
            assert_eq!(image.pixels.len(), 12 * 12 * 4);
            assert!(image.pixels.chunks_exact(4).any(|pixel| pixel[3] != 0));
        }
        assert!(raster("url(x.png)", 12, 12, 16.0).is_none());
        assert!(raster("linear-gradient(red, blue)", 5000, 5000, 16.0).is_none());
    }

    #[test]
    fn single_color_stop_is_a_solid_fill() {
        // CSS Images 3: a gradient with exactly one color stop is valid and
        // renders as a solid fill of that color (WPT
        // css-images/gradient/gradient-single-stop-*.html).
        for case in [
            "linear-gradient(green)",
            "linear-gradient(to right, green)",
            "radial-gradient(green)",
            "conic-gradient(green)",
        ] {
            let image = raster(case, 8, 8, 16.0).expect(case);
            for chunk in image.pixels.chunks_exact(4) {
                assert_eq!(chunk, [0, 128, 0, 255], "{case}");
            }
        }
        // A single color stop with a geometry-invalidating rest is still
        // rejected outright (no guessing at a broken gradient).
        assert!(raster("radial-gradient(to right, red)", 8, 8, 16.0).is_none());
        // An empty gradient (no color stops at all) is still rejected.
        assert!(raster("linear-gradient()", 8, 8, 16.0).is_none());
        assert!(raster("linear-gradient(to right)", 8, 8, 16.0).is_none());
    }

    fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * image.width + x) * 4) as usize;
        image.pixels[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn positioned_and_shaped_gradients_use_their_geometry() {
        // Center at the top-left corner: the corner is the first stop and the
        // far corner trails to the last.
        let image = raster("radial-gradient(circle at 0 0, red, blue)", 8, 8, 16.0).unwrap();
        assert!(pixel(&image, 0, 0)[0] > pixel(&image, 0, 0)[2]);
        assert!(pixel(&image, 7, 7)[2] > pixel(&image, 7, 7)[0]);

        // An explicit radius holds the first stop inside and reaches the last
        // stop outside.
        let image =
            raster("radial-gradient(circle 2px at center, red, blue)", 8, 8, 16.0).unwrap();
        assert!(pixel(&image, 3, 3)[0] > pixel(&image, 3, 3)[2]);
        assert!(pixel(&image, 7, 3)[2] > pixel(&image, 7, 3)[0]);

        // `from 90deg` starts the first stop to the east and trails north.
        let image =
            raster("conic-gradient(from 90deg at center, red, blue)", 8, 8, 16.0).unwrap();
        assert!(pixel(&image, 7, 4)[0] > pixel(&image, 7, 4)[2], "east is red");
        assert!(pixel(&image, 4, 0)[2] > pixel(&image, 4, 0)[0], "north trails blue");
    }

    #[test]
    fn invalid_gradient_geometry_is_rejected() {
        for case in [
            "radial-gradient(circle 50% at center, red, blue)",
            "radial-gradient(circle at, red, blue)",
            "radial-gradient(circle at 1 2 3 4 5, red, blue)",
            "radial-gradient(circle closest-side farthest-side, red, blue)",
            "conic-gradient(from nope, red, blue)",
            "conic-gradient(from 45deg at, red, blue)",
            "linear-gradient(to nope, red, blue)",
        ] {
            assert!(raster(case, 8, 8, 16.0).is_none(), "{case}");
        }
    }
}
