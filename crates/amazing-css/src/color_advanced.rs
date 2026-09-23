//! CSS Color 4 conversions used by the CSS computed-value seam.
//! Matrices and transfer functions follow https://drafts.csswg.org/css-color/#color-conversion-code.

use std::f64::consts::PI;

fn finite(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|number| number.is_finite())
}

fn percentage(value: &str) -> Option<f64> {
    finite(value.strip_suffix('%')?)
}

fn hue(value: &str) -> Option<f64> {
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
    Some(rgba(hsl_rgb(hue(&values[0])?, percentage(&values[1])?, percentage(&values[2])?), a))
}

pub(super) fn hwb(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let h = hue(&values[0])?;
    let white = percentage(&values[1])? / 100.0;
    let black = percentage(&values[2])? / 100.0;
    let rgb = if white + black >= 1.0 {
        [white / (white + black); 3]
    } else {
        hsl_rgb(h, 100.0, 50.0).map(|channel| channel * (1.0 - white - black) + white)
    };
    Some(rgba(rgb, a))
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
        percentage(&values[i]).map(|n| n / 100.0).or_else(|| finite(&values[i]))
    });
    let channels = [channels[0]?, channels[1]?, channels[2]?];
    if space == "srgb" { return Some(rgba(channels, a)); }
    if space == "srgb-linear" {
        return Some(rgba(channels.map(srgb_encode), a));
    }
    let xyz = match space.as_str() {
        "xyz" | "xyz-d65" => channels,
        "xyz-d50" => d50_to_d65(channels),
        "display-p3" | "display-p3-linear" => matrix([
            [608311.0 / 1250200.0, 189793.0 / 714400.0, 198249.0 / 1000160.0],
            [35783.0 / 156275.0, 247089.0 / 357200.0, 198249.0 / 2500400.0],
            [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
        ], if space == "display-p3" { channels.map(srgb_decode) } else { channels }),
        "a98-rgb" => matrix([
            [573536.0 / 994567.0, 263643.0 / 1420810.0, 187206.0 / 994567.0],
            [591459.0 / 1989134.0, 6239551.0 / 9945670.0, 374412.0 / 4972835.0],
            [53769.0 / 1989134.0, 351524.0 / 4972835.0, 4929758.0 / 4972835.0],
        ], channels.map(|value| value.signum() * value.abs().powf(563.0 / 256.0))),
        "rec2020" => matrix([
            [63426534.0 / 99577255.0, 20160776.0 / 139408157.0, 47086771.0 / 278816314.0],
            [26158966.0 / 99577255.0, 472592308.0 / 697040785.0, 8267143.0 / 139408157.0],
            [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
        ], channels.map(rec2020_decode)),
        "prophoto-rgb" => d50_to_d65(matrix([
            [0.79776664490064230, 0.13518129740053308, 0.03134773412839220],
            [0.28807482881940130, 0.71183523424187300, 0.00008993693872564],
            [0.0, 0.0, 0.82510460251046020],
        ], channels.map(|value| if value.abs() <= 16.0 / 512.0 {
            value / 16.0
        } else {
            value.signum() * value.abs().powf(1.8)
        }))),
        _ => return None,
    };
    Some(rgba(xyz_to_srgb(xyz), a))
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
    let lightness = percentage(&values[0]).or_else(|| finite(&values[0]))?.clamp(0.0, 100.0);
    let lab_a = percentage(&values[1]).map(|value| value * 1.25).or_else(|| finite(&values[1]))?;
    let lab_b = percentage(&values[2]).map(|value| value * 1.25).or_else(|| finite(&values[2]))?;
    Some(lab_color(lightness, lab_a, lab_b, alpha))
}

pub(super) fn lch(inner: &str) -> Option<[u8; 4]> {
    let (values, alpha) = components(inner, false)?;
    let lightness = percentage(&values[0]).or_else(|| finite(&values[0]))?.clamp(0.0, 100.0);
    let chroma = percentage(&values[1]).map(|value| value * 1.5)
        .or_else(|| finite(&values[1]))?.max(0.0);
    let angle = hue(&values[2])? * PI / 180.0;
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
    percentage(value).map(|n| n * percentage_scale / 100.0).or_else(|| finite(value))
}

pub(super) fn oklab(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let lab = [
        ok_component(&values[0], 1.0)?.clamp(0.0, 1.0),
        ok_component(&values[1], 0.4)?,
        ok_component(&values[2], 0.4)?,
    ];
    Some(rgba(xyz_to_srgb(oklab_xyz(lab)), a))
}

pub(super) fn oklch(inner: &str) -> Option<[u8; 4]> {
    let (values, a) = components(inner, false)?;
    let lightness = ok_component(&values[0], 1.0)?.clamp(0.0, 1.0);
    let chroma = ok_component(&values[1], 0.4)?.max(0.0);
    let angle = hue(&values[2])? * PI / 180.0;
    let lab = [lightness, chroma * angle.cos(), chroma * angle.sin()];
    Some(rgba(xyz_to_srgb(oklab_xyz(lab)), a))
}
