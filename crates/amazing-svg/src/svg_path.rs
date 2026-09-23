//! One straight-line SVG outline for fill and stroke. Subpaths and curves remain outside this bounded parser.
use crate::DecodeError;
use super::Stroke;

#[derive(Clone, Copy)]
enum Token {
    Command(u8),
    Number(f64),
}

fn tokens(data: &str, limit: usize) -> Result<Vec<Token>, DecodeError> {
    let bytes = data.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    let mut comma = false;
    while i < bytes.len() {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        if i < bytes.len() && bytes[i] == b',' {
            if comma || !matches!(result.last(), Some(Token::Number(_))) {
                return Err(DecodeError::UnsupportedSvg);
            }
            comma = true;
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() { i += 1; }
        }
        if i == bytes.len() { break; }
        let c = bytes[i];
        if c.is_ascii_alphabetic() {
            if comma { return Err(DecodeError::UnsupportedSvg); }
            if !matches!(c, b'M' | b'm' | b'L' | b'l' | b'H' | b'h' | b'V' | b'v' | b'Z' | b'z') {
                return Err(DecodeError::UnsupportedSvg);
            }
            result.push(Token::Command(c));
            i += 1;
        } else {
            if start == i && !comma && matches!(result.last(), Some(Token::Number(_)))
                && !matches!(c, b'+' | b'-' | b'.') {
                return Err(DecodeError::UnsupportedSvg);
            }
            let begin = i;
            if matches!(bytes[i], b'+' | b'-') { i += 1; }
            let mut digits = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; digits += 1; }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; digits += 1; }
            }
            if digits == 0 { return Err(DecodeError::UnsupportedSvg); }
            if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
                i += 1;
                if i < bytes.len() && matches!(bytes[i], b'+' | b'-') { i += 1; }
                let exp_start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                if i == exp_start { return Err(DecodeError::UnsupportedSvg); }
            }
            let number: f64 = data[begin..i].parse().map_err(|_| DecodeError::UnsupportedSvg)?;
            if !number.is_finite() { return Err(DecodeError::UnsupportedSvg); }
            result.push(Token::Number(number));
        }
        comma = false;
        if result.len() > limit.saturating_mul(2).saturating_add(8) {
            return Err(DecodeError::AllocationLimit);
        }
    }
    if comma { return Err(DecodeError::UnsupportedSvg); }
    Ok(result)
}

fn number(tokens: &[Token], pos: &mut usize) -> Result<f64, DecodeError> {
    let Some(Token::Number(n)) = tokens.get(*pos) else { return Err(DecodeError::UnsupportedSvg); };
    *pos += 1;
    Ok(*n)
}

pub(super) fn parse_path(data: &str, limit: usize) -> Result<Vec<(f64, f64)>, DecodeError> {
    let tokens = tokens(data, limit)?;
    let mut pos = 0;
    let mut command = 0;
    let mut points = Vec::new();
    let mut current = (0.0_f64, 0.0_f64);
    while pos < tokens.len() {
        if let Token::Command(c) = tokens[pos] {
            if (points.is_empty() && !matches!(c, b'M' | b'm'))
                || (!points.is_empty() && matches!(c, b'M' | b'm')) {
                return Err(DecodeError::UnsupportedSvg);
            }
            command = c;
            pos += 1;
            if matches!(c, b'Z' | b'z') {
                break;
            }
            if matches!(tokens.get(pos), None | Some(Token::Command(_))) {
                return Err(DecodeError::UnsupportedSvg);
            }
        }
        if command == 0 { return Err(DecodeError::UnsupportedSvg); }
        let relative = command.is_ascii_lowercase();
        let next = match command.to_ascii_uppercase() {
            b'M' | b'L' => {
                let x = number(&tokens, &mut pos)?;
                let y = number(&tokens, &mut pos)?;
                if relative { (current.0 + x, current.1 + y) } else { (x, y) }
            }
            b'H' => {
                let x = number(&tokens, &mut pos)?;
                (if relative { current.0 + x } else { x }, current.1)
            }
            b'V' => {
                let y = number(&tokens, &mut pos)?;
                (current.0, if relative { current.1 + y } else { y })
            }
            _ => return Err(DecodeError::UnsupportedSvg),
        };
        if !next.0.is_finite() || !next.1.is_finite() {
            return Err(DecodeError::UnsupportedSvg);
        }
        current = next;
        points.push(next);
        if points.len() > limit { return Err(DecodeError::AllocationLimit); }
        // Extra coordinate pairs after moveto are lineto, per SVG path grammar.
        if matches!(command, b'M' | b'm') {
            command = if relative { b'l' } else { b'L' };
        }
    }
    // Fill implicitly joins the last point to the first, with or without Z.
    if pos != tokens.len() || points.len() < 2 {
        return Err(DecodeError::UnsupportedSvg);
    }
    Ok(points)
}

pub(super) fn point_in_path(points: &[(f64, f64)], x: f64, y: f64) -> bool {
    // SVG's default fill-rule is nonzero. Count signed ray crossings so a
    // same-direction loop around an area twice still fills it.
    let mut winding = 0_i32;
    let mut previous = *points.last().unwrap();
    for &next in points {
        if (previous.1 > y) != (next.1 > y) {
            let crossing = previous.0 + (y - previous.1) * (next.0 - previous.0)
                / (next.1 - previous.1);
            if x < crossing {
                winding += if next.1 > previous.1 { 1 } else { -1 };
            }
        }
        previous = next;
    }
    winding != 0
}

pub(super) fn point_on_stroke(stroke: &Stroke, x: f64, y: f64) -> bool {
    let points = &stroke.points;
    if points.len() < 2 || stroke.radius <= 0.0 {
        return false;
    }
    let radius_squared = stroke.radius * stroke.radius;
    let segments = points.windows(2).map(|pair| (pair[0], pair[1]));
    let closing = stroke.closed.then(|| (*points.last().unwrap(), points[0]));
    let total_length = if stroke.dash.is_some() {
        segments.clone().chain(closing).map(|((ax, ay), (bx, by))| (bx - ax).hypot(by - ay)).sum()
    } else { 0.0 };
    let mut progress = 0.0;
    segments.chain(closing).any(|((ax, ay), (bx, by))| {
        let (dx, dy) = (bx - ax, by - ay);
        let length_squared = dx * dx + dy * dy;
        let length = length_squared.sqrt();
        let t = if length_squared > 0.0 {
            (((x - ax) * dx + (y - ay) * dy) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (ox, oy) = (x - (ax + t * dx), y - (ay + t * dy));
        let distance = progress + t * length;
        progress += length;
        ox * ox + oy * oy <= radius_squared && stroke.dash.as_ref().is_none_or(|dash| {
            let position = match dash.author_length {
                Some(author) if total_length > 0.0 => distance * author / total_length,
                _ => distance,
            };
            let cycle: f64 = dash.lengths.iter().sum();
            if cycle <= 0.0 { return true; }
            let mut phase = (position + dash.offset).rem_euclid(cycle);
            for (index, length) in dash.lengths.iter().enumerate() {
                if phase < *length { return index % 2 == 0; }
                phase -= length;
            }
            false
        })
    })
}
