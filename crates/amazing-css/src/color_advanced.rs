//! CSS Color 4 conversions used by the CSS computed-value seam.
//! Matrices and transfer functions follow https://drafts.csswg.org/css-color/#color-conversion-code.

use std::f64::consts::PI;
use crate::syntax::{Token, tokenize};

fn finite(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn percentage(value: &str) -> Option<f64> {
    finite(value.strip_suffix('%')?)
}

/// CSS Color 4 §4.4: a `none` component. Direct (non-interpolated) use
/// renders it as 0 in whatever numeric context needs a concrete value;
/// `color-mix()` gives it special treatment instead (see `color_mix`).
fn none_component(value: &str) -> Option<f64> {
    value.eq_ignore_ascii_case("none").then_some(0.0)
}

fn is_none(value: &str) -> bool {
    value.eq_ignore_ascii_case("none")
}

fn hue(value: &str) -> Option<f64> {
    if is_none(value) { return Some(0.0); }
    let degrees = if let Some(value) = value.strip_suffix("deg") {
        finite(value)?
    } else if let Some(value) = value.strip_suffix("grad") {
        finite(value)? * 0.9
    } else if let Some(value) = value.strip_suffix("rad") {
        finite(value)? * 180.0 / PI
    } else if let Some(value) = value.strip_suffix("turn") {
        finite(value)? * 360.0
    } else {
        finite(value)?
    };
    Some(degrees.rem_euclid(360.0))
}

fn alpha(value: &str) -> Option<u8> {
    if is_none(value) { return Some(0); }
    let value = percentage(value).map(|number| number / 100.0).or_else(|| finite(value))?;
    Some((value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn components(inner: &str, allow_commas: bool) -> Option<(Vec<String>, u8)> {
    if inner.contains(',') {
        if !allow_commas || inner.contains('/') { return None; }
        let values: Vec<_> = inner.split(',').map(str::trim).collect();
        if !matches!(values.len(), 3 | 4) || values.iter().any(|value| value.is_empty()) {
            return None;
        }
        let a = if values.len() == 4 { alpha(values[3])? } else { 255 };
        return Some((values[..3].iter().map(|value| (*value).to_owned()).collect(), a));
    }
    let normalized = inner.replace('/', " / ");
    let values: Vec<_> = normalized.split_ascii_whitespace().collect();
    match values.as_slice() {
        [x, y, z] => Some((vec![(*x).to_owned(), (*y).to_owned(), (*z).to_owned()], 255)),
        [x, y, z, slash, a] if *slash == "/" => {
            Some((vec![(*x).to_owned(), (*y).to_owned(), (*z).to_owned()], alpha(a)?))
        },
        _ => None,
    }
}

fn byte(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn rgba(rgb: [f64; 3], alpha: u8) -> [u8; 4] {
    [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), alpha]
}

fn hsl_rgb(hue: f64, saturation: f64, lightness: f64) -> [f64; 3] {
    let saturation = saturation.max(0.0) / 100.0;
    let lightness = lightness / 100.0;
    [0.0, 8.0, 4.0].map(|n| {
        let k = (n + hue / 30.0).rem_euclid(12.0);
        let a = saturation * lightness.min(1.0 - lightness);
        lightness - a * (-1.0_f64).max((k - 3.0).min(9.0 - k).min(1.0))
    })
}

pub(super) fn hsl(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, true)?;
    let saturation = none_component(&values[1]).or_else(|| percentage(&values[1]))?;
    let lightness = none_component(&values[2]).or_else(|| percentage(&values[2]))?;
    Some(rgba(hsl_rgb(hue(&values[0])?, saturation, lightness), a))
}

pub(super) fn hwb(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let h = hue(&values[0])?;
    let white = none_component(&values[1]).or_else(|| percentage(&values[1]))? / 100.0;
    let black = none_component(&values[2]).or_else(|| percentage(&values[2]))? / 100.0;
    Some(rgba(hwb_rgb(h, white, black), a))
}

fn matrix(matrix: [[f64; 3]; 3], values: [f64; 3]) -> [f64; 3] {
    matrix.map(|row| row[0] * values[0] + row[1] * values[1] + row[2] * values[2])
}

fn xyz_to_srgb(xyz: [f64; 3]) -> [f64; 3] {
    let linear = matrix([
        [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
        [-851781.0 / 878810.0, 1648619.0 / 878810.0, 36519.0 / 878810.0],
        [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
    ], xyz);
    linear.map(|value| {
        let sign = value.signum();
        let magnitude = value.abs();
        if magnitude > 0.0031308 {
            sign * (1.055 * magnitude.powf(1.0 / 2.4) - 0.055)
        } else {
            12.92 * value
        }
    })
}

pub(super) fn predefined_rgb(inner: &str) -> Option<[u8; 4]> {
    let mut words = inner.split_ascii_whitespace();
    let space = words.next()?.to_ascii_lowercase();
    let rest = words.collect::<Vec<_>>().join(" ");
    let (values, a) = components(&rest, false)?;
    let channels = [0, 1, 2].map(|i| {
        none_component(&values[i])
            .or_else(|| percentage(&values[i]).map(|n| n / 100.0))
            .or_else(|| finite(&values[i]))
    });
    color_space_rgba(&space, [channels[0]?, channels[1]?, channels[2]?], a)
}

// Predefined `color()` space matrices (CSS Color 4 §19 sample code).
const P3_TO_XYZ: [[f64; 3]; 3] = [
    [608311.0 / 1250200.0, 189793.0 / 714400.0, 198249.0 / 1000160.0],
    [35783.0 / 156275.0, 247089.0 / 357200.0, 198249.0 / 2500400.0],
    [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
];
const XYZ_TO_P3: [[f64; 3]; 3] = [
    [2.493496911941425, -0.9313836179191239, -0.40271078445071684],
    [-0.8294889695615747, 1.7626640603183463, 0.023624685841943577],
    [0.03584583024378447, -0.07617238926804182, 0.9568845240076872],
];
const A98_TO_XYZ: [[f64; 3]; 3] = [
    [573536.0 / 994567.0, 263643.0 / 1420810.0, 187206.0 / 994567.0],
    [591459.0 / 1989134.0, 6239551.0 / 9945670.0, 374412.0 / 4972835.0],
    [53769.0 / 1989134.0, 351524.0 / 4972835.0, 4929758.0 / 4972835.0],
];
const XYZ_TO_A98: [[f64; 3]; 3] = [
    [2.041587903810746, -0.5650069742788594, -0.3447313507783295],
    [-0.9692436362808797, 1.87596750150772, 0.04155505740717561],
    [0.01344428063203101, -0.1183623922310182, 1.015174994391205],
];
const PROPHOTO_TO_XYZ: [[f64; 3]; 3] = [
    [0.79776664490064230, 0.13518129740053308, 0.03134773412839220],
    [0.28807482881940130, 0.71183523424187300, 0.00008993693872564],
    [0.0, 0.0, 0.82510460251046020],
];
const XYZ_TO_PROPHOTO: [[f64; 3]; 3] = [
    [1.3457989731028281, -0.25558010007997534, -0.05110628506753401],
    [-0.5446224939028347, 1.5082327413132781, 0.02053603239147973],
    [0.0, 0.0, 1.2119675456389454],
];
const REC2020_TO_XYZ: [[f64; 3]; 3] = [
    [63426534.0 / 99577255.0, 20160776.0 / 139408157.0, 47086771.0 / 278816314.0],
    [26158966.0 / 99577255.0, 472592308.0 / 697040785.0, 8267143.0 / 139408157.0],
    [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
];
const XYZ_TO_REC2020: [[f64; 3]; 3] = [
    [1.716651187971268, -0.355670783776392, -0.253366281373660],
    [-0.666684351832489, 1.616481236634939, 0.0157685458139111],
    [0.017639857445311, -0.042770613257809, 0.942103121235474],
];
const D65_TO_D50: [[f64; 3]; 3] = [
    [1.0479298208405488, 0.022946793341019088, -0.05019222954313557],
    [0.029627815688159344, 0.990434484573249, -0.01707382502938514],
    [-0.009243058152591178, 0.015055144896577895, 0.7518742899580008],
];

/// Convert numeric `color(<space> c1 c2 c3)` channels into RGBA.
fn color_space_rgba(space: &str, channels: [f64; 3], alpha: u8) -> Option<[u8; 4]> {
    if space == "srgb" { return Some(rgba(channels, alpha)); }
    if space == "srgb-linear" {
        return Some(rgba(channels.map(srgb_encode), alpha));
    }
    let xyz = match space {
        "xyz" | "xyz-d65" => channels,
        "xyz-d50" => d50_to_d65(channels),
        "display-p3" | "display-p3-linear" => matrix(
            P3_TO_XYZ,
            if space == "display-p3" { channels.map(srgb_decode) } else { channels },
        ),
        "a98-rgb" => matrix(
            A98_TO_XYZ,
            channels.map(|value| value.signum() * value.abs().powf(563.0 / 256.0)),
        ),
        "rec2020" => matrix(REC2020_TO_XYZ, channels.map(rec2020_decode)),
        "prophoto-rgb" => d50_to_d65(matrix(
            PROPHOTO_TO_XYZ,
            channels.map(|value| {
                if value.abs() <= 16.0 / 512.0 { value / 16.0 }
                else { value.signum() * value.abs().powf(1.8) }
            }),
        )),
        _ => return None,
    };
    Some(rgba(xyz_to_srgb(xyz), alpha))
}

/// The source color's coordinates in a predefined `color()` space, used by the
/// relative-color producer.
fn rgba_to_color_space(space: &str, rgb: [u8; 4]) -> Option<[f64; 3]> {
    Some(match space {
        "srgb" => [0, 1, 2].map(|i| rgb[i] as f64 / 255.0),
        "srgb-linear" => [0, 1, 2].map(|i| srgb_decode(rgb[i] as f64 / 255.0)),
        "display-p3" => matrix(XYZ_TO_P3, srgb_to_xyz_d65(rgb)).map(srgb_encode),
        "display-p3-linear" => matrix(XYZ_TO_P3, srgb_to_xyz_d65(rgb)),
        "a98-rgb" => matrix(XYZ_TO_A98, srgb_to_xyz_d65(rgb))
            .map(|value| value.signum() * value.abs().powf(256.0 / 563.0)),
        "prophoto-rgb" => matrix(XYZ_TO_PROPHOTO, matrix(D65_TO_D50, srgb_to_xyz_d65(rgb)))
            .map(|value| {
                if value.abs() <= 16.0 / 512.0 { value * 16.0 }
                else { value.signum() * value.abs().powf(1.0 / 1.8) }
            }),
        "rec2020" => matrix(XYZ_TO_REC2020, srgb_to_xyz_d65(rgb)).map(rec2020_encode),
        "xyz" | "xyz-d65" => srgb_to_xyz_d65(rgb),
        "xyz-d50" => matrix(D65_TO_D50, srgb_to_xyz_d65(rgb)),
        _ => return None,
    })
}

fn rec2020_decode(value: f64) -> f64 {
    // ITU-R BT.2020 inverse transfer function, as used by CSS Color 4.
    const ALPHA: f64 = 1.09929682680944;
    if value.abs() < 4.5 * 0.018053968510807 {
        value / 4.5
    } else {
        value.signum() * ((value.abs() + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
    }
}

fn rec2020_encode(value: f64) -> f64 {
    const ALPHA: f64 = 1.09929682680944;
    const BETA: f64 = 0.018053968510807;
    if value.abs() < BETA {
        value * 4.5
    } else {
        value.signum() * (ALPHA * value.abs().powf(0.45) - (ALPHA - 1.0))
    }
}

fn srgb_decode(value: f64) -> f64 {
    if value.abs() <= 0.04045 { value / 12.92 }
    else { value.signum() * ((value.abs() + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(value: f64) -> f64 {
    if value.abs() > 0.0031308 { value.signum() * (1.055 * value.abs().powf(1.0 / 2.4) - 0.055) }
    else { 12.92 * value }
}

fn d50_to_d65(xyz: [f64; 3]) -> [f64; 3] {
    matrix([
        [0.955473421488075, -0.02309845494876471, 0.06325924320057072],
        [-0.0283697093338637, 1.0099953980813041, 0.021041441191917323],
        [0.012314014864481998, -0.020507649298898964, 1.330365926242124],
    ], xyz)
}

pub(super) fn lab(inner: &str) -> Option<[u8; 4]> {
    let (values, alpha) = components(inner, false)?;
    let lightness = none_component(&values[0]).or_else(|| percentage(&values[0]))
        .or_else(|| finite(&values[0]))?.clamp(0.0, 100.0);
    let lab_a = none_component(&values[1])
        .or_else(|| percentage(&values[1]).map(|value| value * 1.25))
        .or_else(|| finite(&values[1]))?;
    let lab_b = none_component(&values[2])
        .or_else(|| percentage(&values[2]).map(|value| value * 1.25))
        .or_else(|| finite(&values[2]))?;
    if lightness == 0.0 { return Some([0, 0, 0, alpha]); }
    if lightness == 100.0 { return Some([255, 255, 255, alpha]); }
    Some(lab_color(lightness, lab_a, lab_b, alpha))
}

pub(super) fn lch(inner: &str) -> Option<[u8; 4]> {
    let (values, alpha) = components(inner, false)?;
    let lightness = none_component(&values[0]).or_else(|| percentage(&values[0]))
        .or_else(|| finite(&values[0]))?.clamp(0.0, 100.0);
    let chroma = none_component(&values[1])
        .or_else(|| percentage(&values[1]).map(|value| value * 1.5))
        .or_else(|| finite(&values[1]))?.max(0.0);
    let angle = hue(&values[2])? * PI / 180.0;
    if lightness == 0.0 { return Some([0, 0, 0, alpha]); }
    if lightness == 100.0 { return Some([255, 255, 255, alpha]); }
    Some(lab_color(lightness, chroma * angle.cos(), chroma * angle.sin(), alpha))
}

fn lab_color(lightness: f64, lab_a: f64, lab_b: f64, alpha: u8) -> [u8; 4] {
    let kappa = 24389.0 / 27.0;
    let epsilon = 216.0 / 24389.0;
    let fy = (lightness + 16.0) / 116.0;
    let fx = lab_a / 500.0 + fy;
    let fz = fy - lab_b / 200.0;
    let d50 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
    let xyz_d50 = [
        (if fx.powi(3) > epsilon { fx.powi(3) } else { (116.0 * fx - 16.0) / kappa }) * d50[0],
        (if lightness > kappa * epsilon { fy.powi(3) } else { lightness / kappa }) * d50[1],
        (if fz.powi(3) > epsilon { fz.powi(3) } else { (116.0 * fz - 16.0) / kappa }) * d50[2],
    ];
    let xyz_d65 = matrix([
        [0.955473421488075, -0.02309845494876471, 0.06325924320057072],
        [-0.0283697093338637, 1.0099953980813041, 0.021041441191917323],
        [0.012314014864481998, -0.020507649298898964, 1.330365926242124],
    ], xyz_d50);
    rgba(xyz_to_srgb(xyz_d65), alpha)
}

/// CSS Color 4 §19 sample matrices, with XYZ relative to D65.
fn oklab_xyz(lab: [f64; 3]) -> [f64; 3] {
    let lms = matrix([
        [1.0, 0.3963377773761749, 0.2158037573099136],
        [1.0, -0.1055613458156586, -0.0638541728258133],
        [1.0, -0.0894841775298119, -1.2914855480194092],
    ], lab).map(|channel| channel.powi(3));
    matrix([
        [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
        [-0.0405757452148008, 1.1122868032803170, -0.0717110580655164],
        [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
    ], lms)
}

fn ok_component(value: &str, percentage_scale: f64) -> Option<f64> {
    none_component(value)
        .or_else(|| percentage(value).map(|n| n * percentage_scale / 100.0))
        .or_else(|| finite(value))
}

pub(super) fn oklab(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let lab = [
        ok_component(&values[0], 1.0)?.clamp(0.0, 1.0),
        ok_component(&values[1], 0.4)?,
        ok_component(&values[2], 0.4)?,
    ];
    if lab[0] <= 1e-6 { return Some([0, 0, 0, a]); }
    if lab[0] == 1.0 { return Some([255, 255, 255, a]); }
    Some(rgba(xyz_to_srgb(oklab_xyz(lab)), a))
}

pub(super) fn oklch(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let lightness = ok_component(&values[0], 1.0)?.clamp(0.0, 1.0);
    let chroma = ok_component(&values[1], 0.4)?.max(0.0);
    let angle = hue(&values[2])? * PI / 180.0;
    if lightness <= 1e-6 { return Some([0, 0, 0, a]); }
    if lightness == 1.0 { return Some([255, 255, 255, a]); }
    let lab = [lightness, chroma * angle.cos(), chroma * angle.sin()];
    Some(rgba(xyz_to_srgb(oklab_xyz(lab)), a))
}

/// CSS Color 4 §12.2 hue-interpolation methods for the polar spaces
/// (`hsl`, `hwb`, `lch`, `oklch`): `shorter hue` is the default.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HueMethod { Shorter, Longer, Increasing, Decreasing }

fn mix_hue(h1: f64, h2: f64, w2: f64, method: HueMethod) -> f64 {
    let shorter = (h2 - h1 + 540.0).rem_euclid(360.0) - 180.0;
    let delta = match method {
        HueMethod::Shorter => shorter,
        HueMethod::Longer => {
            if shorter == 0.0 { 0.0 } else if shorter > 0.0 { shorter - 360.0 } else { shorter + 360.0 }
        }
        HueMethod::Increasing => (h2 - h1).rem_euclid(360.0),
        HueMethod::Decreasing => (h2 - h1).rem_euclid(360.0) - 360.0,
    };
    (h1 + delta * w2).rem_euclid(360.0)
}

/// CSS Color 4 §13: a channel missing (`none`) in exactly one operand takes
/// the other operand's value unattenuated by its mix weight; missing in
/// both stays 0 (there is no further consumer to propagate it to here).
fn mix_channel(v1: f64, missing1: bool, v2: f64, missing2: bool, w1: f64, w2: f64) -> f64 {
    match (missing1, missing2) {
        (true, true) => 0.0,
        (true, false) => v2,
        (false, true) => v1,
        (false, false) => v1 * w1 + v2 * w2,
    }
}

/// CSS Color 5's two-color mix. Keep interpolation in its declared space;
/// converting the endpoints to sRGB before mixing gives visibly wrong hues.
/// Spaces: the rectangular `srgb`/`srgb-linear`/`lab`/`oklab`/`xyz`/
/// `xyz-d65`/`xyz-d50`, and the polar `hsl`/`hwb`/`lch`/`oklch` (each with an
/// optional `<hue-method> hue` after the space name).
pub(super) fn color_mix(inner: &str, depth: usize) -> Option<[u8; 4]> {
    let parts = split_top_level_commas(inner)?;
    let [method, first, second] = parts.as_slice() else { return None };
    let (space, hue_method) = parse_mix_method(method)?;
    let (first, p1, missing1) = mix_item(first, depth, &space)?;
    let (second, p2, missing2) = mix_item(second, depth, &space)?;
    let (p1, p2) = match (p1, p2) {
        (None, None) => (0.5, 0.5),
        (Some(a), None) => (a, 1.0 - a),
        (None, Some(b)) => (1.0 - b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = p1 + p2;
    if sum <= 0.0 { return None; }
    let multiplier = sum.min(1.0);
    let p1 = p1 / sum;
    let p2 = p2 / sum;
    let a1 = first[3] as f64 / 255.0;
    let a2 = second[3] as f64 / 255.0;
    let alpha = mix_channel(a1, missing1[3], a2, missing2[3], p1, p2);
    // The premultiplication weights need each operand's *effective* alpha
    // (SS13: a missing component acts as if it were set equal to the other
    // operand's value), not the literal 0 the parser stores for a `none`
    // alpha -- using the raw 0 would zero out that operand's contribution to
    // every other channel instead of leaving it unaffected.
    let eff_a1 = if missing1[3] { a2 } else { a1 };
    let eff_a2 = if missing2[3] { a1 } else { a2 };
    let (w1, w2) = if alpha > 0.0 { (eff_a1 * p1 / alpha, eff_a2 * p2 / alpha) }
        else { (p1, p2) };
    let output_alpha = byte(alpha * multiplier);
    let polar_hue_index = match space.as_str() {
        "hsl" | "hwb" => Some(0),
        "lch" | "oklch" => Some(2),
        _ => None,
    };
    let (one, two): ([f64; 3], [f64; 3]) = match space.as_str() {
        "srgb" => ([0, 1, 2].map(|i| first[i] as f64 / 255.0), [0, 1, 2].map(|i| second[i] as f64 / 255.0)),
        "srgb-linear" => (
            [0, 1, 2].map(|i| srgb_decode(first[i] as f64 / 255.0)),
            [0, 1, 2].map(|i| srgb_decode(second[i] as f64 / 255.0)),
        ),
        "lab" => (srgb_to_lab(first), srgb_to_lab(second)),
        "oklab" => (srgb_to_oklab(first), srgb_to_oklab(second)),
        "xyz" | "xyz-d65" => (srgb_to_xyz_d65(first), srgb_to_xyz_d65(second)),
        "xyz-d50" => (srgb_to_xyz_d50(first), srgb_to_xyz_d50(second)),
        "hsl" => (srgb_to_hsl(first), srgb_to_hsl(second)),
        "hwb" => (srgb_to_hwb(first), srgb_to_hwb(second)),
        "lch" => (srgb_to_lch(first), srgb_to_lch(second)),
        "oklch" => (srgb_to_oklch(first), srgb_to_oklch(second)),
        _ => return None,
    };
    let mixed = if let Some(hue_index) = polar_hue_index {
        // An achromatic side or a literally missing hue contributes nothing
        // directional; the other side's hue carries through unmixed. `hwb`'s
        // achromatic test is `white + black >= 100`, not a near-zero channel.
        let (one_moot, two_moot) = if space == "hwb" {
            (one[1] + one[2] >= 100.0 - 1e-4 || missing1[0], two[1] + two[2] >= 100.0 - 1e-4 || missing2[0])
        } else {
            (one[1].abs() < 1e-4 || missing1[hue_index], two[1].abs() < 1e-4 || missing2[hue_index])
        };
        let other = [0, 1, 2].into_iter().filter(|&i| i != hue_index);
        let mut mixed = [0.0; 3];
        let h1 = if one_moot { two[hue_index] } else { one[hue_index] };
        let h2 = if two_moot { h1 } else { two[hue_index] };
        mixed[hue_index] = if missing1[hue_index] && missing2[hue_index] { 0.0 }
            else { mix_hue(h1, h2, w2, hue_method) };
        for i in other {
            mixed[i] = mix_channel(one[i], missing1[i], two[i], missing2[i], w1, w2);
        }
        mixed
    } else {
        [0, 1, 2].map(|i| mix_channel(one[i], missing1[i], two[i], missing2[i], w1, w2))
    };
    Some(match space.as_str() {
        "srgb" => rgba(mixed, output_alpha),
        "srgb-linear" => rgba(mixed.map(srgb_encode), output_alpha),
        "lab" => lab_color(mixed[0], mixed[1], mixed[2], output_alpha),
        "oklab" => rgba(xyz_to_srgb(oklab_xyz(mixed)), output_alpha),
        "xyz" | "xyz-d65" => rgba(xyz_to_srgb(mixed), output_alpha),
        "xyz-d50" => rgba(xyz_to_srgb(d50_to_d65(mixed)), output_alpha),
        "hsl" => rgba(hsl_rgb(mixed[0], mixed[1], mixed[2]), output_alpha),
        "hwb" => rgba(hwb_rgb(mixed[0], mixed[1] / 100.0, mixed[2] / 100.0), output_alpha),
        "lch" | "oklch" => {
            let angle = mixed[2].to_radians();
            let (a, b) = (mixed[1] * angle.cos(), mixed[1] * angle.sin());
            if space == "lch" { lab_color(mixed[0], a, b, output_alpha) }
            else { rgba(xyz_to_srgb(oklab_xyz([mixed[0], a, b])), output_alpha) }
        }
        _ => return None,
    })
}

/// `in <space> [<shorter | longer | increasing | decreasing> hue]?`.
fn parse_mix_method(method: &str) -> Option<(String, HueMethod)> {
    let method = method.trim().strip_prefix("in ")?.trim();
    let mut words = method.split_ascii_whitespace();
    let space = words.next()?.to_ascii_lowercase();
    if !matches!(space.as_str(), "srgb" | "srgb-linear" | "lab" | "oklab" | "xyz" | "xyz-d65"
        | "xyz-d50" | "hsl" | "hwb" | "lch" | "oklch") {
        return None;
    }
    let hue_method = match words.next() {
        None => HueMethod::Shorter,
        Some(word) => {
            let method = match word.to_ascii_lowercase().as_str() {
                "shorter" => HueMethod::Shorter,
                "longer" => HueMethod::Longer,
                "increasing" => HueMethod::Increasing,
                "decreasing" => HueMethod::Decreasing,
                _ => return None,
            };
            if !words.next().is_some_and(|word| word.eq_ignore_ascii_case("hue")) {
                return None;
            }
            method
        }
    };
    if words.next().is_some() { return None; }
    Some((space, hue_method))
}

/// `xyz` is an alias for `xyz-d65` (CSS Color 4 SS10.5): treat them as the
/// same space when matching an operand's literal function against the
/// `color-mix()` space it is being mixed in.
fn normalize_space_alias(space: &str) -> &str {
    if space == "xyz" { "xyz-d65" } else { space }
}

/// Whether `color_text` is written directly in `space`'s own function
/// (`hsl()`/`hwb()`/`lab()`/`lch()`/`oklab()`/`oklch()` absolute form,
/// `rgb()`/`rgba()` for `srgb`, or `color(<space> ...)` for `srgb`,
/// `srgb-linear` and the `xyz`/`xyz-d65`/`xyz-d50` family) so a `none`
/// channel there means the *same* channel is missing in the mix. An operand
/// written in a different function or a different `color()` space keeps the
/// established none-is-0 behavior; propagating missing-ness across a space
/// conversion is not attempted.
fn none_mask(color_text: &str, space: &str) -> [bool; 4] {
    let trimmed = color_text.trim();
    let Some(open) = trimmed.find('(') else { return [false; 4] };
    if !trimmed.ends_with(')') { return [false; 4]; }
    let name = trimmed[..open].trim().to_ascii_lowercase();
    let inner = &trimmed[open + 1..trimmed.len() - 1];
    let (canonical, inner): (&str, &str) = match name.as_str() {
        "hsl" | "hsla" => ("hsl", inner),
        "hwb" => ("hwb", inner),
        "lab" => ("lab", inner),
        "lch" => ("lch", inner),
        "oklab" => ("oklab", inner),
        "oklch" => ("oklch", inner),
        "rgb" | "rgba" => ("srgb", inner),
        "color" => {
            let trimmed_inner = inner.trim_start();
            let (space_name, rest) = trimmed_inner
                .split_once(char::is_whitespace)
                .unwrap_or((trimmed_inner, ""));
            let canonical = match space_name.to_ascii_lowercase().as_str() {
                "srgb" => "srgb",
                "srgb-linear" => "srgb-linear",
                "xyz" | "xyz-d65" => "xyz-d65",
                "xyz-d50" => "xyz-d50",
                _ => return [false; 4],
            };
            (canonical, rest)
        }
        _ => return [false; 4],
    };
    if canonical != normalize_space_alias(space) { return [false; 4]; }
    if inner.trim_start().to_ascii_lowercase().starts_with("from") { return [false; 4]; }
    let normalized = inner.replace([',', '/'], " ");
    let mut mask = [false; 4];
    for (i, token) in normalized.split_ascii_whitespace().take(4).enumerate() {
        mask[i] = token.eq_ignore_ascii_case("none");
    }
    mask
}

fn mix_item(item: &str, depth: usize, space: &str) -> Option<([u8; 4], Option<f64>, [bool; 4])> {
    let item = item.trim();
    let mut mix_percent = None;
    let mut color = item;
    if let Some((first, rest)) = item.split_once(' ') {
        if let Some(p) = percentage(first) {
            mix_percent = Some(p / 100.0);
            color = rest.trim();
        }
    }
    if mix_percent.is_none() {
        if let Some((rest, last)) = item.rsplit_once(' ') {
            if let Some(p) = percentage(last) {
                mix_percent = Some(p / 100.0);
                color = rest.trim();
            }
        }
    }
    if mix_percent.is_some_and(|p| !(0.0..=1.0).contains(&p)) { return None; }
    let rgba = super::parse_css_color_depth(color, depth + 1)?;
    Some((rgba, mix_percent, none_mask(color, space)))
}

fn split_top_level_commas(value: &str) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            ',' if depth == 0 => { parts.push(value[start..i].trim()); start = i + 1; }
            _ => {}
        }
    }
    if depth != 0 { return None; }
    parts.push(value[start..].trim());
    (!parts.iter().any(|part| part.is_empty())).then_some(parts)
}

fn srgb_to_xyz_d65(rgb: [u8; 4]) -> [f64; 3] {
    let linear = [0, 1, 2].map(|i| srgb_decode(rgb[i] as f64 / 255.0));
    matrix([
        [0.41239079926595934, 0.357584339383878, 0.1804807884018343],
        [0.21263900587151027, 0.715168678767756, 0.07219231536073371],
        [0.01933081871559185, 0.11919477979462599, 0.9505321522496607],
    ], linear)
}

fn srgb_to_xyz_d50(rgb: [u8; 4]) -> [f64; 3] {
    matrix([
        [1.0479298208405488, 0.022946793341019088, -0.05019222954313557],
        [0.029627815688159344, 0.990434484573249, -0.01707382502938514],
        [-0.009243058152591178, 0.015055144896577895, 0.7518742899580008],
    ], srgb_to_xyz_d65(rgb))
}

fn srgb_to_lab(rgb: [u8; 4]) -> [f64; 3] {
    let d50 = srgb_to_xyz_d50(rgb);
    let white = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
    let epsilon = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let f = [0, 1, 2].map(|i| {
        let value = d50[i] / white[i];
        if value > epsilon { value.cbrt() } else { (kappa * value + 16.0) / 116.0 }
    });
    [116.0 * f[1] - 16.0, 500.0 * (f[0] - f[1]), 200.0 * (f[1] - f[2])]
}

fn srgb_to_hsl(rgb: [u8; 4]) -> [f64; 3] {
    let r = rgb[0] as f64 / 255.0;
    let g = rgb[1] as f64 / 255.0;
    let b = rgb[2] as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut hue = 0.0;
    if delta > 0.0 {
        hue = if max == r {
            ((g - b) / delta).rem_euclid(6.0)
        } else if max == g {
            (b - r) / delta + 2.0
        } else {
            (r - g) / delta + 4.0
        } * 60.0;
    }
    let lightness = (max + min) / 2.0;
    let saturation = if delta == 0.0 {
        0.0
    } else {
        delta / (1.0 - (2.0 * lightness - 1.0).abs())
    };
    [hue, saturation * 100.0, lightness * 100.0]
}

fn srgb_to_hwb(rgb: [u8; 4]) -> [f64; 3] {
    let [hue, _, _] = srgb_to_hsl(rgb);
    let max = rgb[..3].iter().copied().max().unwrap_or(0) as f64 / 255.0;
    let min = rgb[..3].iter().copied().min().unwrap_or(0) as f64 / 255.0;
    [hue, min * 100.0, (1.0 - max) * 100.0]
}

/// CSS Color 4 §19 sample matrices, XYZ relative to D65.
fn srgb_to_oklab(rgb: [u8; 4]) -> [f64; 3] {
    let xyz = srgb_to_xyz_d65(rgb);
    let lms = matrix([
        [0.8190224379967030, 0.3619062600528904, -0.1288737815209879],
        [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
        [0.0481771893596242, 0.2642395317527308, 0.6335478284694309],
    ], xyz).map(f64::cbrt);
    matrix([
        [0.2104542683093140, 0.7936177747023054, -0.0040720430116193],
        [1.9779985324311684, -2.4285922420485799, 0.4505937096174110],
        [0.0259040424655478, 0.7827717124575296, -0.8086757549230774],
    ], lms)
}

fn srgb_to_lch(rgb: [u8; 4]) -> [f64; 3] {
    let [l, a, b] = srgb_to_lab(rgb);
    [l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0)]
}

fn srgb_to_oklch(rgb: [u8; 4]) -> [f64; 3] {
    let [l, a, b] = srgb_to_oklab(rgb);
    [l, a.hypot(b), b.atan2(a).to_degrees().rem_euclid(360.0)]
}

pub(super) fn contrast_color(inner: &str, depth: usize) -> Option<[u8; 4]> {
    let color = super::parse_css_color_depth(inner.trim(), depth + 1)?;
    let alpha = color[3] as f64 / 255.0;
    let luminance = [0, 1, 2].into_iter().zip([0.2126, 0.7152, 0.0722])
        .map(|(i, weight)| srgb_decode((color[i] as f64 * alpha + 255.0 * (1.0 - alpha)) / 255.0) * weight)
        .sum::<f64>();
    Some(if luminance > 0.179 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })
}

// ---------------------------------------------------------------------------
// CSS Color 5 relative color syntax
// ---------------------------------------------------------------------------

/// True when a color function body starts with the `from <color>` producer.
pub(super) fn has_from(inner: &str) -> bool {
    let rest = inner.trim_start();
    let Some(head) = rest.get(..4) else { return false };
    head.eq_ignore_ascii_case("from") && rest[4..].starts_with(char::is_whitespace)
}

/// `rgb(from <color> r g b / a)` and the equivalent forms for the numeric
/// channel functions. Returns `None` when the source is `currentcolor`, which
/// the computed-value seam resolves against the element's own `color`.
pub(super) fn relative(function: &str, inner: &str, depth: usize) -> Option<[u8; 4]> {
    let function = function.to_ascii_lowercase();
    if function == "color" {
        return relative_color_space(inner, depth);
    }
    let (source, channels, alpha_text) = relative_parts(inner, depth)?;
    let keywords = source_keywords(&function, source)?;
    let channel = |index: usize| channel_value(&function, index, channels[index], &keywords);
    let c = [channel(0)?, channel(1)?, channel(2)?];
    let alpha = match alpha_text {
        Some(text) => alpha_channel(text, &keywords, source[3])?,
        None => source[3],
    };
    Some(match function.as_str() {
        "rgb" | "rgba" => [
            byte(c[0] / 255.0), byte(c[1] / 255.0), byte(c[2] / 255.0), alpha,
        ],
        "hsl" | "hsla" => rgba(hsl_rgb(c[0], c[1], c[2]), alpha),
        "hwb" => rgba(hwb_rgb(c[0], c[1] / 100.0, c[2] / 100.0), alpha),
        "lab" => lab_color(c[0].clamp(0.0, 100.0), c[1], c[2], alpha),
        "lch" => lab_color(
            c[0].clamp(0.0, 100.0),
            c[1] * c[2].to_radians().cos(),
            c[1] * c[2].to_radians().sin(),
            alpha,
        ),
        "oklab" => rgba(xyz_to_srgb(oklab_xyz([c[0].clamp(0.0, 1.0), c[1], c[2]])), alpha),
        "oklch" => rgba(
            xyz_to_srgb(oklab_xyz([
                c[0].clamp(0.0, 1.0),
                c[1] * c[2].to_radians().cos(),
                c[1] * c[2].to_radians().sin(),
            ])),
            alpha,
        ),
        _ => return None,
    })
}

/// `color(from <color> <space> <c1> <c2> <c3> / <alpha>)`. The channel
/// keywords are the target space's own axes (`r g b`, or `x y z` for XYZ); the
/// source is converted into that space first.
fn relative_color_space(inner: &str, depth: usize) -> Option<[u8; 4]> {
    let rest = inner.trim_start().get(4..)?.trim_start();
    let (source_text, after) = split_source(rest)?;
    if source_text.eq_ignore_ascii_case("currentcolor") {
        return None;
    }
    let source = super::parse_css_color_depth(source_text, depth + 1)?;
    let (channels_text, alpha_text) = split_alpha(after)?;
    let words = top_level_words(channels_text);
    let [space, c1, c2, c3] = words.as_slice() else { return None };
    let space = space.to_ascii_lowercase();
    let base = rgba_to_color_space(&space, source)?;
    let names: [&'static str; 3] = if matches!(space.as_str(), "xyz" | "xyz-d50" | "xyz-d65") {
        ["x", "y", "z"]
    } else {
        ["r", "g", "b"]
    };
    let keywords: Vec<(&'static str, f64)> = vec![
        (names[0], base[0]),
        (names[1], base[1]),
        (names[2], base[2]),
    ];
    let channels = [
        number_channel(c1, &keywords, 1.0, 1.0)?,
        number_channel(c2, &keywords, 1.0, 1.0)?,
        number_channel(c3, &keywords, 1.0, 1.0)?,
    ];
    let alpha = match alpha_text {
        Some(text) => alpha_channel(text, &keywords, source[3])?,
        None => source[3],
    };
    color_space_rgba(&space, channels, alpha)
}

/// Splits `from <color> <channels> [ / <alpha> ]`.
fn relative_parts(inner: &str, depth: usize) -> Option<([u8; 4], Vec<&str>, Option<&str>)> {
    let rest = inner.trim_start().get(4..)?.trim_start();
    let (source_text, after) = split_source(rest)?;
    if source_text.eq_ignore_ascii_case("currentcolor") {
        return None;
    }
    let source = super::parse_css_color_depth(source_text, depth + 1)?;
    let (channels_text, alpha_text) = split_alpha(after)?;
    let channels = top_level_words(channels_text);
    (channels.len() == 3).then_some((source, channels, alpha_text))
}

/// Splits a leading `<color>` from the channel list. Handles hex, the
/// functional form (matching nested parens) and a bare named color.
fn split_source(rest: &str) -> Option<(&str, &str)> {
    if rest.starts_with('#') {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        return Some((&rest[..end], &rest[end..]));
    }
    if let Some(open) = rest.find('(') {
        let name = &rest[..open];
        if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            let mut depth = 0usize;
            for (offset, ch) in rest[open..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            let end = open + offset + 1;
                            return Some((&rest[..end], &rest[end..]));
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    (!rest[..end].is_empty()).then_some((&rest[..end], &rest[end..]))
}

/// Splits the top-level `/` alpha separator, respecting nested parens.
fn split_alpha(rest: &str) -> Option<(&str, Option<&str>)> {
    let mut depth = 0usize;
    for (i, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.checked_sub(1)?,
            '/' if depth == 0 => {
                let channels = rest[..i].trim();
                let alpha = rest[i + 1..].trim();
                return (!alpha.is_empty()).then_some((channels, Some(alpha)));
            }
            _ => {}
        }
    }
    Some((rest.trim(), None))
}

/// Whitespace-separated tokens at paren depth zero.
fn top_level_words(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                let token = value[start..i].trim();
                if !token.is_empty() {
                    out.push(token);
                }
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    let tail = value[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn source_keywords(function: &str, source: [u8; 4]) -> Option<Vec<(&'static str, f64)>> {
    let keywords = match function {
        "rgb" | "rgba" => vec![
            ("r", source[0] as f64),
            ("g", source[1] as f64),
            ("b", source[2] as f64),
        ],
        "hsl" | "hsla" => {
            let [h, s, l] = srgb_to_hsl(source);
            vec![("h", h), ("s", s), ("l", l)]
        }
        "hwb" => {
            let [h, w, b] = srgb_to_hwb(source);
            vec![("h", h), ("w", w), ("b", b)]
        }
        "lab" => {
            let [l, a, b] = srgb_to_lab(source);
            vec![("l", l), ("a", a), ("b", b)]
        }
        "lch" => {
            let [l, c, h] = srgb_to_lch(source);
            vec![("l", l), ("c", c), ("h", h)]
        }
        "oklab" => {
            let [l, a, b] = srgb_to_oklab(source);
            vec![("l", l), ("a", a), ("b", b)]
        }
        "oklch" => {
            let [l, c, h] = srgb_to_oklch(source);
            vec![("l", l), ("c", c), ("h", h)]
        }
        _ => return None,
    };
    Some(keywords)
}

fn channel_value(function: &str, index: usize, token: &str, keywords: &[(&str, f64)]) -> Option<f64> {
    let hue_channel = matches!(
        (function, index),
        ("hsl" | "hsla", 0) | ("hwb", 0) | ("lch", 2) | ("oklch", 2)
    );
    if hue_channel {
        if let Some(value) = keyword(token, keywords) {
            return Some(value.rem_euclid(360.0));
        }
        if token.eq_ignore_ascii_case("none") {
            return Some(0.0);
        }
        if token.contains('(') {
            return eval_expr(token, keywords, 1.0, 1.0).map(|v| v.rem_euclid(360.0));
        }
        return hue(token);
    }
    let (percent, plain) = match (function, index) {
        ("rgb" | "rgba", _) => (255.0, 1.0),
        ("hsl" | "hsla", 1 | 2) => (1.0, 1.0),
        ("hwb", 1 | 2) => (1.0, 1.0),
        ("lab", 0) => (1.0, 1.0),
        ("lab", 1 | 2) => (125.0, 1.0),
        ("lch", 0) => (1.0, 1.0),
        ("lch", 1) => (150.0, 1.0),
        ("oklab", 0) => (1.0, 1.0),
        ("oklab", 1 | 2) => (0.4, 1.0),
        ("oklch", 0) => (1.0, 1.0),
        ("oklch", 1) => (0.4, 1.0),
        _ => return None,
    };
    number_channel(token, keywords, percent, plain)
}

fn number_channel(
    token: &str,
    keywords: &[(&str, f64)],
    percent: f64,
    plain: f64,
) -> Option<f64> {
    if let Some(value) = keyword(token, keywords) {
        return Some(value);
    }
    if token.eq_ignore_ascii_case("none") {
        return Some(0.0);
    }
    if token.contains('(') {
        return eval_expr(token, keywords, percent, plain);
    }
    percentage(token)
        .map(|value| value * percent / 100.0)
        .or_else(|| finite(token).map(|value| value * plain))
}

fn alpha_channel(token: &str, keywords: &[(&str, f64)], source_alpha: u8) -> Option<u8> {
    if token.eq_ignore_ascii_case("none") {
        return Some(0);
    }
    if token.eq_ignore_ascii_case("alpha") {
        return Some(source_alpha);
    }
    if token.contains('(') {
        let value = eval_expr(token, keywords, 1.0, 1.0)?;
        return Some((value.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    alpha(token)
}

fn keyword(token: &str, keywords: &[(&str, f64)]) -> Option<f64> {
    keywords
        .iter()
        .find(|(name, _)| token.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
}

fn hwb_rgb(hue: f64, white: f64, black: f64) -> [f64; 3] {
    if white + black >= 1.0 {
        [white / (white + black); 3]
    } else {
        hsl_rgb(hue, 100.0, 50.0).map(|channel| channel * (1.0 - white - black) + white)
    }
}

/// Evaluate a channel `calc()` (or bare arithmetic) with `keywords` bound to the
/// source channels. Percentages scale to the channel's 100% value.
fn eval_expr(css: &str, keywords: &[(&str, f64)], percent: f64, plain: f64) -> Option<f64> {
    let tokens = tokenize(css);
    let mut eval = ExprEval { tokens: &tokens, pos: 0, keywords, percent, plain };
    let value = eval.sum()?;
    eval.skip_ws();
    (eval.pos == eval.tokens.len()).then_some(value)
}

struct ExprEval<'t, 'k, 'i> {
    tokens: &'t [Token],
    pos: usize,
    keywords: &'k [(&'i str, f64)],
    percent: f64,
    plain: f64,
}

impl<'t, 'k, 'i> ExprEval<'t, 'k, 'i> {
    fn skip_ws(&mut self) {
        while matches!(self.tokens.get(self.pos), Some(Token::Whitespace)) {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Option<Token> {
        self.skip_ws();
        self.tokens.get(self.pos).cloned()
    }
    fn eat(&mut self) -> Option<Token> {
        self.skip_ws();
        let token = self.tokens.get(self.pos).cloned();
        self.pos += usize::from(token.is_some());
        token
    }
    fn sum(&mut self) -> Option<f64> {
        let mut left = self.product()?;
        loop {
            match self.peek() {
                Some(Token::Delim('+')) => {
                    self.eat();
                    left += self.product()?;
                }
                Some(Token::Delim('-')) => {
                    self.eat();
                    left -= self.product()?;
                }
                // The tokenizer emits a bare sign as an ident when it cannot
                // start a number; math.rs handles the same two spellings.
                Some(Token::Ident(name)) if name == "+" => {
                    self.eat();
                    left += self.product()?;
                }
                Some(Token::Ident(name)) if name == "-" => {
                    self.eat();
                    left -= self.product()?;
                }
                _ => break,
            }
        }
        Some(left)
    }
    fn product(&mut self) -> Option<f64> {
        let mut left = self.atom()?;
        loop {
            match self.peek() {
                Some(Token::Delim('*')) => {
                    self.eat();
                    left *= self.atom()?;
                }
                Some(Token::Delim('/')) => {
                    self.eat();
                    let right = self.atom()?;
                    if right == 0.0 {
                        return None;
                    }
                    left /= right;
                }
                _ => break,
            }
        }
        Some(left)
    }
    fn atom(&mut self) -> Option<f64> {
        match self.eat()? {
            Token::Number(number) if number.is_finite() => Some(number * self.plain),
            Token::Percentage(number) if number.is_finite() => {
                Some(number * self.percent / 100.0)
            }
            Token::Ident(name) => keyword(&name, self.keywords),
            Token::OpenParen => {
                let value = self.sum()?;
                matches!(self.eat()?, Token::CloseParen).then_some(value)
            }
            Token::Function(name) if name.eq_ignore_ascii_case("calc") => {
                if !matches!(self.eat()?, Token::OpenParen) {
                    return None;
                }
                let value = self.sum()?;
                matches!(self.eat()?, Token::CloseParen).then_some(value)
            }
            _ => None,
        }
    }
}
