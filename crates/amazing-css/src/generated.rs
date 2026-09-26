//! CSS Content 3 generated content and CSS Lists 3 counter evaluation.
use crate::cascade::StyledDom;
use crate::selectors::PseudoElement;
use crate::syntax::{Token, tokenize};
use amazing_dom::{Dom, NodeIdx};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRun {
    Text(String),
    Image(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeneratedContent {
    pub runs: Vec<ContentRun>,
    pub alt_text: Option<String>,
}

impl GeneratedContent {
    pub fn text(&self) -> String {
        self.runs
            .iter()
            .filter_map(|run| match run {
                ContentRun::Text(text) => Some(text.as_str()),
                ContentRun::Image(_) => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Text(String),
    Attr(String, Option<String>),
    Counter(String, String),
    MarkerCounter(String, String),
    Counters(String, String, String),
    Image(String),
    OpenQuote,
    CloseQuote,
    NoOpenQuote,
    NoCloseQuote,
}

#[derive(Debug, Clone, PartialEq)]
struct ContentValue {
    items: Vec<Item>,
    alt: Option<Vec<Item>>,
}

fn compact(tokens: &[Token]) -> Vec<Token> {
    tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .cloned()
        .collect()
}

fn args(tokens: &[Token]) -> Option<Vec<Vec<Token>>> {
    let mut result = vec![Vec::new()];
    let mut depth = 0;
    for token in tokens {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen if depth == 0 => return None,
            Token::CloseParen => depth -= 1,
            Token::Comma if depth == 0 => {
                result.push(Vec::new());
                continue;
            }
            _ => {}
        }
        result.last_mut()?.push(token.clone());
    }
    (depth == 0).then_some(result)
}

fn function_body(tokens: &[Token], start: usize) -> Option<(&[Token], usize)> {
    if !matches!(tokens.get(start + 1), Some(Token::OpenParen)) {
        return None;
    }
    let mut depth = 1;
    for (i, token) in tokens.iter().enumerate().skip(start + 2) {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth -= 1;
                if depth == 0 {
                    return Some((&tokens[start + 2..i], i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn ident(tokens: &[Token]) -> Option<String> {
    match compact(tokens).as_slice() {
        [Token::Ident(s)] => Some(s.clone()),
        _ => None,
    }
}

fn string(tokens: &[Token]) -> Option<String> {
    match compact(tokens).as_slice() {
        [Token::Str(s)] => Some(s.clone()),
        _ => None,
    }
}

fn items(tokens: &[Token], alt: bool) -> Option<Vec<Item>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Whitespace => i += 1,
            Token::Str(s) => {
                out.push(Item::Text(s.clone()));
                i += 1;
            }
            Token::Url { value, .. } if !alt && !value.is_empty() => {
                out.push(Item::Image(value.clone()));
                i += 1;
            }
            Token::Ident(name) if !alt => {
                out.push(match name.to_ascii_lowercase().as_str() {
                    "open-quote" => Item::OpenQuote,
                    "close-quote" => Item::CloseQuote,
                    "no-open-quote" => Item::NoOpenQuote,
                    "no-close-quote" => Item::NoCloseQuote,
                    _ => return None,
                });
                i += 1;
            }
            Token::Function(name) => {
                let (body, end) = function_body(tokens, i)?;
                if !alt && image_function_name(name) {
                    out.push(Item::Image(crate::syntax::serialize(&tokens[i..end])));
                    i = end;
                    continue;
                }
                let params = args(body)?;
                out.push(match name.to_ascii_lowercase().as_str() {
                    "attr" if (1..=2).contains(&params.len()) => Item::Attr(
                        ident(&params[0])?,
                        match params.get(1) {
                            Some(v) => Some(string(v)?),
                            None => None,
                        },
                    ),
                    "counter" if (1..=2).contains(&params.len()) => Item::Counter(
                        ident(&params[0])?,
                        params
                            .get(1)
                            .and_then(|v| ident(v))
                            .unwrap_or_else(|| "decimal".into()),
                    ),
                    "counters" if (2..=3).contains(&params.len()) => Item::Counters(
                        ident(&params[0])?,
                        string(&params[1])?,
                        params
                            .get(2)
                            .and_then(|v| ident(v))
                            .unwrap_or_else(|| "decimal".into()),
                    ),
                    "url" if !alt && params.len() == 1 => {
                        let url = string(&params[0]).unwrap_or_else(|| {
                            crate::syntax::serialize(&params[0]).trim().to_owned()
                        });
                        if url.is_empty() {
                            return None;
                        }
                        Item::Image(url)
                    }
                    _ => return None,
                });
                i = end;
            }
            _ => return None,
        }
    }
    (!out.is_empty()).then_some(out)
}

fn parse_content(css: &str) -> Option<ContentValue> {
    if matches!(css.trim().to_ascii_lowercase().as_str(), "normal" | "none") {
        return Some(ContentValue {
            items: Vec::new(),
            alt: None,
        });
    }
    let tokens = tokenize(css);
    let mut depth = 0;
    let mut slash = None;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen if depth > 0 => depth -= 1,
            Token::Delim('/') if depth == 0 => {
                if slash.is_some() {
                    return None;
                }
                slash = Some(i);
            }
            _ => {}
        }
    }
    let (main, alt) = slash.map_or((&tokens[..], None), |i| {
        (&tokens[..i], Some(&tokens[i + 1..]))
    });
    Some(ContentValue {
        items: items(main, false)?,
        alt: match alt {
            Some(v) => Some(items(v, true)?),
            None => None,
        },
    })
}

/// A list marker accepts a CSS `<image>` value, not just `url()`.
/// Keep non-URL images as CSS text for the generated-image consumer.
pub(crate) fn image_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "linear-gradient" | "repeating-linear-gradient" | "radial-gradient"
            | "repeating-radial-gradient" | "conic-gradient" | "repeating-conic-gradient"
            | "image" | "image-set" | "cross-fade" | "element" | "paint"
    )
}

fn list_style_image(css: &str, base: Option<&str>) -> Option<Item> {
    let tokens = tokenize(css);
    let significant = compact(&tokens);
    if let [Token::Url { value, .. }] = significant.as_slice() {
        return (!value.is_empty()).then(|| Item::Image(resolve_image_url(value, base)));
    }
    let Token::Function(name) = significant.first()? else { return None; };
    let (body, end) = function_body(&significant, 0)?;
    if end != significant.len() || body.is_empty() { return None; }
    if name.eq_ignore_ascii_case("url") {
        let parsed = parse_content(css)?;
        let [Item::Image(url)] = parsed.items.as_slice() else { return None; };
        return Some(Item::Image(resolve_image_url(url, base)));
    }
    image_function_name(name)
        .then(|| Item::Image(crate::syntax::serialize(&tokens).trim().to_owned()))
}

/// Resolve a CSS URL against the stylesheet that supplied its declaration.
/// Absolute URLs already carry their own base; fragment and query references
/// retain the stylesheet path.
fn resolve_image_url(url: &str, base: Option<&str>) -> String {
    let url = url.trim();
    if url
        .bytes()
        .take_while(|byte| *byte != b':')
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        && url
            .find(':')
            .is_some_and(|colon| colon > 0 && !url[..colon].contains('/'))
    {
        return url.to_owned();
    }
    let Some(base) = base else {
        return url.to_owned();
    };
    let Some((scheme, rest)) = base.split_once("://") else {
        return url.to_owned();
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.is_empty() {
        return url.to_owned();
    }
    if url.starts_with("//") {
        return format!("{scheme}:{url}");
    }
    let base_path = format!("/{path}");
    let target = if url.starts_with('/') {
        url.to_owned()
    } else if url.starts_with('?') {
        format!(
            "{}{}",
            base_path.split(['?', '#']).next().unwrap_or("/"),
            url
        )
    } else if url.starts_with('#') {
        format!("{}{}", base_path.split('#').next().unwrap_or("/"), url)
    } else {
        let clean = base_path.split(['?', '#']).next().unwrap_or("/");
        format!("{}{}", &clean[..clean.rfind('/').unwrap_or(0) + 1], url)
    };
    let split = target.find(['?', '#']).unwrap_or(target.len());
    let (path, suffix) = target.split_at(split);
    let mut parts = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(segment),
        }
    }
    let trailing = path.ends_with('/') || path.ends_with("/.") || path.ends_with("/..");
    let normalized = format!(
        "/{}{}",
        parts.join("/"),
        if trailing && !parts.is_empty() {
            "/"
        } else {
            ""
        }
    );
    format!("{scheme}://{authority}{normalized}{suffix}")
}

fn parse_counter_ops(css: &str, default: i32, allow_reversed: bool) -> Option<Vec<(String, i32, bool)>> {
    let tokens = compact(&tokenize(css));
    if tokens == [Token::Ident("none".into())] {
        return Some(Vec::new());
    }
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let name = match &tokens[i] {
            Token::Ident(name) => name.clone(),
            Token::Function(function)
                if allow_reversed && function.eq_ignore_ascii_case("reversed")
                    && matches!(tokens.get(i + 1), Some(Token::OpenParen)) =>
            {
                let Some(Token::Ident(name)) = tokens.get(i + 2) else {
                    return None;
                };
                if !matches!(tokens.get(i + 3), Some(Token::CloseParen)) {
                    return None;
                }
                i += 3;
                format!("reversed({name})")
            }
            _ => return None,
        };
        if name.eq_ignore_ascii_case("none") {
            return None;
        }
        let (value, explicit) = match tokens.get(i + 1) {
            Some(Token::Number(n))
                if n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 =>
            {
                i += 1;
                (*n as i32, true)
            }
            _ => (default, false),
        };
        result.push((name, value, explicit));
        i += 1;
    }
    (!result.is_empty()).then_some(result)
}

