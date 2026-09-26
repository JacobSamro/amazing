//! Owned background descriptors. No fetching, decoding, or layout traversal.
use crate::image::ObjectPosition;
use amazing_css::ComputedStyle;

pub const MAX_BACKGROUND_LAYERS: usize = 32;
const MAX_VALUE_BYTES: usize = 64 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxArea {
    Border,
    Padding,
    Content,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Repeat,
    NoRepeat,
    Space,
    Round,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeValue {
    Auto,
    Px(f32),
    Percent(f32),
    Em(f32),
    Rem(f32),
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundSize {
    Explicit(SizeValue, SizeValue),
    Cover,
    Contain,
}
#[derive(Debug, Clone)]
pub struct BackgroundLayer {
    pub url: Option<String>,
    /// Serialized CSS `<gradient>` image function for this layer, if any. At
    /// most one of `url`/`gradient` is set; paint rasterizes gradients through
    /// the shared `gradient::raster` path.
    pub gradient: Option<String>,
    pub repeat_x: Repeat,
    pub repeat_y: Repeat,
    pub position: (ObjectPosition, ObjectPosition),
    pub size: BackgroundSize,
    pub origin: BoxArea,
    pub clip: BoxArea,
}
#[derive(Debug, Clone)]
pub struct BackgroundPlan {
    pub layers: Vec<BackgroundLayer>,
    pub color_clip: BoxArea,
}

impl BackgroundPlan {
    /// Resolve longhand lists into one bounded, owned descriptor per image.
    /// `none` retains its list slot. Unsupported image functions remain empty.
    /// CSS cascade/shorthand expansion stays with amazing-css.
    pub fn from_style(style: &ComputedStyle) -> Self {
        let image = split_layers(style.get("background-image").unwrap_or("none"));
        let repeats = split_layers(style.get("background-repeat").unwrap_or("repeat"));
        let positions = split_layers(style.get("background-position").unwrap_or("0% 0%"));
        let sizes = split_layers(style.get("background-size").unwrap_or("auto"));
        let origins = split_layers(style.get("background-origin").unwrap_or("padding-box"));
        let clips = split_layers(style.get("background-clip").unwrap_or("border-box"));
        let get = |list: &Vec<&str>, index: usize, default: &'static str| -> String {
            list.get(index.checked_rem(list.len()).unwrap_or(0))
                .copied()
                .unwrap_or(default)
                .to_owned()
        };
        let mut layers = Vec::new();
        for (index, value) in image.iter().enumerate() {
            let (repeat_x, repeat_y) = repeat(&get(&repeats, index, "repeat"));
            let position = crate::image::position(&get(&positions, index, "0% 0%")).unwrap_or((
                ObjectPosition {
                    fraction: 0.0,
                    offset: 0.0,
                },
                ObjectPosition {
                    fraction: 0.0,
                    offset: 0.0,
                },
            ));
            layers.push(BackgroundLayer {
                url: url(value),
                gradient: gradient(value),
                repeat_x,
                repeat_y,
                position,
                size: size(&get(&sizes, index, "auto")),
                origin: area(&get(&origins, index, "padding-box"), BoxArea::Padding),
                clip: area(&get(&clips, index, "border-box"), BoxArea::Border),
            });
        }
        let color_clip = layers.last().map(|l| l.clip).unwrap_or_else(|| {
            area(
                style.get("background-clip").unwrap_or("border-box"),
                BoxArea::Border,
            )
        });
        Self { layers, color_clip }
    }
}
/// Token-aware comma splitting, retaining commas inside url()/quoted strings.
/// Malformed or oversized lists fail closed rather than allocating unboundedly.
pub fn split_layers(value: &str) -> Vec<&str> {
    if value.len() > MAX_VALUE_BYTES {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    for (index, ch) in value.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => {
                depth += 1;
                if depth > 32 {
                    return Vec::new();
                }
            }
            ')' => {
                if depth == 0 {
                    return Vec::new();
                }
                depth -= 1;
            }
            ',' if depth == 0 => {
                result.push(value[start..index].trim());
                start = index + 1;
                if result.len() >= MAX_BACKGROUND_LAYERS {
                    return Vec::new();
                }
            }
            _ => {}
        }
    }
    if quote.is_some() || depth != 0 || escape {
        return Vec::new();
    }
    result.push(value[start..].trim());
    result
}
fn url(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.get(..4)?.eq_ignore_ascii_case("url(") || !value.ends_with(')') {
        return None;
    }
    let mut body = value[4..value.len() - 1].trim();
    if body.starts_with(['\'', '"']) {
        let q = body.chars().next()?;
        if !body.ends_with(q) || body.len() < 2 {
            return None;
        }
        body = &body[1..body.len() - 1];
    }
    let mut result = String::new();
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        let first = chars.next()?;
        if first.is_ascii_hexdigit() {
            let mut digits = String::from(first);
            while digits.len() < 6 && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                digits.push(chars.next()?);
            }
            if chars.peek().is_some_and(|c| c.is_ascii_whitespace()) {
                chars.next();
            }
            let code = u32::from_str_radix(&digits, 16).ok()?;
            result.push(
                char::from_u32(code)
                    .filter(|c| *c != '\0')
                    .unwrap_or('\u{fffd}'),
            );
        } else if first != '\n' && first != '\r' {
            result.push(first);
        }
    }
    (!result.is_empty()).then_some(result)
}
/// Serialized `<gradient>` image function for a background layer. CSS owns
/// tokenization, color parsing and value preservation; this only recognizes the
/// function name and returns the bounded original text for the shared raster.
fn gradient(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() > MAX_VALUE_BYTES || !value.ends_with(')') {
        return None;
    }
    let name = value.get(..value.find('(')?)?.trim().to_ascii_lowercase();
    matches!(
        name.as_str(),
        "linear-gradient"
            | "repeating-linear-gradient"
            | "radial-gradient"
            | "repeating-radial-gradient"
            | "conic-gradient"
            | "repeating-conic-gradient"
    )
    .then(|| value.to_owned())
}
fn area(value: &str, default: BoxArea) -> BoxArea {
    match value.trim().to_ascii_lowercase().as_str() {
        "border-box" => BoxArea::Border,
        "padding-box" => BoxArea::Padding,
        "content-box" => BoxArea::Content,
        _ => default,
    }
}
fn repeat(value: &str) -> (Repeat, Repeat) {
    let parse = |s| match s {
        "no-repeat" => Repeat::NoRepeat,
        "space" => Repeat::Space,
        "round" => Repeat::Round,
        _ => Repeat::Repeat,
    };
    let value = value.to_ascii_lowercase();
    let v: Vec<_> = value.split_whitespace().take(3).collect();
    match v.as_slice() {
        ["repeat-x"] => (Repeat::Repeat, Repeat::NoRepeat),
        ["repeat-y"] => (Repeat::NoRepeat, Repeat::Repeat),
        [a, b] => (parse(a), parse(b)),
        [a] => (parse(a), parse(a)),
        _ => (Repeat::Repeat, Repeat::Repeat),
    }
}
fn dimension(value: &str) -> Option<SizeValue> {
    if value == "auto" {
        return Some(SizeValue::Auto);
    }
    let (raw, unit) = if let Some(n) = value.strip_suffix('%') {
        (n, 0)
    } else if let Some(n) = value.strip_suffix("rem") {
        (n, 3)
    } else if let Some(n) = value.strip_suffix("em") {
        (n, 2)
    } else if let Some(n) = value.strip_suffix("px") {
        (n, 1)
    } else if value == "0" {
        (value, 1)
    } else {
        return None;
    };
    let n: f32 = raw.parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    Some(match unit {
        0 => SizeValue::Percent(n / 100.0),
        2 => SizeValue::Em(n),
        3 => SizeValue::Rem(n),
        _ => SizeValue::Px(n),
    })
}
fn size(value: &str) -> BackgroundSize {
    let value = value.to_ascii_lowercase();
    let tokens: Vec<_> = value.split_whitespace().take(3).collect();
    match tokens.as_slice() {
        ["cover"] => BackgroundSize::Cover,
        ["contain"] => BackgroundSize::Contain,
        [a] => BackgroundSize::Explicit(dimension(a).unwrap_or(SizeValue::Auto), SizeValue::Auto),
        [a, b] => BackgroundSize::Explicit(
            dimension(a).unwrap_or(SizeValue::Auto),
            dimension(b).unwrap_or(SizeValue::Auto),
        ),
        _ => BackgroundSize::Explicit(SizeValue::Auto, SizeValue::Auto),
    }
}
