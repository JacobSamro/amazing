//! Specified-to-computed value processing.
//!
//! This is the deep style-value module: cascade supplies one winning string
//! per property, and callers receive a [`ComputedStyle`]. Custom-property
//! inheritance, dependency cycles, `var()` substitution, CSS-wide defaulting,
//! and supported shorthand expansion stay behind this seam.

use crate::syntax::{serialize, tokenize, Token};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq)]
enum CustomValue {
    Valid(Vec<Token>),
    Invalid,
}

/// Final computed properties for one element. Normal property names are
/// ASCII-lowercase. Custom properties remain private implementation state and
/// preserve their case-sensitive names for inheritance and `var()` lookup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComputedStyle {
    props: BTreeMap<String, String>,
    custom: BTreeMap<String, CustomValue>,
}

impl ComputedStyle {
    pub fn get(&self, property: &str) -> Option<&str> {
        self.props.get(property).map(String::as_str)
    }

    /// Sorted normal (property, value) pairs. Custom properties are omitted:
    /// they are substitution state, not values consumed by layout or paint.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Public mutation for layout-time synthesis. Layout only materializes
    /// normal used-value hints, so custom properties intentionally stay behind
    /// the computed-value seam.
    pub fn set(&mut self, property: &str, value: &str) {
        if property.starts_with("--") {
            return;
        }
        for (longhand, expanded) in expand_declaration(property, value) {
            self.props.insert(longhand, expanded);
        }
    }

    #[cfg(test)]
    pub(crate) fn custom_serialized(&self, name: &str) -> Option<Option<String>> {
        self.custom.get(name).map(|value| match value {
            CustomValue::Valid(tokens) => Some(serialize_computed(tokens)),
            CustomValue::Invalid => None,
        })
    }
}

impl FromIterator<(String, String)> for ComputedStyle {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        let mut style = Self::default();
        for (name, value) in iter {
            if name.starts_with("--") {
                style
                    .custom
                    .insert(name, CustomValue::Valid(tokenize(&value)));
            } else {
                style.props.insert(name.to_ascii_lowercase(), value);
            }
        }
        style
    }
}

#[derive(Clone, Copy)]
struct PropertyMetadata {
    name: &'static str,
    initial: &'static str,
    inherited: bool,
}