fn quote_pairs(css: &str) -> Option<Vec<(String, String)>> {
    let lower = css.trim().to_ascii_lowercase();
    if lower == "none" {
        return Some(Vec::new());
    }
    if lower == "auto" {
        return Some(vec![("“".into(), "”".into()), ("‘".into(), "’".into())]);
    }
    let strings: Vec<_> = compact(&tokenize(css))
        .into_iter()
        .map(|t| match t {
            Token::Str(s) => Some(s),
            _ => None,
        })
        .collect::<Option<_>>()?;
    if strings.is_empty() || strings.len() % 2 != 0 {
        return None;
    }
    Some(
        strings
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect(),
    )
}

#[derive(Default)]
struct State {
    counters: HashMap<String, Vec<i32>>,
    reversed: HashMap<String, Vec<bool>>,
    counter_styles: crate::counter_styles::CounterStyles,
    quote_depth: usize,
}

// A popped reset can leave an empty entry in the map. Presence in the map is
// not the same as having a counter in scope. Implicitly instantiated counters
// remain available to following siblings, unlike explicit resets.
fn ensure_counter(state: &mut State, name: &str) {
    let values = state.counters.entry(name.to_owned()).or_default();
    if values.is_empty() {
        values.push(0);
        state.reversed.entry(name.to_owned()).or_default().push(false);
    }
}

