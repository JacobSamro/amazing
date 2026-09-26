//! CSS Filter Effects, Compositing and Blending, Masking, and Shapes.
//!
//! Typed computed values for `filter`/`backdrop-filter` (resolved function
//! lists), `mix-blend-mode`, `isolation`, `clip-path` basic shapes and
//! `path()`, and the `mask-*` longhands. Parsing follows Filter Effects 1
//! (function list), Compositing and Blending 2 (blend-mode), CSS Masking 1
//! (`<mask-reference>`, `<masking-mode>`, `<compositing-operator>`,
//! `<geometry-box>`), and CSS Shapes 1 (`<basic-shape>`).
use crate::syntax::{Token, tokenize};
use crate::values::{Angle, LengthPercentage};

#[derive(Debug, Clone, PartialEq)]
pub enum FilterList {
    None,
    Functions(Vec<FilterFunction>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropShadow {
    pub offset_x: LengthPercentage,
    pub offset_y: LengthPercentage,
    pub blur: Option<LengthPercentage>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilterFunction {
    Blur(LengthPercentage),
    Brightness(f64),
    Contrast(f64),
    Grayscale(f64),
    HueRotate(f64),
    Invert(f64),
    Opacity(f64),
    Saturate(f64),
    Sepia(f64),
    DropShadow(DropShadow),
    Url(String),
    /// A function whose argument is `calc()`: accepted and retained as text for
    /// the compositor, which evaluates the resolved expression.
    Expression { name: String, args: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    PlusLighter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskMode {
    MatchSource,
    Alpha,
    Luminance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskType {
    Luminance,
    Alpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskComposite {
    Add,
    Subtract,
    Intersect,
    Exclude,
}

/// `<geometry-box>` including `no-clip` (used by `mask-clip` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskBox {
    ContentBox,
    PaddingBox,
    BorderBox,
    MarginBox,
    FillBox,
    StrokeBox,
    ViewBox,
    NoClip,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipPath {
    None,
    Url(String),
    Geometry(MaskBox),
    Shape(BasicShape),
    ShapeWithBox(BasicShape, MaskBox),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    Nonzero,
    EvenOdd,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShapeRadius {
    ClosestSide,
    FarthestSide,
    Length(LengthPercentage),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BasicShape {
    Inset {
        offsets: Vec<LengthPercentage>,
        round: Option<Vec<LengthPercentage>>,
    },
    Circle {
        radius: Option<ShapeRadius>,
        position: Option<(LengthPercentage, LengthPercentage)>,
    },
    Ellipse {
        radii: Option<(ShapeRadius, ShapeRadius)>,
        position: Option<(LengthPercentage, LengthPercentage)>,
    },
    Polygon {
        fill_rule: Option<FillRule>,
        vertices: Vec<(LengthPercentage, LengthPercentage)>,
    },
    Path {
        fill_rule: Option<FillRule>,
        path: String,
    },
}

/// Split a token stream at top-level (depth-zero) whitespace or commas.
fn components(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0i32;
    for token in tokens {
        match token {
            Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1;
                current.push(token.clone());
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth -= 1;
                current.push(token.clone());
            }
            Token::Whitespace | Token::Comma if depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(token.clone()),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn function_parts(component: &[Token]) -> Option<(&str, &[Token])> {
    match component {
        [Token::Function(name), Token::OpenParen, rest @ ..]
            if matches!(rest.last(), Some(Token::CloseParen)) =>
        {
            Some((name.as_str(), &rest[..rest.len() - 1]))
        }
        _ => None,
    }
}

fn serialize(tokens: &[Token]) -> String {
    crate::syntax::serialize(tokens)
}

fn significant_str(css: &str) -> Vec<Token> {
    tokenize(css)
        .into_iter()
        .filter(|token| *token != Token::Whitespace)
        .collect()
}

fn as_ident(token: &Token) -> Option<String> {
    match token {
        Token::Ident(name) => Some(name.to_ascii_lowercase()),
        _ => None,
    }
}

/// `<number> | <percentage>` where a percentage is scaled by 1/100.
fn number_or_percentage(css: &str) -> Option<f64> {
    match significant_str(css).as_slice() {
        [Token::Number(n)] if n.is_finite() => Some(*n),
        [Token::Percentage(n)] if n.is_finite() => Some(n / 100.0),
        _ => None,
    }
}

fn nonnegative(value: f64) -> Option<f64> {
    (value.is_finite() && value >= 0.0).then_some(value)
}

pub fn parse_filter_list(css: &str) -> Option<FilterList> {
    let trimmed = css.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(FilterList::None);
    }
    let tokens = tokenize(trimmed);
    let mut functions = Vec::new();
    for component in components(&tokens) {
        functions.push(parse_filter_function(&component)?);
    }
    (!functions.is_empty()).then_some(FilterList::Functions(functions))
}

fn parse_filter_function(component: &[Token]) -> Option<FilterFunction> {
    if let [Token::Url { value, .. }] = component {
        return Some(FilterFunction::Url(value.clone()));
    }
    let (name, args) = function_parts(component)?;
    let name = name.to_ascii_lowercase();
    if name == "url" {
        let value = components(args)
            .first()
            .and_then(|part| match part.as_slice() {
                [Token::Str(value)] => Some(value.clone()),
                [Token::Ident(value)] => Some(value.clone()),
                _ => None,
            })?;
        return Some(FilterFunction::Url(value));
    }
    let argument = serialize(args);
    // `calc()` arguments are valid for every filter function and are retained
    // as text for the compositor to resolve.
    if significant_str(&argument)
        .iter()
        .any(|token| matches!(token, Token::Function(f) if f.eq_ignore_ascii_case("calc")))
    {
        return Some(FilterFunction::Expression {
            name,
            args: argument,
        });
    }
    Some(match name.as_str() {
        "blur" => FilterFunction::Blur(
            LengthPercentage::parse(&argument).filter(|l| l.is_nonnegative())?,
        ),
        "brightness" => FilterFunction::Brightness(nonnegative(number_or_percentage(&argument)?)?),
        "contrast" => FilterFunction::Contrast(nonnegative(number_or_percentage(&argument)?)?),
        "grayscale" => {
            FilterFunction::Grayscale(nonnegative(number_or_percentage(&argument)?)?.min(1.0))
        }
        "invert" => FilterFunction::Invert(nonnegative(number_or_percentage(&argument)?)?.min(1.0)),
        "opacity" => FilterFunction::Opacity(nonnegative(number_or_percentage(&argument)?)?.min(1.0)),
        "saturate" => FilterFunction::Saturate(nonnegative(number_or_percentage(&argument)?)?),
        "sepia" => FilterFunction::Sepia(nonnegative(number_or_percentage(&argument)?)?.min(1.0)),
        "hue-rotate" => FilterFunction::HueRotate(Angle::parse(&argument)?.radians),
        "drop-shadow" => FilterFunction::DropShadow(parse_drop_shadow(args)?),
        _ => return None,
    })
}

fn parse_drop_shadow(args: &[Token]) -> Option<DropShadow> {
    let components = components(args);
    let mut lengths = Vec::new();
    let mut color = None;
    for component in &components {
        if matches!(component.as_slice(), [Token::Str(_)] | [Token::Ident(_)]) {
            // Colour keywords/functions are not always single tokens; keep the
            // colour text and let the paint side parse it.
            let text = serialize(component);
            if crate::color::parse_css_color(&text).is_some() || is_color_ident(&text) {
                color = Some(text);
                continue;
            }
        }
        if let Some(length) = LengthPercentage::parse(&serialize(component)) {
            // drop-shadow offsets and blur are lengths, not percentages.
            if matches!(length, LengthPercentage::Percentage(_)) {
                return None;
            }
            lengths.push(length);
            continue;
        }
        // A colour function such as rgb(...) with a leading Function token.
        let text = serialize(component);
        if crate::color::parse_css_color(&text).is_some() {
            color = Some(text);
            continue;
        }
        return None;
    }
    if lengths.len() < 2 || lengths.len() > 3 {
        return None;
    }
    Some(DropShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur: lengths.get(2).copied(),
        color,
    })
}

fn is_color_ident(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.eq_ignore_ascii_case("transparent") || trimmed.eq_ignore_ascii_case("currentcolor")
}

pub fn parse_blend_mode(css: &str) -> Option<BlendMode> {
    Some(match css.trim().to_ascii_lowercase().as_str() {
        "normal" => BlendMode::Normal,
        "multiply" => BlendMode::Multiply,
        "screen" => BlendMode::Screen,
        "overlay" => BlendMode::Overlay,
        "darken" => BlendMode::Darken,
        "lighten" => BlendMode::Lighten,
        "color-dodge" => BlendMode::ColorDodge,
        "color-burn" => BlendMode::ColorBurn,
        "hard-light" => BlendMode::HardLight,
        "soft-light" => BlendMode::SoftLight,
        "difference" => BlendMode::Difference,
        "exclusion" => BlendMode::Exclusion,
        "hue" => BlendMode::Hue,
        "saturation" => BlendMode::Saturation,
        "color" => BlendMode::Color,
        "luminosity" => BlendMode::Luminosity,
        "plus-lighter" => BlendMode::PlusLighter,
        _ => return None,
    })
}

pub fn parse_isolation(css: &str) -> Option<bool> {
    match css.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(false),
        "isolate" => Some(true),
        _ => None,
    }
}

pub fn parse_mask_mode(css: &str) -> Option<MaskMode> {
    Some(match css.trim().to_ascii_lowercase().as_str() {
        "match-source" => MaskMode::MatchSource,
        "alpha" => MaskMode::Alpha,
        "luminance" => MaskMode::Luminance,
        _ => return None,
    })
}

pub fn parse_mask_type(css: &str) -> Option<MaskType> {
    match css.trim().to_ascii_lowercase().as_str() {
        "luminance" => Some(MaskType::Luminance),
        "alpha" => Some(MaskType::Alpha),
        _ => None,
    }
}

pub fn parse_mask_composite(css: &str) -> Option<MaskComposite> {
    Some(match css.trim().to_ascii_lowercase().as_str() {
        "add" => MaskComposite::Add,
        "subtract" => MaskComposite::Subtract,
        "intersect" => MaskComposite::Intersect,
        "exclude" => MaskComposite::Exclude,
        _ => return None,
    })
}

/// `<geometry-box>`, optionally accepting `no-clip` (mask-clip only).
pub fn parse_mask_box(css: &str, allow_no_clip: bool) -> Option<MaskBox> {
    Some(match css.trim().to_ascii_lowercase().as_str() {
        "content-box" => MaskBox::ContentBox,
        "padding-box" => MaskBox::PaddingBox,
        "border-box" => MaskBox::BorderBox,
        "margin-box" => MaskBox::MarginBox,
        "fill-box" => MaskBox::FillBox,
        "stroke-box" => MaskBox::StrokeBox,
        "view-box" => MaskBox::ViewBox,
        "no-clip" if allow_no_clip => MaskBox::NoClip,
        _ => return None,
    })
}

/// `<mask-reference>#`: `none` or a comma list of `url()`/`<image>` entries.
/// Image functions and gradients are kept as canonical text for the paint side.
pub fn parse_mask_image(css: &str) -> Option<Vec<String>> {
    let trimmed = css.trim();
    if trimmed.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for component in components(&tokenize(trimmed)) {
        let text = serialize(&component);
        if valid_image_component(&component) {
            out.push(text);
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

fn valid_image_component(component: &[Token]) -> bool {
    match component {
        [Token::Url { .. }] => true,
        [Token::Ident(name)] if name.eq_ignore_ascii_case("none") => true,
        [Token::Function(name), ..] => matches!(
            name.to_ascii_lowercase().as_str(),
            "url"
                | "linear-gradient"
                | "repeating-linear-gradient"
                | "radial-gradient"
                | "repeating-radial-gradient"
                | "conic-gradient"
                | "repeating-conic-gradient"
                | "image"
                | "image-set"
                | "element"
                | "cross-fade"
                | "paint"
        ),
        _ => false,
    }
}

/// Validate `mask-repeat`: one or more `<repeat-style>` components.
pub fn valid_repeat_list(css: &str) -> bool {
    let components = components(&tokenize(css));
    !components.is_empty()
        && components.iter().all(|component| {
            let Some(name) = single_ident(component) else { return false };
            matches!(
                name.as_str(),
                "repeat" | "repeat-x" | "repeat-y" | "space" | "round" | "no-repeat"
            )
        })
}

/// Validate `mask-position`: one or more `<position>` components.
pub fn valid_position_list(css: &str) -> bool {
    let components = if css.contains(',') {
        split_comma(&tokenize(css))
    } else {
        components(&tokenize(css))
    };
    !components.is_empty()
        && components
            .iter()
            .all(|component| parse_position(component).is_some())
}

/// Validate `mask-size`: one or more `[ <length-percentage> | auto ]{1,2} | cover | contain`.
pub fn valid_size_list(css: &str) -> bool {
    let components = if css.contains(',') {
        split_comma(&tokenize(css))
    } else {
        components(&tokenize(css))
    };
    !components.is_empty() && components.iter().all(|component| valid_size_component(component))
}

fn valid_size_component(component: &[Token]) -> bool {
    let parts = components(component);
    if parts.len() == 1 {
        if let Some(name) = single_ident(&parts[0]) {
            if matches!(name.as_str(), "cover" | "contain" | "auto") {
                return true;
            }
        }
    }
    if parts.is_empty() || parts.len() > 2 {
        return false;
    }
    parts.iter().all(|part| {
        single_ident(part).is_some_and(|name| name == "auto")
            || LengthPercentage::parse(&serialize(part)).is_some_and(|l| l.is_nonnegative())
    })
}

fn split_comma(tokens: &[Token]) -> Vec<Vec<Token>> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    let mut depth = 0i32;
    for token in tokens {
        match token {
            Token::OpenParen | Token::OpenSquare | Token::OpenCurly => {
                depth += 1;
                current.push(token.clone());
            }
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                depth -= 1;
                current.push(token.clone());
            }
            Token::Comma if depth == 0 => {
                out.push(std::mem::take(&mut current));
            }
            _ => current.push(token.clone()),
        }
    }
    out.push(current);
    out
}

fn single_ident(component: &[Token]) -> Option<String> {
    match component {
        [token] => as_ident(token),
        _ => None,
    }
}

pub fn parse_clip_path(css: &str) -> Option<ClipPath> {
    let tokens = tokenize(css.trim());
    let parts = split_comma(&tokens);
    // A clip-path is a single component; comma splitting only guards malformed input.
    if parts.len() != 1 {
        return None;
    }
    let groups = components(&tokens);
    if groups.is_empty() {
        return None;
    }
    if groups.len() == 1 {
        if let Some(name) = single_ident(&groups[0]) {
            return match name.as_str() {
                "none" => Some(ClipPath::None),
                "margin-box" | "border-box" | "padding-box" | "content-box" | "fill-box"
                | "stroke-box" | "view-box" => parse_mask_box(&name, false).map(ClipPath::Geometry),
                _ => None,
            };
        }
        if let [Token::Url { value, .. }] = groups[0].as_slice() {
            return Some(ClipPath::Url(value.clone()));
        }
        if let Some((name, args)) = function_parts(&groups[0]) {
            if name.eq_ignore_ascii_case("url") {
                let value = components(args).first().and_then(|part| match part.as_slice() {
                    [Token::Str(value)] | [Token::Ident(value)] => Some(value.clone()),
                    _ => None,
                })?;
                return Some(ClipPath::Url(value));
            }
        }
        let shape = parse_basic_shape(&groups[0])?;
        return Some(ClipPath::Shape(shape));
    }
    if groups.len() == 2 {
        // `<basic-shape> <geometry-box>` or `<geometry-box> <basic-shape>`.
        if let Some(shape) = parse_basic_shape(&groups[0]) {
            let name = single_ident(&groups[1])?;
            let geometry = parse_mask_box(&name, false)?;
            return Some(ClipPath::ShapeWithBox(shape, geometry));
        }
        let name = single_ident(&groups[0])?;
        let geometry = parse_mask_box(&name, false)?;
        let shape = parse_basic_shape(&groups[1])?;
        return Some(ClipPath::ShapeWithBox(shape, geometry));
    }
    None
}

fn parse_basic_shape(component: &[Token]) -> Option<BasicShape> {
    let (name, args) = function_parts(component)?;
    Some(match name.to_ascii_lowercase().as_str() {
        "inset" => parse_inset(args)?,
        "circle" => parse_circle(args)?,
        "ellipse" => parse_ellipse(args)?,
        "polygon" => parse_polygon(args)?,
        "path" => parse_path(args)?,
        _ => return None,
    })
}

fn parse_inset(args: &[Token]) -> Option<BasicShape> {
    let parts = components(args);
    let mut offsets = Vec::new();
    let mut round = None;
    let mut index = 0;
    while index < parts.len() {
        if single_ident(&parts[index]).is_some_and(|name| name == "round") {
            index += 1;
            let mut radii = Vec::new();
            while index < parts.len() {
                let length = LengthPercentage::parse(&serialize(&parts[index]))?;
                if !length.is_nonnegative() {
                    return None;
                }
                radii.push(length);
                index += 1;
            }
            if radii.is_empty() || radii.len() > 4 {
                return None;
            }
            round = Some(radii);
            break;
        }
        offsets.push(LengthPercentage::parse(&serialize(&parts[index]))?);
        index += 1;
    }
    if offsets.is_empty() || offsets.len() > 4 {
        return None;
    }
    Some(BasicShape::Inset { offsets, round })
}

fn parse_circle(args: &[Token]) -> Option<BasicShape> {
    let (radius_tokens, position_tokens) = split_at_ident(args, "at");
    let radius = match radius_tokens {
        Some(tokens) if !tokens.is_empty() => Some(parse_shape_radius(&tokens)?),
        _ => None,
    };
    let position = match position_tokens {
        Some(tokens) => Some(parse_position(&tokens)?),
        None => None,
    };
    Some(BasicShape::Circle { radius, position })
}

fn parse_ellipse(args: &[Token]) -> Option<BasicShape> {
    let (radii_tokens, position_tokens) = split_at_ident(args, "at");
    let radii = match radii_tokens {
        Some(tokens) if !tokens.is_empty() => {
            let parts = components(&tokens);
            if parts.len() != 2 {
                return None;
            }
            Some((parse_shape_radius(&parts[0])?, parse_shape_radius(&parts[1])?))
        }
        _ => None,
    };
    let position = match position_tokens {
        Some(tokens) => Some(parse_position(&tokens)?),
        None => None,
    };
    Some(BasicShape::Ellipse { radii, position })
}

fn parse_polygon(args: &[Token]) -> Option<BasicShape> {
    // `polygon( <fill-rule>? [round <length>]? , <length-percentage>{2}# )`
    let groups = split_comma(args);
    if groups.is_empty() {
        return None;
    }
    let mut index = 0;
    let mut fill_rule = None;
    if groups.len() >= 2 {
        let head = components(&groups[0]);
        let starts_declarator = head
            .first()
            .and_then(|part| single_ident(part))
            .is_some_and(|name| name == "round" || parse_fill_rule_opt(&name).is_some());
        if starts_declarator {
            let mut cursor = 0;
            if let Some(name) = head.get(cursor).and_then(|part| single_ident(part)) {
                if let Some(rule) = parse_fill_rule_opt(&name) {
                    fill_rule = Some(rule);
                    cursor += 1;
                }
            }
            if let Some(name) = head.get(cursor).and_then(|part| single_ident(part)) {
                if name == "round" {
                    cursor += 1;
                    let value = head.get(cursor)?;
                    if !contains_calc(value) {
                        let length = LengthPercentage::parse(&serialize(value))?;
                        if matches!(length, LengthPercentage::Percentage(_))
                            || !length.is_nonnegative()
                        {
                            return None;
                        }
                    }
                    cursor += 1;
                }
            }
            if cursor != head.len() {
                return None;
            }
            index = 1;
        }
    }
    let mut vertices = Vec::new();
    for group in &groups[index..] {
        let coordinates = significant_str(&serialize(group));
        if coordinates.len() != 2 {
            return None;
        }
        vertices.push((
            LengthPercentage::parse(&serialize(std::slice::from_ref(&coordinates[0])))?,
            LengthPercentage::parse(&serialize(std::slice::from_ref(&coordinates[1])))?,
        ));
    }
    if vertices.is_empty() {
        return None;
    }
    Some(BasicShape::Polygon { fill_rule, vertices })
}

fn contains_calc(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .any(|token| matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("calc")))
}

fn parse_path(args: &[Token]) -> Option<BasicShape> {
    let parts = components(args);
    let mut fill_rule = None;
    let mut path = None;
    for part in parts {
        match part.as_slice() {
            [Token::Str(value)] => {
                if path.is_some() || value.trim().is_empty() {
                    return None;
                }
                path = Some(value.clone());
            }
            _ => {
                // `<fill-rule>` must precede the path string.
                if path.is_some() {
                    return None;
                }
                let name = single_ident(&part)?;
                fill_rule = Some(parse_fill_rule(&name)?);
            }
        }
    }
    Some(BasicShape::Path {
        fill_rule,
        path: path?,
    })
}

fn parse_fill_rule(name: &str) -> Option<FillRule> {
    parse_fill_rule_opt(name)
}

fn parse_fill_rule_opt(name: &str) -> Option<FillRule> {
    match name {
        "nonzero" => Some(FillRule::Nonzero),
        "evenodd" => Some(FillRule::EvenOdd),
        _ => None,
    }
}

fn parse_shape_radius(component: &[Token]) -> Option<ShapeRadius> {
    if let Some(name) = single_ident(component) {
        return match name.as_str() {
            "closest-side" => Some(ShapeRadius::ClosestSide),
            "farthest-side" => Some(ShapeRadius::FarthestSide),
            _ => None,
        };
    }
    LengthPercentage::parse(&serialize(component))
        .filter(|l| l.is_nonnegative())
        .map(ShapeRadius::Length)
}

/// Split `tokens` into (before, after) around the first top-level ident `needle`.
fn split_at_ident(tokens: &[Token], needle: &str) -> (Option<Vec<Token>>, Option<Vec<Token>>) {
    let parts = components(tokens);
    if let Some(index) = parts.iter().position(|part| single_ident(part).is_some_and(|n| n == needle)) {
        (Some(join_components(&parts[..index])), Some(join_components(&parts[index + 1..])))
    } else {
        (Some(tokens.to_vec()), None)
    }
}

/// Re-join component groups with whitespace so re-tokenizing preserves separators.
fn join_components(parts: &[Vec<Token>]) -> Vec<Token> {
    let mut out = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            out.push(Token::Whitespace);
        }
        out.extend(part.iter().cloned());
    }
    out
}

fn parse_position(component: &[Token]) -> Option<(LengthPercentage, LengthPercentage)> {
    let parts = components(component);
    if parts.is_empty() || parts.len() > 2 {
        return None;
    }
    let value = |part: &[Token]| -> Option<LengthPercentage> {
        if let Some(name) = single_ident(part) {
            return match name.as_str() {
                "left" | "top" => Some(LengthPercentage::Percentage(0.0)),
                "center" => Some(LengthPercentage::Percentage(50.0)),
                "right" | "bottom" => Some(LengthPercentage::Percentage(100.0)),
                _ => None,
            };
        }
        LengthPercentage::parse(&serialize(part))
    };
    let first = value(&parts[0])?;
    if parts.len() == 1 {
        let is_vertical = single_ident(&parts[0])
            .is_some_and(|name| matches!(name.as_str(), "top" | "bottom"));
        return Some(if is_vertical {
            (LengthPercentage::Percentage(50.0), first)
        } else {
            (first, LengthPercentage::Percentage(50.0))
        });
    }
    let second = value(&parts[1])?;
    // A 2-value position may put the vertical keyword first (`top right`).
    let first_is_vertical = single_ident(&parts[0])
        .is_some_and(|name| matches!(name.as_str(), "top" | "bottom"));
    let second_is_horizontal = single_ident(&parts[1])
        .is_some_and(|name| matches!(name.as_str(), "left" | "right"));
    if first_is_vertical && second_is_horizontal {
        return Some((second, first));
    }
    Some((first, second))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter_ok(value: &str) -> bool {
        parse_filter_list(value).is_some()
    }
    fn clip_ok(value: &str) -> bool {
        parse_clip_path(value).is_some()
    }

    #[test]
    fn filter_valid_values_from_wpt_reach_the_parser() {
        // css/filter-effects/parsing/filter-parsing-valid.html
        for value in [
            "none",
            "blur(100px)",
            "blur(0)",
            "brightness(300%)",
            "contrast(0)",
            "drop-shadow(1px 2px)",
            "drop-shadow(1px 2px 3px)",
            "drop-shadow(rgb(4, 5, 6) 1px 2px)",
            "grayscale(300%)",
            "hue-rotate(90deg)",
            "hue-rotate(0)",
            "invert(0)",
            "opacity(2)",
            "saturate(300%)",
            "sepia(0)",
            "url(picture.svg#f)",
            "blur(10px) url(\"picture.svg#f\") contrast(20) brightness(30)",
            "blur(calc(100px))",
            "brightness(calc(3))",
        ] {
            assert!(filter_ok(value), "expected valid filter: {value}");
        }
    }

    #[test]
    fn filter_invalid_values_from_wpt_are_rejected() {
        // css/filter-effects/parsing/filter-parsing-invalid.html
        for value in [
            "auto",
            "none hue-rotate(0deg)",
            "blur(10)",
            "blur(-100px)",
            "brightness(-20)",
            "brightness(30px)",
            "contrast(-20)",
            "contrast(30px)",
            "drop-shadow(10 20)",
            "drop-shadow(10% 20%)",
            "drop-shadow(1px)",
            "drop-shadow(1px 2px 3px 4px)",
            "drop-shadow(rgb(4, 5, 6))",
            "drop-shadow()",
            "grayscale(-20)",
            "grayscale(30px)",
            "hue-rotate(90)",
            "invert(30px)",
            "opacity(30px)",
            "saturate(-20)",
            "sepia(30px)",
        ] {
            assert!(!filter_ok(value), "expected invalid filter: {value}");
        }
    }

    #[test]
    fn clip_path_valid_values_from_wpt_reach_the_parser() {
        // css/css-masking/parsing/clip-path-valid.html (shapes named by COMP-15).
        for value in [
            "none",
            "inset(100%)",
            "inset(0 1px)",
            "inset(0px 1px 2%)",
            "inset(0px 1px 2% 3em)",
            "inset(0px round 100%)",
            "inset(0px round 0 1px)",
            "inset(0px round 0px 1px 2% 3em)",
            "circle()",
            "circle(1px)",
            "circle(closest-side)",
            "circle(at 10% 20%)",
            "circle(farthest-side at center top)",
            "circle(4% at top right)",
            "ellipse()",
            "ellipse(1px closest-side)",
            "ellipse(at 10% 20%)",
            "ellipse(closest-side closest-side at 10% 20%)",
            "ellipse(farthest-side 4% at bottom left)",
            "polygon(1% 2%)",
            "polygon(nonzero, 1px 2px, 3em 4em)",
            "polygon(evenodd, 1px 2px, 3em 4em, 5pt 6%)",
            "polygon(round 1px, 1% 2%)",
            "polygon(round 0px, 1px 2px, 3px 4px)",
            "polygon(nonzero round 1px, 1px 2px, 3em 4em)",
            "polygon(evenodd round 1px, 1px 2px, 3em 4em, 5pt 6%)",
            "polygon(round calc(1em + 2px), 1px 2px, 3px 4px)",
            "path(\"m 20 0 h -100\")",
            "path(evenodd, \"M 20 20 h 60 v 60 h -60 Z\")",
            "border-box",
            "padding-box",
            "content-box",
            "margin-box",
            "fill-box",
            "stroke-box",
            "view-box",
            "circle(7% at 8% 9%) border-box",
            "border-box circle(7% at 8% 9%)",
            "url(https://example.com/)",
        ] {
            assert!(clip_ok(value), "expected valid clip-path: {value}");
        }
    }

    #[test]
    fn clip_path_invalid_values_from_wpt_are_rejected() {
        // css/css-masking/parsing/clip-path-invalid.html
        for value in [
            "auto",
            "ray(0deg)",
            "inset()",
            "inset(123)",
            "inset(1% 2% 3% 4% 5%)",
            "inset(round 0)",
            "inset(0px round)",
            "inset(0px round 123)",
            "inset(0px round 1% 2% 3% 4% 5%)",
            "inset(0px round / 1px)",
            "inset(10px round -20px)",
            "inset(30% round -40%)",
            "circle(123)",
            "circle(at)",
            "circle(10% 20%)",
            "circle(-10px at 20px 30px)",
            "circle(1% 2% at 0% 100%)",
            "ellipse(farthest-side at)",
            "ellipse(1% 2% top right)",
            "ellipse(3%)",
            "ellipse(closest-side)",
            "ellipse(10% -20% at 30% 40%)",
            "ellipse(-50px 60px at 70% 80%)",
            "polygon(1%)",
            "polygon(round, 1px 2px)",
            "polygon(round -1px, 1px 2px)",
            "polygon(round 1%, 1px 2px)",
            "polygon(round 45deg, 1px 2px)",
            "polygon(round 1px 1px 2px)",
            "polygon(1px 2px, round 1px)",
            "polygon(round 1px round 2px, 1px 2px)",
            "polygon(round 1px nonzero, 1px 2px)",
            "unknown-box",
            "path(\" \")",
            "path(evenodd, \"\")",
            "path(abc, \"m 20 0 h -100 z\")",
            "path(nonzero)",
            "path(\"m 20 0 h -100\", nonzero)",
        ] {
            assert!(!clip_ok(value), "expected invalid clip-path: {value}");
        }
    }

    #[test]
    fn blend_isolation_and_mask_grammar() {
        // css/compositing/parsing/mix-blend-mode-valid.html
        for value in [
            "normal",
            "multiply",
            "screen",
            "overlay",
            "darken",
            "lighten",
            "color-dodge",
            "color-burn",
            "hard-light",
            "soft-light",
            "difference",
            "exclusion",
            "hue",
            "saturation",
            "color",
            "luminosity",
        ] {
            assert!(parse_blend_mode(value).is_some(), "{value}");
        }
        assert!(parse_blend_mode("plus-lighter").is_some());
        assert!(parse_blend_mode("multiply-again").is_none());
        assert_eq!(parse_isolation("auto"), Some(false));
        assert_eq!(parse_isolation("isolate"), Some(true));
        assert!(parse_isolation("isolated").is_none());
        // css/css-masking/parsing/mask-image-computed.html
        assert!(parse_mask_image("none").is_some());
        assert!(parse_mask_image("url(\"http://example.com/\")").is_some());
        assert!(parse_mask_image("none, url(\"http://example.com/\")").is_some());
        assert!(parse_mask_image("linear-gradient(to left bottom, red, blue)").is_some());
        assert!(parse_mask_image("conic-gradient(from 45deg, red, blue)").is_some());
        assert!(parse_mask_image("20px").is_none());
    }
}