// Every normal property for which the engine currently has cascade, layout,
// or paint behavior. Keeping initial/inheritance metadata in one registry is
// what makes CSS-wide keywords and `all` property-aware.
const PROPERTIES: &[PropertyMetadata] = &[
    PropertyMetadata { name: "align-content", initial: "normal", inherited: false },
    PropertyMetadata { name: "align-items", initial: "normal", inherited: false },
    PropertyMetadata { name: "align-self", initial: "auto", inherited: false },
    PropertyMetadata { name: "background-color", initial: "transparent", inherited: false },
    PropertyMetadata { name: "border-bottom-color", initial: "currentcolor", inherited: false },
    PropertyMetadata { name: "border-bottom-style", initial: "none", inherited: false },
    PropertyMetadata { name: "border-bottom-width", initial: "medium", inherited: false },
    PropertyMetadata { name: "border-collapse", initial: "separate", inherited: true },
    PropertyMetadata { name: "border-left-color", initial: "currentcolor", inherited: false },
    PropertyMetadata { name: "border-left-style", initial: "none", inherited: false },
    PropertyMetadata { name: "border-left-width", initial: "medium", inherited: false },
    PropertyMetadata { name: "border-right-color", initial: "currentcolor", inherited: false },
    PropertyMetadata { name: "border-right-style", initial: "none", inherited: false },
    PropertyMetadata { name: "border-right-width", initial: "medium", inherited: false },
    PropertyMetadata { name: "border-top-color", initial: "currentcolor", inherited: false },
    PropertyMetadata { name: "border-top-style", initial: "none", inherited: false },
    PropertyMetadata { name: "border-top-width", initial: "medium", inherited: false },
    PropertyMetadata { name: "bottom", initial: "auto", inherited: false },
    PropertyMetadata { name: "box-sizing", initial: "content-box", inherited: false },
    PropertyMetadata { name: "caption-side", initial: "top", inherited: true },
    PropertyMetadata { name: "clear", initial: "none", inherited: false },
    PropertyMetadata { name: "color", initial: "black", inherited: true },
    PropertyMetadata { name: "column-gap", initial: "normal", inherited: false },
    PropertyMetadata { name: "cursor", initial: "auto", inherited: true },
    PropertyMetadata { name: "direction", initial: "ltr", inherited: true },
    PropertyMetadata { name: "display", initial: "inline", inherited: false },
    PropertyMetadata { name: "empty-cells", initial: "show", inherited: true },
    PropertyMetadata { name: "flex-basis", initial: "auto", inherited: false },
    PropertyMetadata { name: "flex-direction", initial: "row", inherited: false },
    PropertyMetadata { name: "flex-grow", initial: "0", inherited: false },
    PropertyMetadata { name: "flex-shrink", initial: "1", inherited: false },
    PropertyMetadata { name: "flex-wrap", initial: "nowrap", inherited: false },
    PropertyMetadata { name: "float", initial: "none", inherited: false },
    PropertyMetadata { name: "font-family", initial: "serif", inherited: true },
    PropertyMetadata { name: "font-size", initial: "medium", inherited: true },
    PropertyMetadata { name: "font-stretch", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-style", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-weight", initial: "normal", inherited: true },
    PropertyMetadata { name: "grid-auto-columns", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-auto-flow", initial: "row", inherited: false },
    PropertyMetadata { name: "grid-auto-rows", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-column-end", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-column-start", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-row-end", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-row-start", initial: "auto", inherited: false },
    PropertyMetadata { name: "grid-template-columns", initial: "none", inherited: false },
    PropertyMetadata { name: "grid-template-rows", initial: "none", inherited: false },
    PropertyMetadata { name: "height", initial: "auto", inherited: false },
    PropertyMetadata { name: "hyphens", initial: "manual", inherited: true },
    PropertyMetadata { name: "justify-content", initial: "normal", inherited: false },
    PropertyMetadata { name: "justify-items", initial: "legacy", inherited: false },
    PropertyMetadata { name: "justify-self", initial: "auto", inherited: false },
    PropertyMetadata { name: "letter-spacing", initial: "normal", inherited: true },
    PropertyMetadata { name: "left", initial: "auto", inherited: false },
    PropertyMetadata { name: "line-break", initial: "auto", inherited: true },
    PropertyMetadata { name: "line-height", initial: "normal", inherited: true },
    PropertyMetadata { name: "list-style-image", initial: "none", inherited: true },
    PropertyMetadata { name: "list-style-position", initial: "outside", inherited: true },
    PropertyMetadata { name: "list-style-type", initial: "disc", inherited: true },
    PropertyMetadata { name: "margin-bottom", initial: "0", inherited: false },
    PropertyMetadata { name: "margin-left", initial: "0", inherited: false },
    PropertyMetadata { name: "margin-right", initial: "0", inherited: false },
    PropertyMetadata { name: "margin-top", initial: "0", inherited: false },
    PropertyMetadata { name: "max-height", initial: "none", inherited: false },
    PropertyMetadata { name: "max-width", initial: "none", inherited: false },
    PropertyMetadata { name: "min-height", initial: "0", inherited: false },
    PropertyMetadata { name: "min-width", initial: "0", inherited: false },
    PropertyMetadata { name: "opacity", initial: "1", inherited: false },
    PropertyMetadata { name: "order", initial: "0", inherited: false },
    PropertyMetadata { name: "orphans", initial: "2", inherited: true },
    PropertyMetadata { name: "overflow", initial: "visible", inherited: false },
    PropertyMetadata { name: "overflow-wrap", initial: "normal", inherited: true },
    PropertyMetadata { name: "padding-bottom", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-left", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-right", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-top", initial: "0", inherited: false },
    PropertyMetadata { name: "position", initial: "static", inherited: false },
    PropertyMetadata { name: "quotes", initial: "auto", inherited: true },
    PropertyMetadata { name: "right", initial: "auto", inherited: false },
    PropertyMetadata { name: "row-gap", initial: "normal", inherited: false },
    PropertyMetadata { name: "text-align", initial: "start", inherited: true },
    PropertyMetadata { name: "text-decoration", initial: "none", inherited: false },
    PropertyMetadata { name: "text-indent", initial: "0", inherited: true },
    PropertyMetadata { name: "text-transform", initial: "none", inherited: true },
    PropertyMetadata { name: "top", initial: "auto", inherited: false },
    PropertyMetadata { name: "unicode-bidi", initial: "normal", inherited: false },
    PropertyMetadata { name: "vertical-align", initial: "baseline", inherited: false },
    PropertyMetadata { name: "visibility", initial: "visible", inherited: true },
    PropertyMetadata { name: "white-space", initial: "normal", inherited: true },
    PropertyMetadata { name: "widows", initial: "2", inherited: true },
    PropertyMetadata { name: "width", initial: "auto", inherited: false },
    PropertyMetadata { name: "word-break", initial: "normal", inherited: true },
    PropertyMetadata { name: "word-spacing", initial: "normal", inherited: true },
    PropertyMetadata { name: "z-index", initial: "auto", inherited: false },
];

fn metadata(name: &str) -> Option<PropertyMetadata> {
    PROPERTIES.iter().copied().find(|property| property.name == name)
}

pub(crate) fn is_supported_property(name: &str) -> bool {
    metadata(&name.to_ascii_lowercase()).is_some()
}

/// Expands shorthands before cascade ranking so each resulting longhand keeps
/// the source declaration's origin, importance, specificity, and order.
pub(crate) fn expand_declaration(property: &str, value: &str) -> Vec<(String, String)> {
    const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
    let property = if property.starts_with("--") {
        property.to_owned()
    } else {
        property.to_ascii_lowercase()
    };
    let parts: Vec<&str> = value.split_ascii_whitespace().collect();
    match property.as_str() {
        "all" if css_wide_keyword(value).is_some() => PROPERTIES
            .iter()
            .filter(|property| !matches!(property.name, "direction" | "unicode-bidi"))
            .map(|property| (property.name.to_owned(), value.to_owned()))
            .collect(),
        "margin" | "padding" | "border-width" | "border-style" | "border-color" => {
            if parts.is_empty() {
                return vec![(property, value.to_owned())];
            }
            SIDES
                .iter()
                .enumerate()
                .map(|(index, side)| {
                    let part = parts[replicate_index(parts.len(), index)];
                    (longhand_name(&property, side), part.to_owned())
                })
                .collect()
        },
        "inset" if !parts.is_empty() => SIDES
            .iter()
            .enumerate()
            .map(|(index, side)| {
                (
                    (*side).to_owned(),
                    parts[replicate_index(parts.len(), index)].to_owned(),
                )
            })
            .collect(),
        "border" if !parts.is_empty() => {
            let mut width = "medium";
            let mut style = "none";
            let mut color = "currentcolor";
            for part in parts {
                if is_border_style(part) {
                    style = part;
                } else if is_border_width(part) {
                    width = part;
                } else {
                    color = part;
                }
            }
            let mut expanded = Vec::with_capacity(12);
            for side in SIDES {
                expanded.push((format!("border-{side}-width"), width.to_owned()));
                expanded.push((format!("border-{side}-style"), style.to_owned()));
                expanded.push((format!("border-{side}-color"), color.to_owned()));
            }
            expanded
        },
        "background" if parts.len() == 1 => vec![(
            "background-color".to_owned(),
            if parts[0].eq_ignore_ascii_case("none") {
                "transparent".to_owned()
            } else {
                parts[0].to_owned()
            },
        )],
        "font" => expand_font_shorthand(value).unwrap_or_else(|| vec![(property, value.to_owned())]),
        "gap" if !parts.is_empty() => {
            let row = parts[0];
            let column = parts.get(1).copied().unwrap_or(row);
            vec![
                ("row-gap".to_owned(), row.to_owned()),
                ("column-gap".to_owned(), column.to_owned()),
            ]
        },
        "grid-column" | "grid-row" if !parts.is_empty() => {
            let prefix = property.as_str();
            let (start, end) = value
                .split_once('/')
                .map(|(start, end)| (start.trim(), end.trim()))
                .unwrap_or((value.trim(), "auto"));
            vec![
                (format!("{prefix}-start"), start.to_owned()),
                (format!("{prefix}-end"), end.to_owned()),
            ]
        },
        "flex" if !parts.is_empty() => {
            let (grow, shrink, basis) = if css_wide_keyword(value).is_some() {
                (value, value, value)
            } else if parts.len() == 1 && parts[0].eq_ignore_ascii_case("none") {
                ("0", "0", "auto")
            } else if parts.len() == 1 && parts[0].eq_ignore_ascii_case("auto") {
                ("1", "1", "auto")
            } else if parts.len() == 1 && parts[0].parse::<f32>().is_ok() {
                (parts[0], "1", "0%")
            } else if parts.len() == 1 {
                ("1", "1", parts[0])
            } else if parts.len() == 2 && parts[1].parse::<f32>().is_ok() {
                (parts[0], parts[1], "0%")
            } else if parts.len() == 2 {
                (parts[0], "1", parts[1])
            } else {
                (parts[0], parts[1], parts[2])
            };
            vec![
                ("flex-grow".to_owned(), grow.to_owned()),
                ("flex-shrink".to_owned(), shrink.to_owned()),
                ("flex-basis".to_owned(), basis.to_owned()),
            ]
        },
        _ => vec![(property, value.to_owned())],
    }
}

fn expand_font_shorthand(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() {
        return Some(
            [
                "font-family",
                "font-size",
                "font-stretch",
                "font-style",
                "font-variant",
                "font-weight",
                "line-height",
            ]
            .into_iter()
            .map(|property| (property.to_owned(), value.to_owned()))
            .collect(),
        );
    }
    let tokens = tokenize(value);
    let mut position = 0usize;
    let mut style = "normal".to_owned();
    let mut variant = "normal".to_owned();
    let mut weight = "normal".to_owned();
    let mut stretch = "normal".to_owned();
    let mut style_seen = false;
    let mut variant_seen = false;
    let mut weight_seen = false;
    let mut stretch_seen = false;
    let mut optional_count = 0usize;
    let size = loop {
        skip_css_whitespace(&tokens, &mut position);
        let token = tokens.get(position)?;
        if is_font_size_token(token) {
            let size = serialize(std::slice::from_ref(token));
            position += 1;
            break size;
        }
        optional_count += 1;
        if optional_count > 4 {
            return None;
        }
        match token {
            Token::Ident(value) if value.eq_ignore_ascii_case("normal") => {},
            Token::Ident(value)
                if value.eq_ignore_ascii_case("italic")
                    || value.eq_ignore_ascii_case("oblique") =>
            {
                if style_seen {
                    return None;
                }
                style_seen = true;
                style = value.clone();
            },
            Token::Ident(value) if value.eq_ignore_ascii_case("small-caps") => {
                if variant_seen {
                    return None;
                }
                variant_seen = true;
                variant = value.clone();
            },
            Token::Ident(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "bold" | "bolder" | "lighter"
                ) =>
            {
                if weight_seen {
                    return None;
                }
                weight_seen = true;
                weight = value.clone();
            },
            Token::Number(value) if (1.0..=1000.0).contains(value) && value.fract() == 0.0 => {
                if weight_seen {
                    return None;
                }
                weight_seen = true;
                weight = serialize(std::slice::from_ref(token));
            },
            Token::Ident(value)
                if value.to_ascii_lowercase().ends_with("condensed")
                    || value.to_ascii_lowercase().ends_with("expanded") =>
            {
                if stretch_seen {
                    return None;
                }
                stretch_seen = true;
                stretch = value.clone();
            },
            _ => return None,
        }
        position += 1;
    };
    skip_css_whitespace(&tokens, &mut position);
    let line_height = if tokens.get(position) == Some(&Token::Delim('/')) {
        position += 1;
        skip_css_whitespace(&tokens, &mut position);
        let token = tokens.get(position)?;
        if !is_line_height_token(token) {
            return None;
        }
        position += 1;
        serialize(std::slice::from_ref(token))
    } else {
        "normal".to_owned()
    };
    skip_css_whitespace(&tokens, &mut position);
    let family_tokens = tokens.get(position..)?;
    if !valid_font_family_tokens(family_tokens) {
        return None;
    }
    let family = serialize(family_tokens);
    Some(vec![
        ("font-style".into(), style),
        ("font-variant".into(), variant),
        ("font-weight".into(), weight),
        ("font-stretch".into(), stretch),
        ("font-size".into(), size),
        ("line-height".into(), line_height),
        ("font-family".into(), family),
    ])
}

fn skip_css_whitespace(tokens: &[Token], position: &mut usize) {
    while matches!(tokens.get(*position), Some(Token::Whitespace)) {
        *position += 1;
    }
}

fn is_font_size_token(token: &Token) -> bool {
    matches!(token, Token::Dimension(value, _) | Token::Percentage(value) if *value >= 0.0)
        || matches!(token, Token::Number(value) if *value == 0.0)
        || matches!(token, Token::Ident(value) if matches!(
            value.to_ascii_lowercase().as_str(),
            "xx-small" | "x-small" | "small" | "medium" | "large" | "x-large" | "xx-large" | "xxx-large" | "smaller" | "larger"
        ))
}

fn is_line_height_token(token: &Token) -> bool {
    matches!(token, Token::Dimension(value, _) | Token::Percentage(value) | Token::Number(value) if *value >= 0.0)
        || matches!(token, Token::Ident(value) if value.eq_ignore_ascii_case("normal"))
}

fn valid_font_family_tokens(tokens: &[Token]) -> bool {
    #[derive(Clone, Copy)]
    enum FamilyKind {
        None,
        String,
        Ident,
    }
    let mut kind = FamilyKind::None;
    for token in tokens {
        match token {
            Token::Whitespace => {},
            Token::Comma if !matches!(kind, FamilyKind::None) => {
                kind = FamilyKind::None;
            },
            Token::Ident(_) if !matches!(kind, FamilyKind::String) => kind = FamilyKind::Ident,
            Token::Str(_) if matches!(kind, FamilyKind::None) => kind = FamilyKind::String,
            _ => return false,
        }
    }
    !matches!(kind, FamilyKind::None)
}

fn replicate_index(count: usize, side: usize) -> usize {
    match count {
        0 | 1 => 0,
        2 => side % 2,
        3 if side == 3 => 1,
        3 => side.min(2),
        _ => side.min(3),
    }
}

fn longhand_name(shorthand: &str, side: &str) -> String {
    match shorthand.rsplit_once('-') {
        Some((head, tail)) => format!("{head}-{side}-{tail}"),
        None => format!("{shorthand}-{side}"),
    }
}

fn is_border_style(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

fn is_border_width(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    ) || matches!(
        value.chars().next(),
        Some('0'..='9') | Some('.') | Some('-') | Some('+')
    )
}

#[derive(Clone, Copy)]
enum CssWide {
    Initial,
    Inherit,
    Unset,
    Revert,
    RevertLayer,
}

fn css_wide_keyword(value: &str) -> Option<CssWide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "initial" => Some(CssWide::Initial),
        "inherit" => Some(CssWide::Inherit),
        "unset" => Some(CssWide::Unset),
        "revert" => Some(CssWide::Revert),
        "revert-layer" => Some(CssWide::RevertLayer),
        _ => None,
    }
}