fn reset_names_counter(style: &crate::ComputedStyle, name: &str) -> bool {
    parse_counter_ops(style.get("counter-reset").unwrap_or("none"), 0, true)
        .unwrap_or_default()
        .iter()
        .any(|(raw, _, _)| raw == name || raw == &format!("reversed({name})"))
}

/// The implicit start of a reversed counter is determined by the increments
/// in its scope, stopping at the first counter-set. A nested reset obscures
/// this scope for its subtree.
fn reversed_initial(dom: &Dom, node: NodeIdx, name: &str, styles: &StyledDom) -> i32 {
    fn scan_style(
        style: &crate::ComputedStyle,
        name: &str,
        reversed: bool,
        total: &mut i32,
        last_step: &mut i32,
    ) -> bool {
        let increments = parse_counter_ops(
            style.get("counter-increment").unwrap_or("none"), 1, false
        ).unwrap_or_default();
        let specified = increments.iter().any(|(counter, _, _)| counter == name);
        let mut increment = increments.iter().filter(|(counter, _, _)| counter == name)
            .fold(0_i32, |sum, (_, value, _)| sum.saturating_add(*value));
        if name == "list-item" && style.get("display") == Some("list-item") && !specified {
            increment = if reversed { -1 } else { 1 };
        }
        let negated = increment.saturating_neg();
        if negated != 0 { *last_step = negated; }
        if let Some((_, value, _)) = parse_counter_ops(
            style.get("counter-set").unwrap_or("none"), 0, false
        ).unwrap_or_default().iter().rev().find(|(counter, _, _)| counter == name) {
            *total = total.saturating_add(*value);
            return true;
        }
        *total = total.saturating_add(negated);
        false
    }
    fn scan_node(
        dom: &Dom, node: NodeIdx, root: bool, name: &str, styles: &StyledDom,
        total: &mut i32, last_step: &mut i32,
    ) -> bool {
        let Some(style) = styles.get(node) else { return false };
        if style.get("display") == Some("none") { return false; }
        if !root && reset_names_counter(style, name) { return false; }
        if scan_style(style, name, true, total, last_step) { return true; }
        for pseudo in [PseudoElement::Before, PseudoElement::Marker] {
            if let Some(pseudo_style) = styles.get_pseudo(node, pseudo) {
                if scan_style(pseudo_style, name, true, total, last_step) { return true; }
            }
        }
        for child in dom.children(node) {
            if scan_node(dom, child, false, name, styles, total, last_step) { return true; }
        }
        if let Some(after) = styles.get_pseudo(node, PseudoElement::After) {
            if scan_style(after, name, true, total, last_step) { return true; }
        }
        false
    }
    let mut total = 0_i32;
    let mut last_step = 0_i32;
    scan_node(dom, node, true, name, styles, &mut total, &mut last_step);
    total.saturating_add(last_step)
}

