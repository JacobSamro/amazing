//! Group-rule parsing retains cascade context, including empty layer statements.
use super::*;

pub(super) fn parse(
    tokens: &[Token],
    layer: &[LayerName],
    scopes: &[Scope],
    anonymous: &mut usize,
    depth: usize,
) -> Stylesheet {
    let mut sheet = Stylesheet::default();
    if depth > 128 {
        return sheet;
    }
    let mut p = Parser {
        toks: tokens,
        pos: 0,
    };
    while let Some(token) = p.peek() {
        match token {
            Token::Whitespace | Token::Semicolon => p.pos += 1,
            Token::AtKeyword(name) => {
                let name = name.to_ascii_lowercase();
                p.pos += 1;
                let start = p.pos;
                let mut nesting = 0usize;
                while let Some(t) = p.peek() {
                    match t {
                        Token::OpenParen | Token::OpenSquare => nesting += 1,
                        Token::CloseParen | Token::CloseSquare => {
                            nesting = nesting.saturating_sub(1)
                        }
                        Token::OpenCurly | Token::Semicolon if nesting == 0 => break,
                        _ => {}
                    }
                    p.pos += 1;
                }
                let prelude = &tokens[start..p.pos];
                let block = p.peek() == Some(&Token::OpenCurly);
                let body_start = p.pos + 1;
                let end = skip_rule_remainder(tokens, start, true);
                let body_end = if tokens.get(end.wrapping_sub(1)) == Some(&Token::CloseCurly) {
                    end - 1
                } else {
                    end
                };
                p.pos = end;
                let body = if block {
                    &tokens[body_start..body_end]
                } else {
                    &[]
                };
                match name.as_str() {
                    "layer" => {
                        let Some(mut paths) = layer_names(prelude) else {
                            continue;
                        };
                        if paths.is_empty() && block {
                            *anonymous += 1;
                            paths.push(vec![LayerName::Anonymous(*anonymous)]);
                        }
                        if paths.is_empty() || (block && paths.len() != 1) {
                            continue;
                        }
                        for path in paths {
                            let mut full = layer.to_vec();
                            full.extend(path);
                            if full.len() > 128 {
                                continue;
                            }
                            sheet.rules.push(Rule {
                                layer: full.clone(),
                                scopes: scopes.to_vec(),
                                selector_srcs: vec![],
                                declarations: vec![],
                            });
                            if block {
                                let inner = parse(body, &full, scopes, anonymous, depth + 1);
                                sheet.rules.extend(inner.rules);
                                sheet.keyframes.extend(inner.keyframes);
                            }
                        }
                    }
                    "scope" if block => {
                        if let Some(scope) = scope_prelude(prelude, !scopes.is_empty()) {
                            let mut inner = scopes.to_vec();
                            inner.push(scope);
                            let inner_sheet = parse(body, layer, &inner, anonymous, depth + 1);
                            sheet.rules.extend(inner_sheet.rules);
                            sheet.keyframes.extend(inner_sheet.keyframes);
                        }
                    }
                    "supports" if block && supports_condition(prelude) => {
                        let inner = parse(body, layer, scopes, anonymous, depth + 1);
                        sheet.rules.extend(inner.rules);
                        sheet.keyframes.extend(inner.keyframes);
                    }
                    "keyframes" if block => {
                        if let Some(rule) = crate::animation::parse_keyframes(prelude, body) {
                            sheet.keyframes.push(rule);
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                if let Some(mut rule) = p.qualified_rule() {
                    rule.layer = layer.to_vec();
                    rule.scopes = scopes.to_vec();
                    if !rule.selector_srcs.is_empty() && !rule.declarations.is_empty() {
                        sheet.rules.push(rule);
                    }
                } else {
                    break;
                }
            }
        }
    }
    sheet
}

fn layer_names(tokens: &[Token]) -> Option<Vec<Vec<LayerName>>> {
    if tokens.iter().all(|t| matches!(t, Token::Whitespace)) {
        return Some(vec![]);
    }
    tokens
        .split(|t| *t == Token::Comma)
        .map(|part| {
            let start = part.iter().position(|t| !matches!(t, Token::Whitespace))?;
            let end = part.iter().rposition(|t| !matches!(t, Token::Whitespace))? + 1;
            let mut path = vec![];
            let mut ident = true;
            for token in &part[start..end] {
                if ident {
                    let Token::Ident(name) = token else {
                        return None;
                    };
                    // <ident>, not <custom-ident>: `default` is a valid layer name.
                    if matches!(
                        name.to_ascii_lowercase().as_str(),
                        "initial" | "inherit" | "unset" | "revert" | "revert-layer"
                    ) {
                        return None;
                    }
                    path.push(LayerName::Named(name.clone()));
                    if path.len() > 128 {
                        return None;
                    }
                } else if *token != Token::Delim('.') {
                    // Whitespace around a dot is forbidden in a layer name.
                    return None;
                }
                ident = !ident;
            }
            (!ident).then_some(path)
        })
        .collect()
}

fn selector_list(tokens: &[Token]) -> Option<Vec<String>> {
    let mut start = 0;
    let mut depth = 0usize;
    let mut out = Vec::new();
    for i in 0..=tokens.len() {
        if i == tokens.len() || (tokens[i] == Token::Comma && depth == 0) {
            let src = serialize(&tokens[start..i]);
            let check = if src.trim_start().starts_with(['>', '+', '~']) {
                format!(":scope {src}")
            } else {
                src.clone()
            };
            crate::selectors::parse_selector(&check)?;
            out.push(src);
            start = i + 1;
        } else {
            match tokens[i] {
                Token::OpenParen | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => depth = depth.checked_sub(1)?,
                _ => {}
            }
        }
    }
    Some(out)
}

fn scope_prelude(tokens: &[Token], nested: bool) -> Option<Scope> {
    let mut pos = 0;
    skip_whitespace(tokens, &mut pos);
    let start = if tokens.get(pos) == Some(&Token::OpenParen) {
        Some(scope_selectors(tokens, &mut pos)?)
    } else {
        None
    };
    if !nested
        && start.as_ref().is_some_and(|list| {
            list.iter()
                .any(|s| s.trim_start().starts_with(['>', '+', '~']))
        })
    {
        return None;
    }
    skip_whitespace(tokens, &mut pos);
    let end = if matches!(tokens.get(pos), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("to")) {
        pos += 1;
        skip_whitespace(tokens, &mut pos);
        Some(scope_selectors(tokens, &mut pos)?)
    } else {
        None
    };
    skip_whitespace(tokens, &mut pos);
    (pos == tokens.len()).then_some(Scope { start, end })
}

fn scope_selectors(tokens: &[Token], pos: &mut usize) -> Option<Vec<String>> {
    if tokens.get(*pos) != Some(&Token::OpenParen) {
        return None;
    }
    *pos += 1;
    let start = *pos;
    let mut depth = 1usize;
    while let Some(token) = tokens.get(*pos) {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            let selectors = selector_list(&tokens[start..*pos])?;
            *pos += 1;
            return Some(selectors);
        }
        *pos += 1;
    }
    None
}