/// Turns one cascaded winner per property into computed values.
#[cfg(test)]
pub(crate) fn resolve(
    parent: Option<&ComputedStyle>,
    cascaded: BTreeMap<String, String>,
) -> ComputedStyle {
    resolve_with_root(parent, cascaded, 16.0)
}

pub(crate) fn resolve_with_root(
    parent: Option<&ComputedStyle>,
    mut cascaded: BTreeMap<String, String>,
    root_font_px: f32,
) -> ComputedStyle {
    let mut inherited_custom = parent
        .map(|style| style.custom.clone())
        .unwrap_or_default();
    let custom_names: Vec<String> = cascaded
        .keys()
        .filter(|name| name.starts_with("--"))
        .cloned()
        .collect();
    let mut local_raw = BTreeMap::new();
    for name in custom_names {
        let value = cascaded.remove(&name).expect("collected custom property");
        match css_wide_keyword(&value) {
            Some(CssWide::Initial) => {
                inherited_custom.insert(name, CustomValue::Invalid);
            },
            Some(CssWide::Inherit | CssWide::Unset) => {
                let inherited = parent
                    .and_then(|style| style.custom.get(&name))
                    .cloned()
                    .unwrap_or(CustomValue::Invalid);
                inherited_custom.insert(name, inherited);
            },
            _ => {
                local_raw.insert(name, tokenize(&value));
            },
        }
    }
    let cycles = cycle_members(&local_raw);
    let mut resolver = CustomResolver {
        local: &local_raw,
        inherited: &inherited_custom,
        cycles: &cycles,
        memo: BTreeMap::new(),
        resolving: BTreeSet::new(),
    };
    let mut resolved_local = Vec::with_capacity(local_raw.len());
    for name in local_raw.keys() {
        let value = resolver.resolve_name(name);
        resolved_local.push((name.clone(), value));
    }

    let mut properties = BTreeMap::new();
    for property in PROPERTIES {
        let parent_value = parent.and_then(|style| style.get(property.name));
        let value = match cascaded.remove(property.name) {
            None => default_value(*property, parent_value),
            Some(specified) => {
                compute_normal_value(
                    *property,
                    &specified,
                    parent_value,
                    root_font_px,
                    &mut resolver,
                )
            },
        };
        properties.insert(property.name.to_owned(), value);
    }
    // Preserve declared unknown properties for diagnostics and forward
    // compatibility. They still receive variable substitution, but without
    // metadata an invalid or CSS-wide value cannot manufacture an initial.
    for (name, specified) in cascaded {
        let tokens = tokenize(&specified);
        if let Some(substituted) = resolver.substitute(&tokens) {
            let value = serialize_computed(&substituted);
            if css_wide_keyword(&value).is_none() {
                properties.insert(name, value);
            }
        }
    }
    drop(resolver);
    inherited_custom.extend(resolved_local);
    ComputedStyle { props: properties, custom: inherited_custom }
}