fn evaluate(
    items: &[Item],
    dom: &Dom,
    node: NodeIdx,
    style: &crate::ComputedStyle,
    state: &mut State,
) -> Vec<ContentRun> {
    let quotes = quote_pairs(style.get("quotes").unwrap_or("auto")).unwrap_or_default();
    let mut runs = Vec::new();
    for item in items {
        let text = match item {
            Item::Text(s) => s.clone(),
            Item::Attr(name, fallback) => dom
                .attrs_of(node)
                .and_then(|attrs| attrs.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)))
                .map(|(_, value)| value.clone())
                .or_else(|| fallback.clone())
                .unwrap_or_default(),
            Item::Counter(name, style) => {
                ensure_counter(state, name);
                let number = state.counters.get(name).and_then(|v| v.last()).copied().unwrap_or(0);
                state.counter_styles.format(number, style)
            }
            Item::MarkerCounter(name, style) => {
                ensure_counter(state, name);
                let number = state.counters.get(name).and_then(|v| v.last()).copied().unwrap_or(0);
                state.counter_styles.marker(number, style)
            }
            Item::Counters(name, separator, style) => {
                ensure_counter(state, name);
                state.counters.get(name).filter(|values| !values.is_empty()).map(|values| {
                    values
                        .iter()
                        .map(|value| state.counter_styles.format(*value, style))
                        .collect::<Vec<_>>()
                        .join(separator)
                })
                .unwrap_or_else(|| "0".into())
            }
            Item::Image(url) => {
                runs.push(ContentRun::Image(url.clone()));
                continue;
            }
            Item::OpenQuote | Item::NoOpenQuote => {
                let text = if matches!(item, Item::OpenQuote) {
                    quotes
                        .get(state.quote_depth.min(quotes.len().saturating_sub(1)))
                        .map(|pair| pair.0.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                state.quote_depth += 1;
                text
            }
            Item::CloseQuote | Item::NoCloseQuote => {
                state.quote_depth = state.quote_depth.saturating_sub(1);
                if matches!(item, Item::CloseQuote) {
                    quotes
                        .get(state.quote_depth.min(quotes.len().saturating_sub(1)))
                        .map(|pair| pair.1.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
        };
        runs.push(ContentRun::Text(text));
    }
    runs
}

fn apply_counters(
    dom: &Dom,
    node: NodeIdx,
    style: &crate::ComputedStyle,
    state: &mut State,
    styles: &StyledDom,
) -> Vec<String> {
    let mut pushed = Vec::new();
    for (raw_name, mut value, explicit) in
        parse_counter_ops(style.get("counter-reset").unwrap_or("none"), 0, true).unwrap_or_default()
    {
        let (name, reversed) = if let Some(name) = raw_name
            .strip_prefix("reversed(")
            .and_then(|s| s.strip_suffix(')'))
        {
            (name.to_owned(), true)
        } else {
            (raw_name, false)
        };
        if reversed && !explicit {
            value = reversed_initial(dom, node, &name, styles);
        }
        state.counters.entry(name.clone()).or_default().push(value);
        state
            .reversed
            .entry(name.clone())
            .or_default()
            .push(reversed);
        pushed.push(name);
    }
    let mut increments =
        parse_counter_ops(style.get("counter-increment").unwrap_or("none"), 1, false).unwrap_or_default();
    if style.get("display") == Some("list-item")
        && !increments.iter().any(|(name, _, _)| name == "list-item")
    {
        let step = if state.reversed.get("list-item").and_then(|v| v.last()) == Some(&true) {
            -1
        } else {
            1
        };
        increments.push(("list-item".into(), step, false));
    }
    for (name, value, _) in increments {
        ensure_counter(state, &name);
        if let Some(current) = state.counters.get_mut(&name).and_then(|values| values.last_mut()) {
            *current = current.saturating_add(value);
        }
    }
    for (name, value, _) in
        parse_counter_ops(style.get("counter-set").unwrap_or("none"), 0, false).unwrap_or_default()
    {
        ensure_counter(state, &name);
        if let Some(current) = state.counters.get_mut(&name).and_then(|values| values.last_mut()) {
            *current = value;
        }
    }
    pushed
}

fn pop_counters(pushed: Vec<String>, state: &mut State) {
    for name in pushed.into_iter().rev() {
        if let Some(values) = state.counters.get_mut(&name) {
            values.pop();
        }
        if let Some(values) = state.reversed.get_mut(&name) {
            values.pop();
        }
    }
}

fn resolve_pseudo(
    dom: &Dom,
    node: NodeIdx,
    pseudo: PseudoElement,
    out: &mut StyledDom,
    state: &mut State,
) {
    let Some(style) = out.get_pseudo(node, pseudo) else {
        return;
    };
    let css = style.get("content").unwrap_or("normal");
    let mut value = if css == "normal" && pseudo == PseudoElement::Marker {
        let image = style.get("list-style-image").unwrap_or("none");
        let image_item = list_style_image(image, style.url_base("list-style-image"));
        let list_style = style.get("list-style-type").unwrap_or("disc");
        let string_marker = match compact(&tokenize(list_style)).as_slice() {
            [Token::Str(text)] => Some(text.clone()),
            _ => None,
        };
        let items = image_item.map(|image| vec![image]).unwrap_or_else(|| {
            if let Some(text) = string_marker {
                return vec![Item::Text(text)];
            }
            match list_style {
                "disc" => vec![Item::Text("• ".into())],
                "circle" => vec![Item::Text("◦ ".into())],
                "square" => vec![Item::Text("▪ ".into())],
                "none" => Vec::new(),
                counter_style => vec![Item::MarkerCounter("list-item".into(), counter_style.into())],
            }
        });
        ContentValue { items, alt: None }
    } else {
        match parse_content(css) {
            Some(value) => value,
            None => return,
        }
    };
    for item in &mut value.items {
        if let Item::Image(url) = item {
            *url = resolve_image_url(url, style.url_base("content"));
        }
    }
    let pushed = apply_counters(dom, node, style, state, out);
    let mut content = GeneratedContent {
        runs: evaluate(&value.items, dom, node, style, state),
        alt_text: None,
    };
    content.alt_text = value.alt.map(|items| {
        evaluate(&items, dom, node, style, state)
            .into_iter()
            .filter_map(|run| match run {
                ContentRun::Text(text) => Some(text),
                _ => None,
            })
            .collect()
    });
    out.generated.insert((node.0, pseudo), content);
    pop_counters(pushed, state);
}

fn walk(dom: &Dom, node: NodeIdx, out: &mut StyledDom, state: &mut State) {
    if let Some(style) = out.get(node).cloned() {
        if style.get("display") == Some("none") {
            return;
        }
        let pushed = apply_counters(dom, node, &style, state, out);
        resolve_pseudo(dom, node, PseudoElement::Before, out, state);
        resolve_pseudo(dom, node, PseudoElement::Marker, out, state);
        for child in dom.children(node) {
            walk(dom, child, out, state);
        }
        resolve_pseudo(dom, node, PseudoElement::After, out, state);
        pop_counters(pushed, state);
    } else {
        for child in dom.children(node) {
            walk(dom, child, out, state);
        }
    }
}

pub(crate) fn resolve_document(
    dom: &Dom,
    out: &mut StyledDom,
    sheets: &[crate::Stylesheet],
    environment: &crate::MediaEnvironment,
) {
    let mut state = State {
        counter_styles: crate::counter_styles::CounterStyles::new(sheets, environment),
        ..Default::default()
    };
    walk(dom, NodeIdx(0), out, &mut state);
}

pub(crate) fn valid_property(property: &str, css: &str) -> bool {
    match property {
        "content" => parse_content(css).is_some(),
        "counter-reset" => parse_counter_ops(css, 0, true).is_some(),
        "counter-set" => parse_counter_ops(css, 0, false).is_some(),
        "counter-increment" => parse_counter_ops(css, 1, false).is_some(),
        "quotes" => quote_pairs(css).is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_content_counter_and_quotes_values() {
        for value in [
            "counter(x, decimal, extra)",
            "url()",
            "'x' /",
            "open-quote trailing",
        ] {
            assert!(!valid_property("content", value), "{value}");
        }
        assert!(!valid_property("counter-increment", "x 1.5"));
        assert!(!valid_property("counter-increment", "reversed(x)"));
        assert!(!valid_property("quotes", "'unpaired'"));
        assert!(valid_property("counter-reset", "reversed(list-item) 5"));
    }

    #[test]
    fn content_accepts_image_functions_as_image_runs() {
        for image in [
            "linear-gradient(red, blue)",
            "radial-gradient(circle, red, blue)",
            "conic-gradient(red, blue)",
            "repeating-linear-gradient(red, blue)",
            "image-set(url(a.png) 1x, url(b.png) 2x)",
            "cross-fade(url(a.png), url(b.png), 50%)",
        ] {
            let parsed = parse_content(image).expect(image);
            let canonical = crate::syntax::serialize(&tokenize(image));
            assert!(matches!(parsed.items.as_slice(), [Item::Image(value)] if value == &canonical), "{image}");
        }
    }
}
