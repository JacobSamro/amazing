//! CSS Cascade 5 §2 ordered imports, typed conditions and loaded projection.
//! Transport, decoding, MIME and response URL resolution belong to the caller.
use crate::{Stylesheet, Rule, FontFaceRule, MediaEnvironment, MediaQueryList};
use crate::parser::LayerName;
use crate::syntax::{Token, tokenize, serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportLayer { Named(Vec<String>), Anonymous }
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRule {
    pub url: String,
    /// Insertion slot among stylesheet style/layer-ordering rules.
    pub rule_index: usize,
    pub layer: Option<ImportLayer>,
    pub supports: Option<SupportsCondition>,
    pub media: MediaQueryList,
}
impl ImportRule {
    /// False supports must not be fetched (Cascade 5 §2.1).
    pub fn supports_matches(&self) -> bool {
        self.supports.as_ref().is_none_or(SupportsCondition::evaluate)
    }
    pub fn conditions_match(&self, environment: &MediaEnvironment) -> bool {
        self.supports_matches() && self.media.evaluate(environment)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum SupportsCondition {
    Declaration { property: String, value: String },
    Selector(String), AtRule(String), GeneralEnclosed(String),
    Not(Box<SupportsCondition>), And(Vec<SupportsCondition>), Or(Vec<SupportsCondition>),
}
impl SupportsCondition {
    pub fn parse(css: &str) -> Option<Self> { Self::parse_tokens(&tokenize(css), false) }
    pub fn evaluate(&self) -> bool {
        match self {
            Self::Declaration { property, value } => crate::parser::supports_declaration(
                &tokenize(property), &tokenize(value)),
            Self::Selector(css) => crate::selectors::parse_selector(css).is_some(),
            Self::AtRule(name) => matches!(name.as_str(), "media" | "supports" | "layer" | "scope"
                | "keyframes" | "font-face" | "counter-style" | "import"),
            Self::GeneralEnclosed(_) => false,
            Self::Not(value) => !value.evaluate(),
            Self::And(values) => values.iter().all(Self::evaluate),
            Self::Or(values) => values.iter().any(Self::evaluate),
        }
    }
    pub(crate) fn parse_tokens(tokens: &[Token], declaration: bool) -> Option<Self> {
        fn parse(tokens: &[Token], declaration: bool, depth: usize) -> Option<SupportsCondition> {
            if depth > 128 { return None; }
            let tokens = trim(tokens);
            if declaration {
                if let Some(result) = decl(tokens) { return Some(result); }
            }
            if matches!(tokens.first(),Some(Token::Ident(name)) if name.eq_ignore_ascii_case("not")) {
                return Some(SupportsCondition::Not(Box::new(atom(trim(&tokens[1..]), depth + 1)?)));
            }
            let mut cursor = 0;
            let mut parts = Vec::new();
            let mut operator = None;
            while cursor < tokens.len() {
                let end = component_end(tokens, cursor)?;
                parts.push(atom(&tokens[cursor..end], depth + 1)?);
                cursor = end;
                while matches!(tokens.get(cursor),Some(Token::Whitespace)) { cursor += 1; }
                if cursor == tokens.len() { break; }
                let Token::Ident(name) = tokens.get(cursor)? else { return None; };
                let and = if name.eq_ignore_ascii_case("and") { true }
                    else if name.eq_ignore_ascii_case("or") { false } else { return None; };
                if operator.is_some_and(|old| old != and) { return None; }
                operator = Some(and);
                cursor += 1;
                // Operators are separated tokens, including comment boundaries.
                while matches!(tokens.get(cursor),Some(Token::Whitespace)) { cursor += 1; }
                if cursor == tokens.len() { return None; }
            }
            match (operator, parts.len()) {
                (_, 0) => None,
                (None, 1) => parts.pop(),
                (Some(true), _) => Some(SupportsCondition::And(parts)),
                (Some(false), _) => Some(SupportsCondition::Or(parts)),
                _ => None,
            }
        }
        fn atom(tokens: &[Token], depth: usize) -> Option<SupportsCondition> {
            let tokens = trim(tokens);
            if component_end(tokens, 0)? != tokens.len() { return None; }
            match tokens.first()? {
                Token::OpenParen => {
                    let inner = trim(&tokens[1..tokens.len()-1]);
                    if let Some(value) = decl(inner) { return Some(value); }
                    if let Some(value) = parse(inner, false, depth) { return Some(value); }
                    matches!(inner.first(), Some(Token::Ident(_)))
                        .then(|| SupportsCondition::GeneralEnclosed(serialize(tokens)))
                }
                Token::Function(name) => {
                    let inner = trim(&tokens[2..tokens.len()-1]);
                    if name.eq_ignore_ascii_case("selector") {
                        return Some(SupportsCondition::Selector(serialize(inner)));
                    }
                    if name.eq_ignore_ascii_case("at-rule") {
                        if let [Token::AtKeyword(name)] = inner {
                            return Some(SupportsCondition::AtRule(name.to_ascii_lowercase()));
                        }
                    }
                    Some(SupportsCondition::GeneralEnclosed(serialize(tokens)))
                }
                _ => None,
            }
        }
        fn decl(tokens: &[Token]) -> Option<SupportsCondition> {
            let tokens = trim(tokens);
            let Token::Ident(property) = tokens.first()? else { return None; };
            let rest = trim(&tokens[1..]);
            if rest.first() != Some(&Token::Colon) { return None; }
            let value = trim(&rest[1..]);
            if (value.is_empty() && !property.starts_with("--")) || !balanced(value)
                || value.iter().any(|t| matches!(t,Token::Semicolon)) { return None; }
            Some(SupportsCondition::Declaration { property: property.to_ascii_lowercase(), value: serialize(value) })
        }
        if !balanced(tokens) { return None; }
        parse(tokens, declaration, 0)
    }
}
fn trim(mut tokens: &[Token]) -> &[Token] {
    while tokens.first() == Some(&Token::Whitespace) { tokens = &tokens[1..]; }
    while tokens.last() == Some(&Token::Whitespace) { tokens = &tokens[..tokens.len()-1]; }
    tokens
}
fn balanced(tokens: &[Token]) -> bool {
    let mut stack = Vec::new();
    for token in tokens {
        match token {
            Token::OpenParen => stack.push(Token::CloseParen),
            Token::OpenSquare => stack.push(Token::CloseSquare),
            Token::OpenCurly => stack.push(Token::CloseCurly),
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => if stack.pop().as_ref() != Some(token) { return false; },
            Token::BadUrl => return false,
            _ => (),
        }
        if stack.len() > 128 { return false; }
    }
    stack.is_empty()
}
fn component_end(tokens: &[Token], start: usize) -> Option<usize> {
    let open = match tokens.get(start)? {
        Token::Function(_) if tokens.get(start+1) == Some(&Token::OpenParen) => start+1,
        Token::OpenParen => start,
        _ => return None,
    };
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth = depth.checked_sub(1)?;
                if depth == 0 { return Some(i+1); }
            }
            _ => (),
        }
    }
    None
}
pub(crate) fn parse_import(tokens: &[Token], rule_index: usize) -> Option<ImportRule> {
    let tokens = trim(tokens);
    if !balanced(tokens) { return None; }
    let (url, consumed) = match tokens.first()? {
        Token::Str(value) | Token::Url { value, .. } => (value.clone(), 1),
        Token::Function(name) if name.eq_ignore_ascii_case("url") => {
            let end = component_end(tokens, 0)?;
            let [Token::Str(value)] = trim(&tokens[2..end-1]) else { return None; };
            (value.clone(), end)
        }
        _ => return None,
    };
    let mut rest = trim(&tokens[consumed..]);
    let mut layer = None;
    if matches!(rest.first(),Some(Token::Ident(name)) if name.eq_ignore_ascii_case("layer")) {
        layer = Some(ImportLayer::Anonymous); rest = trim(&rest[1..]);
    } else if matches!(rest.first(),Some(Token::Function(name)) if name.eq_ignore_ascii_case("layer")) {
        let end = component_end(rest, 0)?;
        let inner = trim(&rest[2..end-1]);
        let mut path = Vec::new();
        for (i, token) in inner.iter().enumerate() {
            if i%2 == 0 {
                let Token::Ident(name) = token else { return None; };
                if matches!(name.to_ascii_lowercase().as_str(),"initial" | "inherit" | "unset" | "revert" | "revert-layer") { return None; }
                path.push(name.clone());
            } else if token != &Token::Delim('.') { return None; }
        }
        // Modern grammar requires a name in layer(); bare layer is anonymous.
        if path.is_empty() || inner.len()%2 == 0 || path.len()>128 { return None; }
        layer = Some(ImportLayer::Named(path)); rest = trim(&rest[end..]);
    }
    let mut supports = None;
    if matches!(rest.first(),Some(Token::Function(name)) if name.eq_ignore_ascii_case("supports")) {
        let end = component_end(rest, 0)?;
        supports = Some(SupportsCondition::parse_tokens(&rest[2..end-1], true)?);
        rest = trim(&rest[end..]);
    }
    // Media parser owns invalid-query recovery. Reserved misplaced import
    // clauses cannot be interpreted as an alternate import grammar.
    if rest.iter().any(|token| matches!(token,Token::Function(name) if name.eq_ignore_ascii_case("layer") || name.eq_ignore_ascii_case("supports"))) { return None; }
    Some(ImportRule { url, rule_index, layer, supports, media: MediaQueryList::parse(&serialize(rest)) })
}
pub(crate) fn namespace_prelude_valid(tokens: &[Token]) -> bool {
    let mut tokens = trim(tokens);
    if matches!(tokens.first(), Some(Token::Ident(_))) { tokens = trim(&tokens[1..]); }
    parse_import(tokens, 0).is_some_and(|rule|
        rule.layer.is_none() && rule.supports.is_none() && rule.media.css_text().is_empty())
}
impl Stylesheet {
    pub fn imports(&self) -> &[ImportRule] { &self.imports }
    /// Parsed conditional fonts; use the actual owning stylesheet base URL.
    pub fn font_faces(&self, environment: &MediaEnvironment) -> Vec<&FontFaceRule> {
        self.font_faces.iter().filter(|face| matches_media(&face.media, environment)).map(|face| &face.face).collect()
    }
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConditionalFontFace { pub face: FontFaceRule, pub media: Vec<String> }
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedStylesheet {
    pub sheet: Stylesheet,
    /// Child entries keyed by index in sheet.imports(), not rule_index.
    /// Failed/unloaded imports have no child; repeated occurrences stay distinct.
    pub imports: Vec<(usize, LoadedStylesheet)>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedFontFace {
    pub font_face: FontFaceRule,
    pub base_url: Option<String>,
    /// Actual source occurrence transport context, independent of request origin.
    pub resource_context: Option<u64>,
    pub scope_root: Option<amazing_dom::NodeIdx>,
    pub media: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportProjection { pub stylesheets: Vec<Stylesheet>, pub font_faces: Vec<ProjectedFontFace> }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportProjectionError { Cycle(String), DepthLimit, InstanceLimit, InvalidChildIndex(usize), DuplicateChildIndex(usize), InvalidRuleIndex(usize), LayerDepthLimit, AnonymousIdentityExhausted }
static ANONYMOUS: AtomicUsize = AtomicUsize::new(1);
fn anonymous() -> Result<LayerName, ImportProjectionError> {
    ANONYMOUS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
        .map(LayerName::Anonymous).map_err(|_| ImportProjectionError::AnonymousIdentityExhausted)
}
fn matches_media(media: &[String], environment: &MediaEnvironment) -> bool {
    media.iter().all(|query| MediaQueryList::parse(query).evaluate(environment))
}
/// Project actual loaded occurrences in Cascade 5 import order. Reproject for
/// environment changes; no transport or source URL resolution occurs here.
/// Depth128 /4096 active instances and layer depth128 are explicit limits.
pub fn project_imports(root: &LoadedStylesheet, environment: &MediaEnvironment) -> Result<ImportProjection, ImportProjectionError> {
    project_imports_with_media(root,environment,&[])
}
/// Project a loaded sheet under its actual link/style owner's media conditions.
/// Conditions are conjunctive; false/invalid root media produces no occurrences,
/// layer markers or fonts. Reproject the same loaded graph on environment changes.
pub fn project_imports_with_media(root:&LoadedStylesheet,environment:&MediaEnvironment,media:&[String])->Result<ImportProjection,ImportProjectionError> {
    let mut result = ImportProjection::default();
    if !matches_media(media,environment) {return Ok(result);}
    project(root, environment, &[], media, root.sheet.scope_root, &mut Vec::new(), 0, &mut 0, &mut result)?;
    Ok(result)
}
fn project(loaded: &LoadedStylesheet, environment: &MediaEnvironment, prefix: &[LayerName], media: &[String], owner: Option<amazing_dom::NodeIdx>, ancestors: &mut Vec<String>, depth: usize, instances: &mut usize, result: &mut ImportProjection) -> Result<(), ImportProjectionError> {
    if depth>128 { return Err(ImportProjectionError::DepthLimit); }
    *instances += 1;
    if *instances>4096 { return Err(ImportProjectionError::InstanceLimit); }
    if let Some(url) = &loaded.sheet.base_url {
        if ancestors.contains(url) { return Err(ImportProjectionError::Cycle(url.clone())); }
        ancestors.push(url.clone());
    }
    let mut children = HashMap::new();
    for (index, child) in &loaded.imports {
        if *index>=loaded.sheet.imports.len() { return Err(ImportProjectionError::InvalidChildIndex(*index)); }
        if children.insert(*index, child).is_some() { return Err(ImportProjectionError::DuplicateChildIndex(*index)); }
    }
    let mut remapped = HashMap::new();
    let mut start = 0usize;
    for (index, import) in loaded.sheet.imports.iter().enumerate() {
        let end = import.rule_index;
        if end < start || end > loaded.sheet.rules.len() { return Err(ImportProjectionError::InvalidRuleIndex(end)); }
        segment(&loaded.sheet, start, end, prefix, media, owner, false, &mut remapped, environment, result)?;
        start = end;
        if !import.conditions_match(environment) { continue; }
        let mut path = prefix.to_vec();
        if let Some(layer) = &import.layer {
            match layer {
                ImportLayer::Named(names) => path.extend(names.iter().cloned().map(LayerName::Named)),
                ImportLayer::Anonymous => path.push(anonymous()?),
            }
        }
        if path.len()>128 { return Err(ImportProjectionError::LayerDepthLimit); }
        let mut conditions = media.to_vec();
        let css = import.media.css_text();
        if !css.is_empty() { conditions.push(css); }
        if import.layer.is_some() {
            let mut marker = Stylesheet::default();
            marker.scope_root = owner;
            marker.base_url = loaded.sheet.base_url.clone();
            marker.resource_context=loaded.sheet.resource_context;
            marker.projected_layers = true;
            marker.rules.push(Rule { layer: path.clone(), scopes: vec![], media: conditions.clone(), selector_srcs: vec![], declarations: vec![] });
            result.stylesheets.push(marker);
        }
        if let Some(child) = children.get(&index) {
            // Imported sheets share the owner's scoping root, but their actual
            // final URL remains their own. Never fall back to the document URL.
            project(child, environment, &path, &conditions, owner, ancestors, depth+1, instances, result)?;
        }
    }
    segment(&loaded.sheet, start, loaded.sheet.rules.len(), prefix, media, owner, true, &mut remapped, environment, result)?;
    if loaded.sheet.base_url.is_some() { ancestors.pop(); }
    Ok(())
}
fn segment(source: &Stylesheet, start: usize, end: usize, prefix: &[LayerName], media: &[String], owner: Option<amazing_dom::NodeIdx>, final_segment: bool, remapped: &mut HashMap<usize,LayerName>, environment: &MediaEnvironment, result: &mut ImportProjection) -> Result<(), ImportProjectionError> {
    if start==end && !final_segment { return Ok(()); }
    let mut sheet = Stylesheet::default();
    sheet.scope_root = owner; sheet.base_url = source.base_url.clone(); sheet.projected_layers = true;
    sheet.resource_context=source.resource_context;
    for source_rule in &source.rules[start..end] {
        let mut rule = source_rule.clone();
        let mut path = prefix.to_vec();
        for layer in &rule.layer {
            path.push(match layer {
                LayerName::Named(name) => LayerName::Named(name.clone()),
                LayerName::Anonymous(index) => {
                    if !remapped.contains_key(index) { remapped.insert(*index,anonymous()?); }
                    remapped[index].clone()
                }
            });
        }
        if path.len()>128 { return Err(ImportProjectionError::LayerDepthLimit); }
        rule.layer = path;
        let mut conditions = media.to_vec(); conditions.extend(rule.media);
        rule.media = conditions; sheet.rules.push(rule);
    }
    if final_segment {
        for (i,keyframe) in source.keyframes.iter().enumerate() {
            if matches_media(source.keyframe_media.get(i).map(Vec::as_slice).unwrap_or(&[]),environment) {
                sheet.keyframes.push(keyframe.clone());
            }
        }
        for counter in &source.counter_styles {
            let mut counter = counter.clone(); let mut conditions = media.to_vec(); conditions.extend(counter.media);
            counter.media = conditions; sheet.counter_styles.push(counter);
        }
        for face in &source.font_faces {
            let mut conditions = media.to_vec(); conditions.extend(face.media.clone());
            if matches_media(&conditions, environment) {
                sheet.font_faces.push(ConditionalFontFace {face:face.face.clone(),media:conditions.clone()});
                result.font_faces.push(ProjectedFontFace {font_face:face.face.clone(),base_url:source.base_url.clone(),resource_context:source.resource_context,scope_root:owner,media:conditions});
            }
        }
    }
    if !sheet.rules.is_empty() || !sheet.keyframes.is_empty() || !sheet.counter_styles.is_empty() || !sheet.font_faces.is_empty() { result.stylesheets.push(sheet); }
    Ok(())
}