fn default_value(property: PropertyMetadata, parent: Option<&str>) -> String {
    if property.inherited {
        parent.unwrap_or(property.initial).to_owned()
    } else {
        property.initial.to_owned()
    }
}

fn compute_normal_value(
    property: PropertyMetadata,
    specified: &str,
    parent: Option<&str>,
    root_font_px: f32,
    resolver: &mut CustomResolver<'_>,
) -> String {
    let Some(tokens) = resolver.substitute(&tokenize(specified)) else {
        // Invalid at computed-value time behaves as `unset`.
        return default_value(property, parent);
    };
    if trim_tokens(&tokens).is_empty() {
        return default_value(property, parent);
    }
    let value = serialize_computed(&tokens);
    let computed = match css_wide_keyword(&value) {
        Some(CssWide::Initial) => property.initial.to_owned(),
        Some(CssWide::Inherit) => parent.unwrap_or(property.initial).to_owned(),
        Some(CssWide::Unset) => default_value(property, parent),
        // Origin/layer rollback needs retained lower cascade candidates and is
        // deliberately not fabricated at this stage.
        Some(CssWide::Revert | CssWide::RevertLayer) | None => value,
    };
    if property.name == "font-size" {
        computed_font_size(&computed, parent, root_font_px).unwrap_or(computed)
    } else {
        computed
    }
}

