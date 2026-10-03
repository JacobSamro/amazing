//! CSS core parser: qualified rules + declarations (css-syntax-3 §5).
//!
//! Phase 2 subset — implemented:
//! - stylesheets as a list of qualified rules (`selector-list { decls }`)
//! - comma-split selector preludes kept as source strings for the selectors
//!   module to parse
//! - `!important` flags (case-insensitive)
//! - spec-style error recovery: malformed declarations and rules are skipped,
//!   parsing continues
//!
//! Deferred/deviations:
//! - `@supports` evaluates declaration and selector conditions; `@media`,
//!   `@layer` and `@scope` retain group context; `@keyframes` is retained
//!   for the compositor; other at-rules are consumed
//!   and dropped
//! - nested style rules remain deferred
//! - declaration values stay normalized strings; the private computed-value
//!   module tokenizes them for custom-property substitution and defaulting

use crate::syntax::{Token, serialize, tokenize};

#[derive(Debug, Clone, PartialEq)]
pub struct Declaration {
    /// Lowercased property name.
    pub property: String,
    /// Normalized serialization of the value tokens.
    pub value: String,
    pub important: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Layer path; an empty selector list records a layer ordering statement.
    pub layer: Vec<LayerName>,
    pub scopes: Vec<Scope>,
    /// Nested `@media` conditions, all of which must match the environment.
    pub media: Vec<String>,
    /// One entry per top-level comma-separated selector (trimmed).
    pub selector_srcs: Vec<String>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// Animation rules in stylesheet order, including supported group rules.
    pub keyframes: Vec<crate::animation::KeyframesRule>,
    pub(crate) counter_styles: Vec<CounterStyleRule>,
    pub(crate) imports: Vec<crate::imports::ImportRule>,
    pub(crate) font_faces: Vec<crate::imports::ConditionalFontFace>,
    pub(crate) keyframe_media: Vec<Vec<String>>,
    pub(crate) projected_layers: bool,
    /// Parent of the stylesheet owner node, for implicit `@scope`.
    pub scope_root: Option<amazing_dom::NodeIdx>,
    /// Final fetched URL, or the document URL for an inline stylesheet.
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CounterStyleRule {
    pub name: String,
    pub descriptors: Vec<Declaration>,
    pub media: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LayerName {
    Named(String),
    Anonymous(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub start: Option<Vec<String>>,
    pub end: Option<Vec<String>>,
}

#[path = "group_rules.rs"]
mod group_rules;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontSource {
    pub url: String,
    pub local: Option<String>,
    pub format: Option<String>,
    pub technologies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFaceRule {
    pub family: String,
    pub sources: Vec<FontSource>,
    pub weight: String,
    pub style: String,
    pub stretch: String,
    pub unicode_range: Option<String>,
    pub display: Option<String>,
    pub size_adjust: Option<String>,
    pub ascent_override: Option<String>,
    pub descent_override: Option<String>,
    pub line_gap_override: Option<String>,
    pub feature_settings: Option<String>,
}

/// Parses a full stylesheet. Invalid constructs are dropped with recovery.
pub fn parse_stylesheet(src: &str) -> Stylesheet {
    let toks = tokenize(src);
    parse_rule_list(&toks)
}

// Comment removal can leave two identifier tokens adjacent (for example
// `of/*comment*/target`). Preserve the token boundary when a selector prelude
// is serialized and then parsed again by the selector module.
fn serialize_selector_prelude(tokens: &[Token]) -> String {
    let mut separated = Vec::with_capacity(tokens.len());
    for token in tokens {
        if matches!(separated.last(), Some(Token::Ident(_))) && matches!(token, Token::Ident(_)) {
            separated.push(Token::Whitespace);
        }
        separated.push(token.clone());
    }
    serialize(&separated)
}

/// Extracts `@font-face` descriptors without placing provider or resource
/// types in the CSS crate. Invalid rules are ignored with normal CSS recovery.
pub fn parse_font_faces(src: &str) -> Vec<FontFaceRule> {
    let tokens = tokenize(src);
    let mut faces = Vec::new();
    let mut position = 0usize;
    while position < tokens.len() {
        let is_font_face = matches!(
            tokens.get(position),
            Some(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("font-face")
        );
        if !is_font_face {
            match tokens.get(position) {
                Some(Token::Whitespace | Token::Semicolon) => position += 1,
                Some(Token::AtKeyword(_)) => {
                    let mut recovery = Parser {
                        toks: &tokens,
                        pos: position,
                    };
                    recovery.skip_at_rule();
                    position = recovery.pos;
                }
                Some(_) => position = skip_qualified_rule(&tokens, position),
                None => break,
            }
            continue;
        }
        let at_rule_start = position;
        position += 1;
        while matches!(tokens.get(position), Some(Token::Whitespace)) {
            position += 1;
        }
        if tokens.get(position) != Some(&Token::OpenCurly) {
            let mut recovery = Parser {
                toks: &tokens,
                pos: at_rule_start,
            };
            recovery.skip_at_rule();
            position = recovery.pos;
            continue;
        }
        let body_start = position + 1;
        position += 1;
        let mut depth = 1usize;
        while position < tokens.len() && depth > 0 {
            match tokens[position] {
                Token::OpenCurly => depth += 1,
                Token::CloseCurly => depth -= 1,
                _ => {}
            }
            position += 1;
        }
        if depth != 0 {
            break;
        }
        let body_end = position - 1;
        let mut declaration_parser = Parser {
            toks: &tokens[body_start..body_end],
            pos: 0,
        };
        let declarations = declaration_parser.block_declarations();
        let descriptor = |name: &str| {
            declarations
                .iter()
                .rev()
                .find(|declaration| declaration.property.eq_ignore_ascii_case(name))
                .map(|declaration| declaration.value.as_str())
        };
        let Some(family) = descriptor("font-family").and_then(parse_font_family_descriptor) else {
            continue;
        };
        let sources = descriptor("src")
            .map(parse_font_sources)
            .unwrap_or_default();
        if sources.is_empty() {
            continue;
        }
        faces.push(FontFaceRule {
            family,
            sources,
            weight: descriptor("font-weight").unwrap_or("normal").to_owned(),
            style: descriptor("font-style").unwrap_or("normal").to_owned(),
            stretch: descriptor("font-stretch").unwrap_or("normal").to_owned(),
            // CSS-26: `unicode-range` values now come through as verbatim
            // `Token::UnicodeRange` text (see syntax.rs's `<urange>`
            // tokenization), so `descriptor()` already yields correct,
            // sign/leading-zero-preserving spellings like `U+0131`; this
            // just normalizes the canonical ", " list separator down to a
            // bare comma for this field's established (no-space) format.
            unicode_range: descriptor("unicode-range").map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .collect::<Vec<_>>()
                    .join(",")
            }),
            display: descriptor("font-display").map(str::to_owned),
            size_adjust: descriptor("size-adjust").map(str::to_owned),
            ascent_override: descriptor("ascent-override").map(str::to_owned),
            descent_override: descriptor("descent-override").map(str::to_owned),
            line_gap_override: descriptor("line-gap-override").map(str::to_owned),
            feature_settings: descriptor("font-feature-settings").map(str::to_owned),
        });
    }
    faces
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ComponentEnd {
    Paren,
    Square,
    Curly,
}

fn component_end(token: &Token) -> Option<ComponentEnd> {
    match token {
        Token::OpenParen => Some(ComponentEnd::Paren),
        Token::OpenSquare => Some(ComponentEnd::Square),
        Token::OpenCurly => Some(ComponentEnd::Curly),
        _ => None,
    }
}

fn closes_component(token: &Token, end: ComponentEnd) -> bool {
    matches!(
        (token, end),
        (Token::CloseParen, ComponentEnd::Paren)
            | (Token::CloseSquare, ComponentEnd::Square)
            | (Token::CloseCurly, ComponentEnd::Curly)
    )
}

fn skip_rule_remainder(tokens: &[Token], mut position: usize, semicolon_ends_rule: bool) -> usize {
    let mut stack = Vec::new();
    let mut rule_block_started = false;
    while position < tokens.len() {
        let token = &tokens[position];
        if let Some(end) = component_end(token) {
            if end == ComponentEnd::Curly && stack.is_empty() {
                rule_block_started = true;
            }
            stack.push(end);
        } else if stack
            .last()
            .is_some_and(|end| closes_component(token, *end))
        {
            stack.pop();
            if rule_block_started && stack.is_empty() {
                position += 1;
                return position;
            }
        } else if semicolon_ends_rule
            && !rule_block_started
            && stack.is_empty()
            && matches!(token, Token::Semicolon)
        {
            return position + 1;
        }
        position += 1;
    }
    position
}

fn skip_qualified_rule(tokens: &[Token], position: usize) -> usize {
    skip_rule_remainder(tokens, position, false)
}

fn parse_font_family_descriptor(value: &str) -> Option<String> {
    let tokens = tokenize(value);
    let significant = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace))
        .collect::<Vec<_>>();
    match significant.as_slice() {
        [Token::Str(value)] => Some(value.clone()),
        values
            if !values.is_empty()
                && values.iter().all(|token| matches!(token, Token::Ident(_))) =>
        {
            Some(serialize(&tokens))
        }
        _ => None,
    }
}

fn parse_font_sources(value: &str) -> Vec<FontSource> {
    let tokens = tokenize(value);
    let mut sources = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    for index in 0..=tokens.len() {
        let at_end = index == tokens.len();
        if !at_end {
            match tokens[index] {
                Token::OpenParen | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if (at_end || matches!(tokens[index], Token::Comma)) && depth == 0 {
            if let Some(source) = parse_font_source_candidate(&tokens[start..index]) {
                sources.push(source);
            }
            start = index + 1;
        }
    }
    sources
}

fn parse_font_source_candidate(tokens: &[Token]) -> Option<FontSource> {
    let mut position = 0usize;
    skip_whitespace(tokens, &mut position);
    if let Some(Token::Url { value, .. }) = tokens.get(position) {
        if value.is_empty() { return None; }
        let url = value.clone();
        position += 1;
        skip_whitespace(tokens, &mut position);
        let mut format = None;
        let mut technologies = Vec::new();
        while let Some(Token::Function(name)) = tokens.get(position) {
            let (argument, after_function) = function_argument(tokens, position)?;
            if name.eq_ignore_ascii_case("format") && format.is_none() {
                format = Some(parse_one_ident_or_string(argument)?);
            } else if name.eq_ignore_ascii_case("tech") {
                technologies.push(parse_one_ident_or_string(argument)?);
            } else { return None; }
            position = after_function;
            skip_whitespace(tokens, &mut position);
        }
        return (position == tokens.len()).then_some(FontSource { url, local: None, format, technologies });
    }
    let Token::Function(name) = tokens.get(position)? else {
        return None;
    };
    if !name.eq_ignore_ascii_case("local") && !name.eq_ignore_ascii_case("url") {
        return None;
    }
    let (argument, after_url) = function_argument(tokens, position)?;
    let local = if name.eq_ignore_ascii_case("local") {
        Some(parse_one_ident_or_string(argument)?)
    } else {
        None
    };
    let url = if local.is_some() {
        String::new()
    } else {
        parse_url_argument(argument)?
    };
    position = after_url;
    skip_whitespace(tokens, &mut position);
    let mut format = None;
    let mut technologies = Vec::new();
    while let Some(Token::Function(name)) = tokens.get(position) {
        let (argument, after_function) = function_argument(tokens, position)?;
        if name.eq_ignore_ascii_case("format") && format.is_none() && local.is_none() {
            format = Some(parse_one_ident_or_string(argument)?);
        } else if name.eq_ignore_ascii_case("tech") && local.is_none() {
            technologies.push(parse_one_ident_or_string(argument)?);
        } else {
            return None;
        }
        position = after_function;
        skip_whitespace(tokens, &mut position);
    }
    (position == tokens.len()).then_some(FontSource {
        url,
        local,
        format,
        technologies,
    })
}

fn parse_url_argument(tokens: &[Token]) -> Option<String> {
    let significant = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace))
        .collect::<Vec<_>>();
    if let [Token::Str(value)] = significant.as_slice() {
        return Some(value.clone());
    }
    if tokens.is_empty()
        || tokens.iter().any(|token| {
            matches!(
                token,
                Token::Whitespace
                    | Token::Str(_)
                    | Token::Function(_)
                    | Token::AtKeyword(_)
                    | Token::OpenCurly
                    | Token::CloseCurly
                    | Token::OpenParen
                    | Token::CloseParen
                    | Token::OpenSquare
                    | Token::CloseSquare
            )
        })
    {
        return None;
    }
    let url = serialize(tokens);
    (!url.is_empty()).then_some(url)
}

fn parse_one_ident_or_string(tokens: &[Token]) -> Option<String> {
    let significant = tokens
        .iter()
        .filter(|token| !matches!(token, Token::Whitespace))
        .collect::<Vec<_>>();
    match significant.as_slice() {
        [Token::Ident(value)] | [Token::Str(value)] => Some(value.clone()),
        _ => None,
    }
}

fn skip_whitespace(tokens: &[Token], position: &mut usize) {
    while matches!(tokens.get(*position), Some(Token::Whitespace)) {
        *position += 1;
    }
}

fn function_argument(tokens: &[Token], function: usize) -> Option<(&[Token], usize)> {
    if !matches!(tokens.get(function), Some(Token::Function(_)))
        || tokens.get(function + 1) != Some(&Token::OpenParen)
    {
        return None;
    }
    let mut depth = 1usize;
    let mut position = function + 2;
    while position < tokens.len() {
        match tokens[position] {
            Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    return Some((&tokens[function + 2..position], position + 1));
                }
            }
            _ => {}
        }
        position += 1;
    }
    None
}

