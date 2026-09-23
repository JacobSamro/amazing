//! Bounded SVG polygon points-list: finite coordinate pairs separated by comma or whitespace.
use crate::DecodeError;

pub(super) fn parse_points(data: &str, vertex_limit: usize) -> Result<Vec<(f64, f64)>, DecodeError> {
    let bytes = data.as_bytes();
    let mut i = 0;
    let mut values = Vec::new();
    loop {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }
        let start = i;
        if matches!(bytes[i], b'+' | b'-') {
            i += 1;
        }
        let mut digits = 0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(DecodeError::UnsupportedSvg);
        }
        if i < bytes.len() && matches!(bytes[i], b'e' | b'E') {
            i += 1;
            if i < bytes.len() && matches!(bytes[i], b'+' | b'-') {
                i += 1;
            }
            let exponent = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == exponent {
                return Err(DecodeError::UnsupportedSvg);
            }
        }
        let value: f64 = data[start..i]
            .parse()
            .map_err(|_| DecodeError::UnsupportedSvg)?;
        if !value.is_finite() {
            return Err(DecodeError::UnsupportedSvg);
        }
        if values.len() == vertex_limit.saturating_mul(2) {
            return Err(DecodeError::AllocationLimit);
        }
        values.push(value);

        let separator = i;
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
                i += 1;
            }
            if i == bytes.len() {
                return Err(DecodeError::UnsupportedSvg);
            }
        } else if i < bytes.len()
            && i == separator
            && !(values.len() % 2 == 1 && bytes[i] == b'-')
        {
            return Err(DecodeError::UnsupportedSvg);
        }
    }
    if values.len() < 6 || values.len() % 2 != 0 {
        return Err(DecodeError::UnsupportedSvg);
    }
    Ok(values.chunks_exact(2).map(|v| (v[0], v[1])).collect())
}
