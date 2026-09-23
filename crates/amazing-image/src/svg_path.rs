//! One closed, straight-line SVG outline. This deliberately excludes subpaths and fill rules.
use crate::DecodeError;

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
    let mut closed = false;
    while pos < tokens.len() {
        if let Token::Command(c) = tokens[pos] {
            if (points.is_empty() && !matches!(c, b'M' | b'm'))
                || (!points.is_empty() && matches!(c, b'M' | b'm')) {
                return Err(DecodeError::UnsupportedSvg);
            }
            command = c;
            pos += 1;
            if matches!(c, b'Z' | b'z') {
                closed = true;
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
    if !closed || pos != tokens.len() || points.len() < 3 {
        return Err(DecodeError::UnsupportedSvg);
    }
    Ok(points)
}

pub(super) fn point_in_path(points: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut previous = *points.last().unwrap();
    for &next in points {
        if (previous.1 > y) != (next.1 > y) {
            let crossing = previous.0 + (y - previous.1) * (next.0 - previous.0)
                / (next.1 - previous.1);
            if x < crossing { inside = !inside; }
        }
        previous = next;
    }
    inside
}