fn computed_font_size(value: &str, parent: Option<&str>, root_font_px: f32) -> Option<String> {
    let parent_px = parent.and_then(font_size_px).unwrap_or(16.0);
    let lower = value.trim().to_ascii_lowercase();
    let px = match lower.as_str() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        "larger" => relative_font_size(parent_px, true),
        "smaller" => relative_font_size(parent_px, false),
        _ if lower.ends_with('%') => {
            parent_px * lower[..lower.len() - 1].parse::<f32>().ok()? / 100.0
        },
        _ if lower.ends_with("rem") => {
            root_font_px * lower[..lower.len() - 3].parse::<f32>().ok()?
        },
        _ if lower.ends_with("em") => {
            parent_px * lower[..lower.len() - 2].parse::<f32>().ok()?
        },
        _ if lower.ends_with("px") => lower[..lower.len() - 2].parse::<f32>().ok()?,
        _ => return None,
    };
    if !px.is_finite() || !(0.0..=4096.0).contains(&px) {
        return None;
    }
    Some(format_css_px(px))
}

fn relative_font_size(parent_px: f32, larger: bool) -> f32 {
    const LADDER: [f32; 8] = [9.0, 10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0];
    if larger {
        LADDER
            .into_iter()
            .find(|candidate| *candidate > parent_px)
            .unwrap_or(parent_px * 1.5)
    } else {
        LADDER
            .into_iter()
            .rev()
            .find(|candidate| *candidate < parent_px)
            .unwrap_or(parent_px / 1.5)
    }
}