fn parse_rule_list(tokens: &[Token]) -> Stylesheet {
    let mut parser = Parser {
        toks: tokens,
        pos: 0,
    };
    parser.rule_list()
}

/// Parses a standalone declaration block — the contents of an inline
/// `style=""` attribute. Runs to EOF; malformed declarations are dropped
/// with the same recovery rules as stylesheet blocks.
pub fn parse_declarations(src: &str) -> Vec<Declaration> {
    let toks = tokenize(src);
    let mut p = Parser {
        toks: &toks,
        pos: 0,
    };
    p.block_declarations()
}

pub(crate) struct Parser<'t> {
    pub(crate) toks: &'t [Token],
    pub(crate) pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn rule_list(&mut self) -> Stylesheet {
        group_rules::parse(self.toks, &[], &[], &[], &mut 0, 0)
    }

    /// Skips an at-rule: through the next ';' or its balanced {} block.
    pub(crate) fn skip_at_rule(&mut self) {
        debug_assert!(matches!(self.peek(), Some(Token::AtKeyword(_))));
        self.pos = skip_rule_remainder(self.toks, self.pos + 1, true);
    }

    /// Prelude up to '{', then declarations until the matching '}'.
    /// Returns None only on EOF-without-block (rule discarded).
    fn qualified_rule(&mut self) -> Option<Rule> {
        // collect prelude tokens (skipping leading whitespace)
        let prelude_start = self.pos;
        loop {
            match self.peek() {
                None => return None,
                Some(Token::Whitespace) => self.pos += 1,
                _ => break,
            }
        }
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => return None,
                // at-keyword mid-prelude: not a qualified rule; bail out by
                // consuming everything up to the block and dropping it
                Some(Token::OpenCurly) if depth == 0 => break,
                Some(Token::OpenParen | Token::OpenSquare) => depth += 1,
                Some(Token::CloseParen | Token::CloseSquare) => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.pos += 1;
        }
        let prelude_toks = &self.toks[prelude_start..self.pos];
        self.pos += 1; // consume '{'

        let declarations = self.block_declarations();

        // split prelude on top-level commas
        let mut selector_srcs = Vec::new();
        let mut current_start = 0usize;
        let mut depth = 0usize;
        for (i, t) in prelude_toks.iter().enumerate() {
            match t {
                Token::OpenParen | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
                Token::Comma if depth == 0 => {
                    let seg = serialize_selector_prelude(&prelude_toks[current_start..i]);
                    if !seg.trim().is_empty() {
                        selector_srcs.push(seg);
                    }
                    current_start = i + 1;
                }
                _ => {}
            }
        }
        if current_start < prelude_toks.len() {
            let seg = serialize_selector_prelude(&prelude_toks[current_start..]);
            if !seg.trim().is_empty() {
                selector_srcs.push(seg);
            }
        }

        Some(Rule {
            selector_srcs,
            declarations,
            layer: Vec::new(),
            scopes: Vec::new(),
            media: Vec::new(),
        })
    }

    /// Reads declarations until the block's closing '}' (consumed) or EOF.
    pub(crate) fn block_declarations(&mut self) -> Vec<Declaration> {
        let mut out = Vec::new();
        let mut current: Vec<Token> = Vec::new();
        let mut depth = 0usize;
        loop {
            match self.peek().cloned() {
                None => {
                    out.extend(self.finish_declaration(&current));
                    return out;
                }
                // Parens belong to the value (rgba(…), var(…)); track nesting
                // but keep the tokens so serialization stays faithful.
                Some(tok @ (Token::OpenParen | Token::OpenSquare)) => {
                    depth += 1;
                    current.push(tok);
                    self.pos += 1;
                }
                Some(tok @ (Token::CloseParen | Token::CloseSquare)) => {
                    depth = depth.saturating_sub(1);
                    current.push(tok);
                    self.pos += 1;
                }
                // A nested '{' inside a value is invalid here: drop this
                // declaration through the matching '}' (recovery).
                Some(Token::OpenCurly) => {
                    current.clear();
                    let mut d = 1usize;
                    self.pos += 1;
                    while d > 0 {
                        match self.peek() {
                            None => return out,
                            Some(Token::OpenCurly) => d += 1,
                            Some(Token::CloseCurly) => d -= 1,
                            _ => {}
                        }
                        self.pos += 1;
                    }
                    return out;
                }
                Some(Token::CloseCurly) => {
                    self.pos += 1;
                    if depth == 0 {
                        out.extend(self.finish_declaration(&current));
                        return out;
                    }
                    depth -= 1; // stray closer inside parens — tolerate
                }
                Some(Token::Semicolon) if depth == 0 => {
                    self.pos += 1;
                    out.extend(self.finish_declaration(&current));
                    current.clear();
                }
                // Whitespace between declarations is not part of the next
                // declaration's fragment; inside values/parens it is kept
                // (serialize collapses runs to single spaces).
                Some(Token::Whitespace) if depth == 0 && current.is_empty() => {
                    self.pos += 1;
                }
                Some(tok) => {
                    current.push(tok);
                    self.pos += 1;
                }
            }
        }
    }

    /// Turns accumulated value tokens into a Declaration, or None when the
    /// fragment is not a well-formed declaration.
    fn finish_declaration(&self, toks: &[Token]) -> Option<Declaration> {
        // leading whitespace / stray semicolons already filtered upstream
        let property = match toks.first() {
            Some(Token::Ident(name)) if name.starts_with("--") => name.clone(),
            Some(Token::Ident(name)) => name.to_ascii_lowercase(),
            _ => return None, // missing colon / garbage → drop (recovery)
        };
        let mut i = 1; // past the property ident
        // skip whitespace, expect ':'
        while matches!(toks.get(i), Some(Token::Whitespace)) {
            i += 1;
        }
        if toks.get(i) != Some(&Token::Colon) {
            return None;
        }
        i += 1;

        // value tokens: trim ends, detect '! important'
        let mut start = i;
        let mut end = toks.len();
        while matches!(toks.get(start), Some(Token::Whitespace)) {
            start += 1;
        }
        while end > start && matches!(toks.get(end - 1), Some(Token::Whitespace)) {
            end -= 1;
        }
        if start > end || (start == end && !property.starts_with("--")) {
            return None; // only custom declarations can have an empty value
        }

        let mut important = false;
        // pattern: ... '!' WS? Ident("important")
        let mut scan = end;
        if scan > start
            && let Some(Token::Ident(name)) = toks.get(scan - 1)
            && name.eq_ignore_ascii_case("important")
        {
            scan -= 1;
            while scan > start && matches!(toks.get(scan - 1), Some(Token::Whitespace)) {
                scan -= 1;
            }
            if scan > start && toks.get(scan - 1) == Some(&Token::Delim('!')) {
                important = true;
                end = scan - 1;
                while end > start && matches!(toks.get(end - 1), Some(Token::Whitespace)) {
                    end -= 1;
                }
            }
        }

        if start > end || (start == end && !property.starts_with("--")) {
            return None;
        }
        let value_tokens = &toks[start..end];
        if !crate::computed::valid_variable_syntax(value_tokens)
            || (property.starts_with("--")
                && !crate::computed::valid_custom_property_value(value_tokens))
        {
            return None;
        }
        let value = serialize(value_tokens);
        if value.is_empty() && !property.starts_with("--") {
            return None;
        }
        Some(Declaration {
            property,
            value,
            important,
        })
    }
}

