//! Bounded SVG attribute transforms. Column vectors; lists post-multiply.
//! https://www.w3.org/TR/SVG11/coords.html#TransformAttribute
use crate::DecodeError;

#[derive(Clone, Copy, Debug)]
pub(super) struct Matrix(pub [f64; 6]);
impl Matrix {
    pub const IDENTITY: Self = Self([1., 0., 0., 1., 0., 0.]);
    pub fn then(self, rhs: Self) -> Result<Self, DecodeError> {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = rhs.0;
        let out = Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ]);
        if out.0.iter().all(|v| v.is_finite()) {
            Ok(out)
        } else {
            Err(DecodeError::GeometryLimit)
        }
    }
    pub fn inverse(self) -> Result<Option<Self>, DecodeError> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if !det.is_finite() {
            return Err(DecodeError::GeometryLimit);
        }
        // A singular CTM paints nothing; it is not an identity transform.
        if det == 0. {
            return Ok(None);
        }
        let out = Self([
            d / det,
            -b / det,
            -c / det,
            a / det,
            (c * f - d * e) / det,
            (b * e - a * f) / det,
        ]);
        if out.0.iter().all(|v| v.is_finite()) {
            Ok(Some(out))
        } else {
            Err(DecodeError::GeometryLimit)
        }
    }
    pub fn point(self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
}

pub(super) fn parse(mut text: &str) -> Result<Matrix, DecodeError> {
    let mut out = Matrix::IDENTITY;
    text = text.trim();
    if text.is_empty() {
        return Ok(out);
    }
    for _ in 0..32 {
        let open = text.find('(').ok_or(DecodeError::UnsupportedSvg)?;
        let name = text[..open].trim_end();
        let close = text[open + 1..]
            .find(')')
            .ok_or(DecodeError::UnsupportedSvg)?
            + open
            + 1;
        let numbers = &text[open + 1..close];
        let mut args = Vec::new();
        // Require SVG comma-wsp separators, rejecting empty or trailing arguments.
        let mut rest = numbers.trim();
        while !rest.is_empty() {
            let end = rest
                .find(|c: char| c == ',' || c.is_ascii_whitespace())
                .unwrap_or(rest.len());
            let token = &rest[..end];
            // Rust accepts inf/NaN; SVG numbers do not. CSS units are also excluded.
            if token.is_empty()
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b))
            {
                return Err(DecodeError::UnsupportedSvg);
            }
            let n: f64 = token.parse().map_err(|_| DecodeError::UnsupportedSvg)?;
            if !n.is_finite() {
                return Err(DecodeError::GeometryLimit);
            }
            args.push(n);
            if args.len() > 6 {
                return Err(DecodeError::UnsupportedSvg);
            }
            rest = rest[end..].trim_start();
            if let Some(next) = rest.strip_prefix(',') {
                rest = next.trim_start();
                if rest.is_empty() {
                    return Err(DecodeError::UnsupportedSvg);
                }
            }
        }
        let m = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Matrix([*a, *b, *c, *d, *e, *f]),
            ("translate", [x]) => Matrix([1., 0., 0., 1., *x, 0.]),
            ("translate", [x, y]) => Matrix([1., 0., 0., 1., *x, *y]),
            ("scale", [x]) => Matrix([*x, 0., 0., *x, 0., 0.]),
            ("scale", [x, y]) => Matrix([*x, 0., 0., *y, 0., 0.]),
            ("rotate", [a]) => rotation(*a),
            ("rotate", [a, x, y]) => Matrix([1., 0., 0., 1., *x, *y])
                .then(rotation(*a))?
                .then(Matrix([1., 0., 0., 1., -*x, -*y]))?,
            ("skewX", [a]) => Matrix([1., 0., a.to_radians().tan(), 1., 0., 0.]),
            ("skewY", [a]) => Matrix([1., a.to_radians().tan(), 0., 1., 0., 0.]),
            _ => return Err(DecodeError::UnsupportedSvg),
        };
        out = out.then(m)?;
        let suffix = &text[close + 1..];
        text = suffix.trim_start();
        if text.is_empty() {
            return Ok(out);
        }
        if let Some(next) = text.strip_prefix(',') {
            text = next.trim_start();
            if text.is_empty() {
                return Err(DecodeError::UnsupportedSvg);
            }
        } else if text.len() == suffix.len() {
            return Err(DecodeError::UnsupportedSvg);
        }
    }
    Err(DecodeError::AllocationLimit)
}
fn rotation(degrees: f64) -> Matrix {
    let (s, c) = (degrees.rem_euclid(360.).to_radians()).sin_cos();
    Matrix([c, s, -s, c, 0., 0.])
}