fn font_size_px(value: &str) -> Option<f32> {
    value
        .trim()
        .strip_suffix("px")?
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn format_css_px(px: f32) -> String {
    if px.fract().abs() < f32::EPSILON {
        format!("{px:.0}px")
    } else {
        let mut value = format!("{px:.4}");
        while value.ends_with('0') {
            value.pop();
        }
        format!("{value}px")
    }
}

/// Serializes substituted tokens without merging adjacent component values
/// into a different token on the layout/paint side of the string interface.
/// For example, `var(--n)px` with `--n: 1` remains `1 px` (invalid length),
/// rather than being accidentally retokenized as the valid dimension `1px`.
fn serialize_computed(tokens: &[Token]) -> String {
    fn word_like(token: &Token) -> bool {
        matches!(
            token,
            Token::Ident(_)
                | Token::Hash(_)
                | Token::Number(_)
                | Token::Percentage(_)
                | Token::Dimension(_, _)
        )
    }

    let mut output = String::new();
    let mut previous: Option<&Token> = None;
    for token in tokens {
        if matches!(token, Token::Whitespace) {
            if !output.is_empty() && !output.ends_with(' ') {
                output.push(' ');
            }
            previous = Some(token);
            continue;
        }
        if previous.is_some_and(word_like)
            && (word_like(token)
                || matches!(token, Token::Function(_) | Token::OpenParen))
        {
            output.push_str("/**/");
        }
        output.push_str(&serialize(std::slice::from_ref(token)));
        previous = Some(token);
    }
    output
}

struct VarCall<'a> {
    name: &'a str,
    fallback: Option<&'a [Token]>,
    end: usize,
}

fn parse_var_call(tokens: &[Token], function_index: usize) -> Option<VarCall<'_>> {
    if !matches!(tokens.get(function_index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
        || tokens.get(function_index + 1) != Some(&Token::OpenParen)
    {
        return None;
    }
    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    let mut comma = None;
    let mut close = None;
    for (offset, token) in tokens[function_index + 2..].iter().enumerate() {
        let index = function_index + 2 + offset;
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth == 0 => {
                close = Some(index);
                break;
            },
            Token::CloseParen => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Comma if paren_depth == 0 && square_depth == 0 && comma.is_none() => {
                comma = Some(index);
            },
            _ => {},
        }
    }
    let close = close?;
    let name_end = comma.unwrap_or(close);
    let name_tokens = trim_tokens(&tokens[function_index + 2..name_end]);
    let [Token::Ident(name)] = name_tokens else {
        return None;
    };
    if !name.starts_with("--") {
        return None;
    }
    let fallback = comma.map(|comma| &tokens[comma + 1..close]);
    Some(VarCall { name, fallback, end: close + 1 })
}

fn trim_tokens(mut tokens: &[Token]) -> &[Token] {
    while matches!(tokens.first(), Some(Token::Whitespace)) {
        tokens = &tokens[1..];
    }
    while matches!(tokens.last(), Some(Token::Whitespace)) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

/// Parser-facing validation for `var()` grammar and forbidden top-level
/// declaration remnants inside its fallback.
pub(crate) fn valid_variable_syntax(tokens: &[Token]) -> bool {
    let mut index = 0usize;
    while index < tokens.len() {
        if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var")) {
            let Some(call) = parse_var_call(tokens, index) else {
                return false;
            };
            if let Some(fallback) = call.fallback
                && (contains_bad_top_level_fallback_token(fallback)
                    || !valid_variable_syntax(fallback)
                )
            {
                return false;
            }
            index = call.end;
        } else {
            index += 1;
        }
    }
    true
}

