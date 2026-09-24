//! Rasterization of CSS generated `<gradient>` images. CSS owns tokenization,
//! color parsing and preservation of the generated image value; this module
//! resolves the image function against its concrete replaced-fragment size.

use amazing_css::syntax::{serialize, tokenize, Token};
use amazing_media::Image;

#[derive(Clone, Copy)]
enum Kind {
    Linear(f32, f32),
    Radial,
    Conic(f32),
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
    let (kind, repeating) = match name.to_ascii_lowercase().as_str() {
        "linear-gradient" => (Kind::Linear(0.0, 1.0), false),
        "repeating-linear-gradient" => (Kind::Linear(0.0, 1.0), true),
        "radial-gradient" => (Kind::Radial, false),
        "repeating-radial-gradient" => (Kind::Radial, true),
        "conic-gradient" => (Kind::Conic(0.0), false),
        "repeating-conic-gradient" => (Kind::Conic(0.0), true),
        _ => return None,
    };
    let parts = comma_parts(&tokens[2..tokens.len() - 1])?;
    if parts.len() < 2 {
        return None;
    }
    let (kind, first) = direction(parts[0], kind).map_or((kind, 0), |direction| (direction, 1));
    if parts.len() - first < 2 {
        return None;
    }
    let extent = match kind {
        Kind::Linear(dx, dy) => (dx.abs() * width as f32 + dy.abs() * height as f32).max(1.0),
        Kind::Radial => ((width as f32).hypot(height as f32) * 0.5).max(1.0),
        Kind::Conic(_) => 360.0,
    };
    let mut stops = Vec::new();
    for part in &parts[first..] {
        let (color, positions) =
            color_stop(part, extent, font_size, matches!(kind, Kind::Conic(_)))?;
        if positions.is_empty() {
            stops.push((color, None));
        } else {
            for position in positions {
                stops.push((color, Some(position)));
            }
        }
    }
    if stops.len() < 2 {
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
            let px = x as f32 + 0.5 - width as f32 * 0.5;
            let py = y as f32 + 0.5 - height as f32 * 0.5;
            let t = match kind {
                Kind::Linear(dx, dy) => 0.5 + (px * dx + py * dy) / extent,
                Kind::Radial => px.hypot(py) / extent,
                Kind::Conic(start) => {
                    ((px.atan2(-py).to_degrees() - start).rem_euclid(360.0)) / 360.0
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

fn direction(tokens: &[Token], kind: Kind) -> Option<Kind> {
    let text = serialize(tokens).trim().to_ascii_lowercase();
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
                    let angle = text.strip_suffix("deg")?.trim().parse::<f32>().ok()?;
                    if !angle.is_finite() {
                        return None;
                    }
                    (angle.to_radians().sin(), -angle.to_radians().cos())
                }
            };
            Some(Kind::Linear(vector.0, vector.1))
        }
        Kind::Radial if text == "circle" || text == "ellipse" => Some(kind),
        Kind::Conic(_) => {
            let angle = text
                .strip_prefix("from ")?
                .strip_suffix("deg")?
                .trim()
                .parse::<f32>()
                .ok()?;
            angle.is_finite().then_some(Kind::Conic(angle))
        }
        _ => None,
    }
}

fn color_stop(
    tokens: &[Token],
    extent: f32,
    font_size: f32,
    conic: bool,
) -> Option<([u8; 4], Vec<f32>)> {
    let tokens = trim(tokens);
    for end in 1..=tokens.len() {
        let color = amazing_css::color::parse_css_color(serialize(&tokens[..end]).trim());
        let Some(color) = color else {
            continue;
        };
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
        return Some((color, positions));
    }
    None
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
}
