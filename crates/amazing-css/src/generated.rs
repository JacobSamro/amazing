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

fn parse_counter_ops(css: &str, default: i32, allow_reversed: bool) -> Option<Vec<(String, i32)>> {
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
        let value = match tokens.get(i + 1) {
            Some(Token::Number(n))
                if n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 =>
            {
                i += 1;
                *n as i32
            }
            _ => default,
        };
        result.push((name, value));
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

fn format_counter(n: i32, style: &str) -> String {
    match style.to_ascii_lowercase().as_str() {
        "decimal-leading-zero" if (0..10).contains(&n) => format!("0{n}"),
        "lower-roman" | "upper-roman" if (1..4000).contains(&n) => {
            let mut number = n;
            let mut result = String::new();
            for (value, numeral) in [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ] {
                while number >= value {
                    result.push_str(numeral);
                    number -= value;
                }
            }
            if style.starts_with("upper") {
                result.to_ascii_uppercase()
            } else {
                result
            }
        }
        "lower-alpha" | "lower-latin" | "upper-alpha" | "upper-latin" if n > 0 => {
            let mut n = n as u32;
            let mut letters = String::new();
            while n > 0 {
                n -= 1;
                letters.insert(0, char::from(b'a' + (n % 26) as u8));
                n /= 26;
            }
            if style.starts_with("upper") {
                letters.to_ascii_uppercase()
            } else {
                letters
            }
        }
        _ => n.to_string(),
    }
}

#[derive(Default)]
struct State {
    counters: HashMap<String, Vec<i32>>,
    reversed: HashMap<String, Vec<bool>>,
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
                format_counter(number, style)
            }
            Item::Counters(name, separator, style) => {
                ensure_counter(state, name);
                state.counters.get(name).filter(|values| !values.is_empty()).map(|values| {
                    values
                        .iter()
                        .map(|value| format_counter(*value, style))
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
) -> Vec<String> {
    let mut pushed = Vec::new();
    for (raw_name, mut value) in
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
        if reversed && value == 0 && dom.name(node) == Some("ol") {
            value = dom
                .children(node)
                .filter(|child| dom.name(*child) == Some("li"))
                .count() as i32
                + 1;
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
        && !increments.iter().any(|(name, _)| name == "list-item")
    {
        let step = if state.reversed.get("list-item").and_then(|v| v.last()) == Some(&true) {
            -1
        } else {
            1
        };
        increments.push(("list-item".into(), step));
    }
    for (name, value) in increments {
        ensure_counter(state, &name);
        if let Some(current) = state.counters.get_mut(&name).and_then(|values| values.last_mut()) {
            *current = current.saturating_add(value);
        }
    }
    for (name, value) in
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
        let image_item = parse_content(image).and_then(|value| match value.items.as_slice() {
            [Item::Image(url)] => Some(Item::Image(resolve_image_url(
                url,
                style.url_base("list-style-image"),
            ))),
            _ => None,
        });
        let list_style = style.get("list-style-type").unwrap_or("disc");
        let items = image_item
            .map(|image| vec![image])
            .unwrap_or_else(|| match list_style {
                "disc" => vec![Item::Text("• ".into())],
                "circle" => vec![Item::Text("◦ ".into())],
                "square" => vec![Item::Text("▪ ".into())],
                "none" => Vec::new(),
                counter_style => vec![
                    Item::Counter("list-item".into(), counter_style.into()),
                    Item::Text(". ".into()),
                ],
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
    let pushed = apply_counters(dom, node, style, state);
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
        let pushed = apply_counters(dom, node, &style, state);
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

pub(crate) fn resolve_document(dom: &Dom, out: &mut StyledDom) {
    walk(dom, NodeIdx(0), out, &mut State::default());
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
}