pub(crate) fn valid_custom_property_value(tokens: &[Token]) -> bool {
    !contains_bad_top_level_fallback_token(tokens) && valid_variable_syntax(tokens)
}

fn contains_bad_top_level_fallback_token(tokens: &[Token]) -> bool {
    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    for token in tokens {
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth > 0 => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Semicolon | Token::Delim('!')
                if paren_depth == 0 && square_depth == 0 =>
            {
                return true;
            },
            _ => {},
        }
    }
    false
}

fn collect_dependencies(tokens: &[Token], dependencies: &mut BTreeSet<String>) {
    let mut index = 0usize;
    while index < tokens.len() {
        if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
            && let Some(call) = parse_var_call(tokens, index)
        {
            dependencies.insert(call.name.to_owned());
            if let Some(fallback) = call.fallback {
                collect_dependencies(fallback, dependencies);
            }
            index = call.end;
            continue;
        }
        index += 1;
    }
}

fn cycle_members(local: &BTreeMap<String, Vec<Token>>) -> BTreeSet<String> {
    fn visit(
        name: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        states: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
        cycles: &mut BTreeSet<String>,
    ) {
        match states.get(name).copied() {
            Some(1) => {
                if let Some(position) = stack.iter().position(|entry| entry == name) {
                    cycles.extend(stack[position..].iter().cloned());
                }
                return;
            },
            Some(2) => return,
            _ => {},
        }
        states.insert(name.to_owned(), 1);
        stack.push(name.to_owned());
        if let Some(dependencies) = graph.get(name) {
            for dependency in dependencies {
                if graph.contains_key(dependency) {
                    visit(dependency, graph, states, stack, cycles);
                }
            }
        }
        stack.pop();
        states.insert(name.to_owned(), 2);
    }

    let graph: BTreeMap<String, BTreeSet<String>> = local
        .iter()
        .map(|(name, tokens)| {
            let mut dependencies = BTreeSet::new();
            collect_dependencies(tokens, &mut dependencies);
            (name.clone(), dependencies)
        })
        .collect();
    let mut states = HashMap::new();
    let mut cycles = BTreeSet::new();
    for name in graph.keys() {
        visit(name, &graph, &mut states, &mut Vec::new(), &mut cycles);
    }
    cycles
}

struct CustomResolver<'a> {
    local: &'a BTreeMap<String, Vec<Token>>,
    inherited: &'a BTreeMap<String, CustomValue>,
    cycles: &'a BTreeSet<String>,
    memo: BTreeMap<String, CustomValue>,
    resolving: BTreeSet<String>,
}

