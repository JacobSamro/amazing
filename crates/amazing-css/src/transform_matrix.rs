// Portions ported from bitshifter/glam-rs (src/f64/dmat4.rs, from_axis_angle)
// @ e7e588454babd944376baabaae539407992a7408, MIT. Scalar arrays replace glam vectors.
//! Row-major matrices acting on column vectors; CSS function order post-multiplies.
use crate::values::{LengthPercentage, Transform};
pub type TransformMatrix = [[f64; 4]; 4];
const IDENTITY: TransformMatrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
fn mul(a: TransformMatrix, b: TransformMatrix) -> TransformMatrix {
    let mut out = [[0.0; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            out[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}
fn translate(v: [f64; 3]) -> TransformMatrix {
    let mut m = IDENTITY;
    for i in 0..3 {
        m[i][3] = v[i];
    }
    m
}
fn length(v: &LengthPercentage, basis: Option<f64>) -> Option<f64> {
    match v {
        LengthPercentage::Px(n) => Some(*n),
        LengthPercentage::Percentage(n) => Some(n * basis? / 100.0),
        LengthPercentage::Relative(_, _) => None,
    }
}
fn expression(css: &str, basis: Option<f64>) -> Option<f64> {
    if let Some(v) = LengthPercentage::parse(css) {
        return length(&v, basis);
    }
    let tokens = crate::syntax::tokenize(css);
    if tokens.iter().any(|t|matches!(t,crate::syntax::Token::Dimension(n,u) if !matches!(LengthPercentage::parse(&format!("{n}{u}")),Some(LengthPercentage::Px(_))))){return None;}
    if tokens
        .iter()
        .any(|t| matches!(t, crate::syntax::Token::Percentage(_)))
        && basis.is_none()
    {
        return None;
    }
    crate::math::font_size_px(css, basis.unwrap_or(0.0), 0.0).filter(|n| n.is_finite())
}
fn rotation(axis: [f64; 3], angle: f64) -> Option<TransformMatrix> {
    if !angle.is_finite() || axis.iter().any(|n| !n.is_finite()) {
        return None;
    }
    // Normalize without overflowing very large finite CSS axis components.
    let max = axis.iter().copied().map(f64::abs).fold(0.0, f64::max);
    if max == 0.0 {
        return Some(IDENTITY);
    }
    if !max.is_finite() {
        return None;
    }
    let axis = axis.map(|n| n / max);
    let norm = axis.iter().map(|n| n * n).sum::<f64>().sqrt();
    let [x, y, z] = axis.map(|n| n / norm);
    let (sin, cos) = angle.sin_cos();
    let omc = 1.0 - cos;
    let xy = x * y * omc;
    let xz = x * z * omc;
    let yz = y * z * omc;
    // glam's column-major from_axis_angle is transposed into the engine row-major ABI.
    Some([
        [x * x * omc + cos, xy - z * sin, xz + y * sin, 0.0],
        [xy + z * sin, y * y * omc + cos, yz - x * sin, 0.0],
        [xz - y * sin, yz + x * sin, z * z * omc + cos, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}
/// Used-value matrix handoff. Caller supplies actual reference box and origin;
/// unresolved relative units conservatively decline the compositor shortcut.
pub fn transform_matrix(
    operations: &[Transform],
    box_size: Option<[f64; 2]>,
    origin: [f64; 3],
) -> Option<TransformMatrix> {
    if !origin.iter().all(|n| n.is_finite())
        || box_size.is_some_and(|s| s.iter().any(|n| !n.is_finite() || *n < 0.0))
    {
        return None;
    }
    let x = box_size.map(|s| s[0]);
    let y = box_size.map(|s| s[1]);
    let mut matrix = IDENTITY;
    for op in operations {
        let next = match op {
            Transform::Translate(a, b) => translate([length(a, x)?, length(b, y)?, 0.0]),
            Transform::Translate3D(a, b, c) => {
                translate([length(a, x)?, length(b, y)?, length(c, None)?])
            }
            Transform::TranslateExpression(v) => translate([
                expression(&v[0], x)?,
                expression(&v[1], y)?,
                expression(&v[2], None)?,
            ]),
            Transform::Scale(a, b) => {
                let mut m = IDENTITY;
                m[0][0] = *a;
                m[1][1] = *b;
                m
            }
            Transform::Scale3D(v) => {
                let mut m = IDENTITY;
                for i in 0..3 {
                    m[i][i] = v[i];
                }
                m
            }
            Transform::Rotate(a) => rotation([0.0, 0.0, 1.0], a.radians)?,
            Transform::Rotate3D(v, a) => rotation(*v, a.radians)?,
            Transform::SkewX(a) => {
                let mut m = IDENTITY;
                m[0][1] = a.radians.tan();
                m
            }
            Transform::SkewY(a) => {
                let mut m = IDENTITY;
                m[1][0] = a.radians.tan();
                m
            }
            Transform::Skew(a, b) => {
                let mut m = IDENTITY;
                m[0][1] = a.radians.tan();
                m[1][0] = b.radians.tan();
                m
            }
            Transform::Matrix(v) => [
                [v[0], v[2], 0.0, v[4]],
                [v[1], v[3], 0.0, v[5]],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            Transform::Matrix3D(v) => {
                let mut m = [[0.0; 4]; 4];
                for i in 0..4 {
                    for j in 0..4 {
                        m[i][j] = v[j * 4 + i];
                    }
                }
                m
            }
            Transform::Perspective(None) => IDENTITY,
            Transform::Perspective(Some(v)) => {
                let mut m = IDENTITY;
                m[3][2] = -1.0 / expression(v, None)?.max(1.0);
                m
            }
        };
        matrix = mul(matrix, next);
    }
    matrix = mul(
        mul(translate(origin), matrix),
        translate(origin.map(|n| -n)),
    );
    matrix
        .iter()
        .flatten()
        .all(|n| n.is_finite())
        .then_some(matrix)
}
