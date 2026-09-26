//! Media Queries 4 evaluation shared by resource selection and future CSSOM.
//! Unknown media features conservatively do not match.
use crate::values::{LengthPercentage, RelativeUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaType { #[default] Screen, Print }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme { #[default] Light, Dark }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContrastPreference { #[default] NoPreference, More, Less, Custom }

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaEnvironment {
    pub viewport_width_px: f32,
    pub viewport_height_px: f32,
    pub device_pixel_ratio: f32,
    pub media_type: MediaType,
    pub prefers_color_scheme: ColorScheme,
    pub prefers_reduced_motion: bool,
    pub prefers_contrast: ContrastPreference,
    pub prefers_reduced_transparency: bool,
    pub prefers_reduced_data: bool,
    pub forced_colors: bool,
}
impl Default for MediaEnvironment {
    fn default() -> Self {
        Self { viewport_width_px: 1280.0, viewport_height_px: 720.0,
            device_pixel_ratio: 1.0, media_type: MediaType::Screen,
            prefers_color_scheme: ColorScheme::Light, prefers_reduced_motion: false,
            prefers_contrast: ContrastPreference::NoPreference,
            prefers_reduced_transparency: false, prefers_reduced_data: false, forced_colors: false }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaQueryList { queries: Vec<String> }
impl MediaQueryList {
    pub fn parse(css: &str) -> Self {
        Self { queries: split_top_level(css, ',').into_iter().map(|query| query.trim().to_owned()).collect() }
    }
    pub fn evaluate(&self, environment: &MediaEnvironment) -> bool {
        if self.queries.len() == 1 && self.queries[0].is_empty() { return true; }
        self.queries.iter().any(|query| !query.is_empty() && match_query(query, environment))
    }
}
/// Evaluate an already parsed query list against a rendering environment.
pub fn evaluate(list: &MediaQueryList, environment: &MediaEnvironment) -> bool {
    list.evaluate(environment)
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let (mut out, mut depth, mut start, mut quote, mut escape) = (Vec::new(), 0i32, 0usize, None, false);
    for (index, ch) in input.char_indices() {
        if let Some(q) = quote {
            if escape { escape = false; } else if ch == '\\' { escape = true; } else if ch == q { quote = None; }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if ch == delimiter && depth == 0 => { out.push(&input[start..index]); start = index + ch.len_utf8(); }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}
fn split_word<'a>(input: &'a str, word: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] { b'(' => depth += 1, b')' => depth -= 1, _ => {} }
        if depth == 0 && i + word.len() <= bytes.len()
            && input.is_char_boundary(i) && input.is_char_boundary(i + word.len())
            && input[i..i + word.len()].eq_ignore_ascii_case(word)
            && (i == 0 || bytes[i - 1].is_ascii_whitespace())
            && (i + word.len() == bytes.len() || bytes[i + word.len()].is_ascii_whitespace()) {
            out.push(input[start..i].trim());
            i += word.len(); start = i; continue;
        }
        i += 1;
    }
    out.push(input[start..].trim());
    out
}
fn match_query(input: &str, env: &MediaEnvironment) -> bool {
    let input = input.trim();
    if input.is_empty() { return false; }
    let input = input.strip_prefix("only ").unwrap_or(input).trim();
    // A leading `not` on a media query negates the complete query.
    if let Some(rest) = input.strip_prefix("not ") {
        if ["screen", "print", "all"].iter().any(|kind| rest.starts_with(kind)) {
            return !match_condition(rest, env);
        }
    }
    match_condition(input, env)
}
fn match_condition(input: &str, env: &MediaEnvironment) -> bool {
    let ors = split_word(input, "or");
    if ors.len() > 1 { return ors.iter().any(|part| match_condition(part, env)); }
    let ands = split_word(input, "and");
    if ands.len() > 1 { return ands.iter().all(|part| match_condition(part, env)); }
    let input = input.trim();
    if let Some(rest) = input.strip_prefix("not ") {
        if rest.starts_with('(') && rest.ends_with(')') && !known_feature(&rest[1..rest.len()-1]) {
            return false;
        }
        return !match_condition(rest, env);
    }
    if input.starts_with('(') && input.ends_with(')') { return match_feature(&input[1..input.len()-1], env); }
    match input.to_ascii_lowercase().as_str() {
        "all" => true, "screen" => env.media_type == MediaType::Screen,
        "print" => env.media_type == MediaType::Print, _ => false,
    }
}
fn known_feature(input: &str) -> bool {
    let name = input.split([':', '<', '>', '=']).next().unwrap_or("").trim();
    let name = name.strip_prefix("min-").or_else(|| name.strip_prefix("max-")).unwrap_or(name);
    matches!(name, "width" | "height" | "resolution" | "orientation" |
        "prefers-color-scheme" | "prefers-reduced-motion" | "prefers-contrast" |
        "prefers-reduced-transparency" | "prefers-reduced-data" | "forced-colors")
}
fn match_feature(input: &str, env: &MediaEnvironment) -> bool {
    let input = input.trim().to_ascii_lowercase();
    if let Some((name, value)) = input.split_once(':') {
        let (name, value) = (name.trim(), value.trim());
        let (relation, name) = if let Some(name) = name.strip_prefix("min-") { (1, name) }
            else if let Some(name) = name.strip_prefix("max-") { (-1, name) }
            else { (0, name) };
        return match name {
            "width" | "height" => source_size_value_px(value, env).is_some_and(|candidate| {
                let actual = if name == "width" { env.viewport_width_px } else { env.viewport_height_px };
                compare(actual, candidate, relation)
            }),
            "resolution" => resolution_dppx(value).is_some_and(|candidate| compare(env.device_pixel_ratio, candidate, relation)),
            "orientation" if relation == 0 => match value {
                "portrait" => env.viewport_height_px >= env.viewport_width_px,
                "landscape" => env.viewport_width_px > env.viewport_height_px, _ => false },
            "prefers-color-scheme" if relation == 0 => match value {
                "light" => env.prefers_color_scheme == ColorScheme::Light,
                "dark" => env.prefers_color_scheme == ColorScheme::Dark, _ => false },
            "prefers-reduced-motion" if relation == 0 => boolean_preference(value, env.prefers_reduced_motion),
            "prefers-reduced-transparency" if relation == 0 => boolean_preference(value, env.prefers_reduced_transparency),
            "prefers-reduced-data" if relation == 0 => boolean_preference(value, env.prefers_reduced_data),
            "forced-colors" if relation == 0 => match value { "active" => env.forced_colors, "none" => !env.forced_colors, _ => false },
            "prefers-contrast" if relation == 0 => match value {
                "no-preference" => env.prefers_contrast == ContrastPreference::NoPreference,
                "more" => env.prefers_contrast == ContrastPreference::More,
                "less" => env.prefers_contrast == ContrastPreference::Less,
                "custom" => env.prefers_contrast == ContrastPreference::Custom, _ => false },
            _ => false,
        };
    }
    for (operator, relation) in [("<=", -1), (">=", 1), ("<", -2), (">", 2), ("=", 0)] {
        if let Some((name, value)) = input.split_once(operator) {
            let (name, value) = (name.trim(), value.trim());
            return match name {
                "width" | "height" => source_size_value_px(value, env).is_some_and(|candidate| {
                    let actual = if name == "width" { env.viewport_width_px } else { env.viewport_height_px };
                    compare(actual, candidate, relation)
                }),
                "resolution" => resolution_dppx(value).is_some_and(|candidate| compare(env.device_pixel_ratio, candidate, relation)),
                _ => match value {
                    "width" | "height" => source_size_value_px(name, env).is_some_and(|candidate| {
                        let actual = if value == "width" { env.viewport_width_px } else { env.viewport_height_px };
                        compare(actual, candidate, -relation)
                    }),
                    "resolution" => resolution_dppx(name).is_some_and(|candidate|
                        compare(env.device_pixel_ratio, candidate, -relation)),
                    _ => false,
                },
            };
        }
    }
    match input.as_str() {
        "width" => env.viewport_width_px > 0.0, "height" => env.viewport_height_px > 0.0,
        "resolution" => env.device_pixel_ratio > 0.0,
        "prefers-reduced-motion" => env.prefers_reduced_motion,
        "prefers-reduced-transparency" => env.prefers_reduced_transparency,
        "prefers-reduced-data" => env.prefers_reduced_data,
        "forced-colors" => env.forced_colors,
        "prefers-contrast" => env.prefers_contrast != ContrastPreference::NoPreference,
        _ => false,
    }
}
fn compare(actual: f32, candidate: f32, relation: i8) -> bool {
    if !actual.is_finite() || !candidate.is_finite() { return false; }
    match relation { -2 => actual < candidate, -1 => actual <= candidate, 0 => (actual-candidate).abs() < 0.0001,
        1 => actual >= candidate, 2 => actual > candidate, _ => false }
}
fn boolean_preference(value: &str, enabled: bool) -> bool {
    match value { "reduce" => enabled, "no-preference" => !enabled, _ => false }
}
fn resolution_dppx(input: &str) -> Option<f32> {
    for (unit, scale) in [("dppx", 1.0), ("x", 1.0), ("dpi", 1.0/96.0), ("dpcm", 2.54/96.0)] {
        if let Some(number) = input.strip_suffix(unit) { return number.trim().parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n*scale); }
    }
    None
}
fn length_px(input: &str, env: &MediaEnvironment) -> Option<f32> {
    let length = LengthPercentage::parse(input)?;
    let value = match length {
        LengthPercentage::Px(n) => n,
        LengthPercentage::Percentage(_) => return None,
        LengthPercentage::Relative(n, unit) => n * match unit {
            RelativeUnit::Vw => env.viewport_width_px as f64 / 100.0,
            RelativeUnit::Vh => env.viewport_height_px as f64 / 100.0,
            RelativeUnit::Vmin => env.viewport_width_px.min(env.viewport_height_px) as f64 / 100.0,
            RelativeUnit::Vmax => env.viewport_width_px.max(env.viewport_height_px) as f64 / 100.0,
            RelativeUnit::Rem | RelativeUnit::Em => 16.0,
            RelativeUnit::Ex | RelativeUnit::Ch | RelativeUnit::Ic | RelativeUnit::Cap |
            RelativeUnit::Lh | RelativeUnit::Rch | RelativeUnit::Ric | RelativeUnit::Rex |
            RelativeUnit::Rcap | RelativeUnit::Rlh => return None,
        },
    };
    value.is_finite().then_some(value as f32)
}

fn source_size_value_px(input: &str, env: &MediaEnvironment) -> Option<f32> {
    let input = input.trim();
    if let Some(value) = length_px(input, env) { return Some(value); }
    for name in ["calc", "min", "max", "clamp"] {
        if let Some(inner) = input.strip_prefix(name).and_then(|rest| rest.strip_prefix('(')).and_then(|rest| rest.strip_suffix(')')) {
            if name == "calc" { return math_sum(inner, env); }
            let values: Vec<_> = split_top_level(inner, ',').into_iter().map(|part| math_sum(part, env)).collect::<Option<_>>()?;
            return match (name, values.as_slice()) {
                ("min", [first, rest @ ..]) if !rest.is_empty() => Some(rest.iter().copied().fold(*first, f32::min)),
                ("max", [first, rest @ ..]) if !rest.is_empty() => Some(rest.iter().copied().fold(*first, f32::max)),
                ("clamp", [low, preferred, high]) => Some(preferred.max(*low).min(*high)),
                _ => None,
            };
        }
    }
    None
}
fn math_sum(input: &str, env: &MediaEnvironment) -> Option<f32> {
    let input = input.trim();
    let mut depth = 0i32;
    for (index, ch) in input.char_indices().rev() {
        match ch { ')' => depth += 1, '(' => depth -= 1,
            '+' | '-' if depth == 0 && index > 0 => {
                let (left, right) = input.split_at(index);
                if left.ends_with(char::is_whitespace) && right[1..].starts_with(char::is_whitespace) {
                    let (a, b) = (math_sum(left, env)?, math_product(&right[1..], env)?);
                    return Some(if ch == '+' { a + b } else { a - b });
                }
            }, _ => {} }
    }
    math_product(input, env)
}
fn math_product(input: &str, env: &MediaEnvironment) -> Option<f32> {
    let input = input.trim();
    let mut depth = 0i32;
    for (index, ch) in input.char_indices().rev() {
        match ch { ')' => depth += 1, '(' => depth -= 1,
            '*' | '/' if depth == 0 && index > 0 => {
                let (left, right) = input.split_at(index);
                let number = right[1..].trim().parse::<f32>().ok()?;
                if !number.is_finite() || (ch == '/' && number == 0.0) { return None; }
                let value = math_product(left, env)?;
                return Some(if ch == '*' { value * number } else { value / number });
            }, _ => {} }
    }
    if input.starts_with('(') && input.ends_with(')') { return math_sum(&input[1..input.len()-1], env); }
    source_size_value_px(input, env).filter(|value| value.is_finite())
}

/// Resolve HTML `sizes` to a source size in CSS pixels. `auto_width` is the
/// rendered width when HTML permits `sizes="auto"`; otherwise the fallback is
/// 100vw. Invalid entries are skipped, as in the HTML source-size algorithm.
pub fn parse_sizes_attribute(sizes: &str, env: &MediaEnvironment, auto_width: Option<f32>) -> f32 {
    for entry in split_top_level(sizes, ',') {
        let entry = entry.trim();
        if entry.eq_ignore_ascii_case("auto") {
            if let Some(width) = auto_width.filter(|n| n.is_finite() && *n >= 0.0) { return width; }
            continue;
        }
        let mut depth = 0i32;
        let mut boundary = None;
        for (index, ch) in entry.char_indices() {
            match ch { '(' => depth += 1, ')' => depth -= 1, _ if ch.is_whitespace() && depth == 0 => boundary = Some(index), _ => {} }
        }
        let (condition, size) = if let Some(index) = boundary {
            (&entry[..index], entry[index..].trim())
        } else { ("", entry) };
        if !condition.is_empty() && !match_condition(condition, env) { continue; }
        if let Some(width) = source_size_value_px(size, env).filter(|n| *n >= 0.0) { return width; }
    }
    env.viewport_width_px
}