impl CustomResolver<'_> {
    fn resolve_name(&mut self, name: &str) -> CustomValue {
        if let Some(value) = self.memo.get(name) {
            return value.clone();
        }
        if self.cycles.contains(name) || !self.resolving.insert(name.to_owned()) {
            return CustomValue::Invalid;
        }
        let value = match self.local.get(name).cloned() {
            Some(tokens) => self
                .substitute(&tokens)
                .map(CustomValue::Valid)
                .unwrap_or(CustomValue::Invalid),
            None => self
                .inherited
                .get(name)
                .cloned()
                .unwrap_or(CustomValue::Invalid),
        };
        self.resolving.remove(name);
        self.memo.insert(name.to_owned(), value.clone());
        value
    }

    fn substitute(&mut self, tokens: &[Token]) -> Option<Vec<Token>> {
        let mut output = Vec::new();
        let mut index = 0usize;
        while index < tokens.len() {
            if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var")) {
                let call = parse_var_call(tokens, index)?;
                let replacement = match self.resolve_name(call.name) {
                    CustomValue::Valid(tokens) => Some(tokens),
                    CustomValue::Invalid => call.fallback.and_then(|fallback| self.substitute(fallback)),
                }?;
                output.extend(replacement);
                index = call.end;
            } else {
                output.push(tokens[index].clone());
                index += 1;
            }
        }
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cascaded(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn case_sensitive_custom_properties_inherit_and_substitute() {
        let parent = resolve(
            None,
            cascaded(&[("--A", "green"), ("--a", "crimson")]),
        );
        let child = resolve(
            Some(&parent),
            cascaded(&[("color", "var(--A)"), ("background-color", "var(--a)")]),
        );
        assert_eq!(child.get("color"), Some("green"));
        assert_eq!(child.get("background-color"), Some("crimson"));
        assert_eq!(child.custom_serialized("--A"), Some(Some("green".into())));
        assert_eq!(child.custom_serialized("--a"), Some(Some("crimson".into())));
    }

    #[test]
    fn nested_fallback_and_fallback_commas_survive_substitution() {
        let style = resolve(
            None,
            cascaded(&[
                ("color", "var(--missing,var(--also-missing,green))"),
                ("font-family", "var(--family,serif,sans-serif)"),
            ]),
        );
        assert_eq!(style.get("color"), Some("green"));
        assert_eq!(style.get("font-family"), Some("serif,sans-serif"));
    }

    #[test]
    fn cycle_members_are_invalid_before_consumer_fallback_runs() {
        let style = resolve(
            None,
            cascaded(&[
                ("--a", "var(--b)"),
                ("--b", "var(--a,red)"),
                ("color", "var(--a,green)"),
            ]),
        );
        assert_eq!(style.custom_serialized("--a"), Some(None));
        assert_eq!(style.custom_serialized("--b"), Some(None));
        assert_eq!(style.get("color"), Some("green"));
    }

    #[test]
    fn missing_var_is_invalid_at_computed_value_time_not_previous_cascade_value() {
        let style = resolve(None, cascaded(&[("color", "var(--missing)")]));
        assert_eq!(style.get("color"), Some("black"));
    }

    #[test]
    fn css_wide_keywords_use_property_metadata() {
        let parent = resolve(
            None,
            cascaded(&[("color", "blue"), ("display", "block")]),
        );
        let child = resolve(
            Some(&parent),
            cascaded(&[
                ("color", "unset"),
                ("display", "inherit"),
                ("background-color", "initial"),
            ]),
        );
        assert_eq!(child.get("color"), Some("blue"));
        assert_eq!(child.get("display"), Some("block"));
        assert_eq!(child.get("background-color"), Some("transparent"));
    }

    #[test]
    fn all_expands_supported_properties_but_excludes_direction_and_unicode_bidi() {
        let expanded = expand_declaration("all", "initial");
        assert!(expanded.iter().any(|(name, _)| name == "color"));
        assert!(expanded.iter().any(|(name, _)| name == "margin-left"));
        assert!(!expanded.iter().any(|(name, _)| name == "direction"));
        assert!(!expanded.iter().any(|(name, _)| name == "unicode-bidi"));
    }

    #[test]
    fn variable_grammar_distinguishes_nested_from_top_level_remnants() {
        assert!(valid_custom_property_value(&tokenize("var(--a,(;))")));
        assert!(valid_custom_property_value(&tokenize("var(--a,)")));
        assert!(!valid_custom_property_value(&tokenize("var(--a,;)")));
        assert!(!valid_custom_property_value(&tokenize("var(--a,!)")));
    }

    #[test]
    fn substitution_preserves_token_boundaries() {
        let style = resolve(
            None,
            cascaded(&[("--n", "1"), ("width", "var(--n)px")]),
        );
        assert_eq!(style.get("width"), Some("1/**/px"));
        assert_eq!(tokenize(style.get("width").unwrap()), vec![Token::Number(1.0), Token::Ident("px".into())]);
    }

    #[test]
    fn public_set_uses_the_same_shorthand_expansion_seam() {
        let mut style = ComputedStyle::default();
        style.set("gap", "10px 20px");
        style.set("grid-column", "1 / -1");
        assert_eq!(style.get("row-gap"), Some("10px"));
        assert_eq!(style.get("column-gap"), Some("20px"));
        assert_eq!(style.get("grid-column-start"), Some("1"));
        assert_eq!(style.get("grid-column-end"), Some("-1"));
        assert_eq!(style.get("gap"), None);
        assert_eq!(style.get("grid-column"), None);
    }

    #[test]
    fn font_shorthand_expands_size_line_height_family_and_resets_optionals() {
        let expanded = expand_declaration("font", "italic 700 120px / 2 \"Arabic Test\",sans-serif");
        let values: BTreeMap<_, _> = expanded.into_iter().collect();

        assert_eq!(values.get("font-size").map(String::as_str), Some("120px"));
        assert_eq!(values.get("line-height").map(String::as_str), Some("2"));
        assert_eq!(values.get("font-family").map(String::as_str), Some("\"Arabic Test\",sans-serif"));
        assert_eq!(values.get("font-style").map(String::as_str), Some("italic"));
        assert_eq!(values.get("font-variant").map(String::as_str), Some("normal"));
        assert_eq!(values.get("font-weight").map(String::as_str), Some("700"));

        assert_eq!(
            expand_declaration("font", "italic oblique 16px serif"),
            vec![("font".into(), "italic oblique 16px serif".into())]
        );
        assert_eq!(
            expand_declaration("font", "16px \"Quoted\" adjacent"),
            vec![("font".into(), "16px \"Quoted\" adjacent".into())]
        );
        assert_eq!(
            expand_declaration(
                "font",
                "normal normal normal normal italic 16px serif"
            ),
            vec![(
                "font".into(),
                "normal normal normal normal italic 16px serif".into()
            )]
        );
    }
}