fn supports_condition(tokens: &[Token]) -> bool {
    crate::imports::SupportsCondition::parse_tokens(tokens, false)
        .is_some_and(|condition| condition.evaluate())
}

pub(crate) fn supports_declaration(name_tokens: &[Token], value_tokens: &[Token]) -> bool {
    let [Token::Ident(name)] = trim_ws(name_tokens) else {
        return false;
    };
    let value_tokens = trim_ws(value_tokens);
    if (value_tokens.is_empty() && !name.starts_with("--")) || !crate::computed::valid_variable_syntax(value_tokens) {
        return false;
    }
    if name.starts_with("--") {
        return crate::computed::valid_custom_property_value(value_tokens);
    }
    if name.eq_ignore_ascii_case("color") && !value_tokens.iter().any(
        |token| matches!(token, Token::Function(function) if function.eq_ignore_ascii_case("var")),
    ) {
        return crate::color::parse_css_color(&serialize(value_tokens)).is_some();
    }
    (crate::computed::is_supported_property(name)
        || name.eq_ignore_ascii_case("font-synthesis"))
        && (value_tokens.iter().any(
            |token| matches!(token, Token::Function(function) if function.eq_ignore_ascii_case("var")),
        ) || if name.eq_ignore_ascii_case("font-synthesis") {
            crate::font_properties::expand_synthesis(&serialize(value_tokens)).is_some()
        } else if crate::font_properties::is_audited_longhand(&name.to_ascii_lowercase())
            || matches!(name.to_ascii_lowercase().as_str(), "font-feature-settings" | "font-variant-ligatures"
                | "image-orientation" | "object-fit" | "object-position") {
            let value = serialize(value_tokens);
            matches!(value.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset" | "revert" | "revert-layer")
                || crate::values::parse(&name.to_ascii_lowercase(), &value).is_some()
        } else {
            let value = serialize(value_tokens);
            matches!(value.to_ascii_lowercase().as_str(), "inherit" | "initial" | "unset" | "revert" | "revert-layer")
                || crate::values::parse(&name.to_ascii_lowercase(), &value).is_some()
        })
}

fn trim_ws(mut tokens: &[Token]) -> &[Token] {
    while matches!(tokens.first(), Some(Token::Whitespace)) {
        tokens = &tokens[1..];
    }
    while matches!(tokens.last(), Some(Token::Whitespace)) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(src: &str) -> Vec<(String, String, bool)> {
        parse_stylesheet(src)
            .rules
            .into_iter()
            .flat_map(|r| {
                r.declarations
                    .into_iter()
                    .map(|d| (d.property, d.value, d.important))
            })
            .collect()
    }

    #[test]
    fn simple_rule_parses() {
        let sheet = parse_stylesheet("h1 { color: red; font-size: 14px }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector_srcs, vec!["h1"]);
        assert_eq!(
            sheet.rules[0].declarations,
            vec![
                Declaration {
                    property: "color".into(),
                    value: "red".into(),
                    important: false
                },
                Declaration {
                    property: "font-size".into(),
                    value: "14px".into(),
                    important: false
                },
            ]
        );
    }

    #[test]
    fn selector_list_splits_on_commas() {
        let sheet = parse_stylesheet("a, .b > c { color: blue }");
        assert_eq!(sheet.rules[0].selector_srcs, vec!["a", ".b > c"]);
    }

    #[test]
    fn important_flag_case_insensitive() {
        assert_eq!(
            decls("p{margin:0 !IMPORTANT}"),
            vec![("margin".into(), "0".into(), true)]
        );
        assert_eq!(
            decls("p{margin:0!important}"),
            vec![("margin".into(), "0".into(), true)]
        );
        assert_eq!(
            decls("p{margin:0}"),
            vec![("margin".into(), "0".into(), false)]
        );
    }

    #[test]
    fn invalid_declarations_dropped_with_recovery() {
        // empty value + missing colon both dropped; good one survives
        let d = decls("p{ color : ; color red ; color: green }");
        assert_eq!(d, vec![("color".into(), "green".into(), false)]);
    }

    #[test]
    fn media_rules_are_retained_and_imports_skipped() {
        let sheet =
            parse_stylesheet("@import url(x.css); @media screen { p { color: red } } div { x:y }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector_srcs, vec!["p"]);
        assert_eq!(sheet.rules[0].media, vec!["screen"]);
        assert_eq!(sheet.rules[1].selector_srcs, vec!["div"]);
        assert_eq!(sheet.rules[1].declarations[0].value, "y");
    }

    #[test]
    fn font_face_extracts_provider_neutral_descriptors() {
        let faces = parse_font_faces(
            "@font-face {\n\
             font-family: 'csstest_noto';\n\
             src: local('ignored'), url('/fonts/noto/NotoNaskhArabic-regular.woff2') format('woff2');\n\
             font-weight: normal; font-style: italic;\n\
             }",
        );

        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].family, "csstest_noto");
        assert_eq!(faces[0].weight, "normal");
        assert_eq!(faces[0].style, "italic");
        assert_eq!(faces[0].stretch, "normal");
        assert_eq!(
            faces[0].sources,
            vec![
                FontSource {
                    url: String::new(),
                    local: Some("ignored".into()),
                    format: None,
                    technologies: Vec::new(),
                },
                FontSource {
                    url: "/fonts/noto/NotoNaskhArabic-regular.woff2".into(),
                    local: None,
                    format: Some("woff2".into()),
                    technologies: Vec::new(),
                }
            ]
        );
    }

    #[test]
    fn unquoted_data_url_survives_declarations_and_font_sources() {
        let url = "data:font/woff2;base64,AA+10/20==";
        let css = format!("@font-face {{ font-family: Data; src: url({url}) format(woff2) }}");
        let faces = parse_font_faces(&css);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].sources[0].url, url);
        let declarations = parse_declarations(&format!("background-image: url({url});"));
        assert_eq!(declarations[0].value, format!("url({url})"));
    }

    #[test]
    fn invalid_font_face_descriptor_grammars_are_rejected() {
        assert!(
            parse_font_faces(
                "@font-face { font-family: One,Two; src: url(font.woff2) format(woff2) }"
            )
            .is_empty()
        );
        assert!(
            parse_font_faces("@font-face { font-family: One; src: junk url(font.woff2) }")
                .is_empty()
        );
        assert!(
            parse_font_faces("@font-face { font-family: One; src: url(bad path.woff2) }")
                .is_empty()
        );
    }

    #[test]
    fn font_face_exposes_ordered_local_url_and_extended_descriptors() {
        let faces = parse_font_faces(
            "@font-face { font-family: Demo; src: local('Demo Local'), url('../demo.woff2') format('woff2') tech(color-COLRv1); font-weight: 300 700; font-stretch: 80% 120%; font-style: oblique 10deg 20deg; unicode-range: U+41-5A, U+4??; font-display: swap; size-adjust: 90%; ascent-override: 80%; descent-override: 20%; line-gap-override: 0%; font-feature-settings: 'liga' 0; }",
        );
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].sources[0].local.as_deref(), Some("Demo Local"));
        assert_eq!(faces[0].sources[1].url, "../demo.woff2");
        assert_eq!(faces[0].sources[1].technologies, ["color-COLRv1"]);
        assert_eq!(faces[0].unicode_range.as_deref(), Some("U+41-5A,U+4??"));
        assert_eq!(faces[0].display.as_deref(), Some("swap"));
        assert_eq!(faces[0].size_adjust.as_deref(), Some("90%"));
        assert_eq!(faces[0].ascent_override.as_deref(), Some("80%"));
        assert_eq!(faces[0].descent_override.as_deref(), Some("20%"));
        assert_eq!(faces[0].line_gap_override.as_deref(), Some("0%"));
        assert_eq!(faces[0].feature_settings.as_deref(), Some("\"liga\" 0"));
    }

    // CSS-26 regression: `@font-face { unicode-range: U+0131; }` used to
    // reach this field as `"u131"` (the '+' sign and the leading zero were
    // lost when the descriptor's value was re-tokenized generically as
    // `Ident("u")` + `Number(131.0)`, then re-serialized). A real
    // `@font-face` rule for Turkish dotless-i coverage, exactly the reported
    // shape.
    #[test]
    fn font_face_unicode_range_preserves_sign_and_leading_zeros() {
        let faces = parse_font_faces(
            "@font-face { font-family: Tr; src: url(tr.woff2); unicode-range: U+0131; }",
        );
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].unicode_range.as_deref(), Some("U+0131"));
    }

    #[test]
    fn font_face_unicode_range_covers_single_range_and_wildcard_forms() {
        // Single low-codepoint value with no leading zero.
        let faces = parse_font_faces(
            "@font-face { font-family: A; src: url(a.woff2); unicode-range: U+41; }",
        );
        assert_eq!(faces[0].unicode_range.as_deref(), Some("U+41"));

        // Explicit hyphenated range, wildcard, and a leading-zero single
        // value together in one real descriptor list (mirrors a Latin +
        // CJK-fallback @font-face split).
        let faces = parse_font_faces(
            "@font-face { font-family: B; src: url(b.woff2); \
             unicode-range: U+0041-005A, U+4??, U+0100; }",
        );
        assert_eq!(faces.len(), 1);
        assert_eq!(
            faces[0].unicode_range.as_deref(),
            Some("U+0041-005A,U+4??,U+0100")
        );
    }

    #[test]
    fn font_face_recovery_consumes_invalid_rules_and_last_descriptor_wins() {
        assert!(
            parse_font_faces(
                "@font-face invalid @font-face { font-family: Wrong; src: url(wrong.woff2) }"
            )
            .is_empty()
        );

        let faces = parse_font_faces(
            "@font-face { font-family: Wrong; font-family: Right; \
             src: url(wrong.woff2); src: url(right.woff2) format(woff2) }",
        );
        assert_eq!(faces[0].family, "Right");
        assert_eq!(faces[0].sources[0].url, "right.woff2");

        let faces = parse_font_faces(
            "@bad fn(; @font-face { font-family: Wrong; src: url(wrong.woff2) }); \
             @font-face { font-family: Right; src: url(right.woff2) }",
        );
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].family, "Right");

        assert!(
            parse_font_faces(
                "@bad fn([)]; @font-face { font-family: Wrong; src: url(wrong.woff2) }"
            )
            .is_empty()
        );

        assert!(
            parse_font_faces("garbage; @font-face { font-family: Wrong; src: url(wrong.woff2) }")
                .is_empty()
        );
    }

    #[test]
    fn function_values_keep_function_syntax() {
        let d = decls("p{ background: rgba(255, 0, 0, 0.5) }");
        assert_eq!(
            d,
            vec![("background".into(), "rgba(255, 0, 0, .5)".into(), false)]
        );
    }

    #[test]
    fn unterminated_block_at_eof_recovers() {
        let d = decls("p { color: red");
        assert_eq!(d, vec![("color".into(), "red".into(), false)]);
    }

    #[test]
    fn whitespace_normalized_in_values() {
        let d = decls("p{ margin:\n\t 1px   2px }");
        assert_eq!(d, vec![("margin".into(), "1px 2px".into(), false)]);
    }

    #[test]
    fn custom_property_names_preserve_case() {
        let declarations = decls("p{--A:green;--a:crimson;COLOR:blue}");
        assert_eq!(
            declarations,
            vec![
                ("--A".into(), "green".into(), false),
                ("--a".into(), "crimson".into(), false),
                ("color".into(), "blue".into(), false),
            ]
        );
    }

    #[test]
    fn invalid_top_level_var_fallback_remnants_drop_the_declaration() {
        let declarations =
            decls("p{--a:green;--a:var(--b,!);--b:blue;--b:var(--c,;);color:var(--a,(;))}");
        assert_eq!(
            declarations,
            vec![
                ("--a".into(), "green".into(), false),
                ("--b".into(), "blue".into(), false),
                ("color".into(), "var(--a, (;))".into(), false),
            ]
        );
    }

    #[test]
    fn supports_accepts_valid_variable_grammar_and_boolean_and() {
        let sheet = parse_stylesheet(
            "@supports (color:1px var(--a)) { p{color:green} }\
             @supports (--a:a) and (--a:var(--b,)) { div{color:blue} }\
             @supports (unknown:var(--a)) { span{color:red} }",
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector_srcs, ["p"]);
        assert_eq!(sheet.rules[1].selector_srcs, ["div"]);
    }

    #[test]
    fn supports_selector_and_at_rule_conditions() {
        let sheet = parse_stylesheet(
            "@supports selector(div > :first-child) and at-rule(@media) { a { color: green } } \
             @supports selector(:not-a-real-pseudo) { b { color: red } } \
             @supports at-rule(@unknown) { c { color: red } }",
        );
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector_srcs, ["a"]);
    }
}
