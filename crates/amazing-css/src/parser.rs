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
//! - `@supports` evaluates the static variable-bearing declaration conditions
//!   used by the computed-value gate; other at-rules are consumed and dropped
//! - nested style rules remain deferred
//! - declaration values stay normalized strings; the private computed-value
//!   module tokenizes them for custom-property substitution and defaulting

use crate::syntax::{tokenize, serialize, Token};

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
    /// One entry per top-level comma-separated selector (trimmed).
    pub selector_srcs: Vec<String>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

/// Parses a full stylesheet. Invalid constructs are dropped with recovery.
pub fn parse_stylesheet(src: &str) -> Stylesheet {
    let toks = tokenize(src);
    parse_rule_list(&toks)
}

fn parse_rule_list(tokens: &[Token]) -> Stylesheet {
    let mut parser = Parser { toks: tokens, pos: 0 };
    parser.rule_list()
}

/// Parses a standalone declaration block — the contents of an inline
/// `style=""` attribute. Runs to EOF; malformed declarations are dropped
/// with the same recovery rules as stylesheet blocks.
pub fn parse_declarations(src: &str) -> Vec<Declaration> {
    let toks = tokenize(src);
    let mut p = Parser { toks: &toks, pos: 0 };
    p.block_declarations()
}

struct Parser<'t> {
    toks: &'t [Token],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn rule_list(&mut self) -> Stylesheet {
        let mut sheet = Stylesheet::default();
        while let Some(token) = self.peek() {
            match token {
                Token::Whitespace | Token::Semicolon => self.pos += 1,
                Token::AtKeyword(name) if name.eq_ignore_ascii_case("supports") => {
                    sheet.rules.extend(self.supports_rule());
                },
                Token::AtKeyword(_) => self.skip_at_rule(),
                _ => {
                    if let Some(rule) = self.qualified_rule()
                        && !rule.selector_srcs.is_empty()
                        && !rule.declarations.is_empty()
                    {
                        sheet.rules.push(rule);
                    }
                },
            }
        }
        sheet
    }

    fn supports_rule(&mut self) -> Vec<Rule> {
        debug_assert!(matches!(self.peek(), Some(Token::AtKeyword(name)) if name.eq_ignore_ascii_case("supports")));
        self.pos += 1;
        let prelude_start = self.pos;
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            match token {
                Token::OpenParen | Token::OpenSquare => depth += 1,
                Token::CloseParen | Token::CloseSquare => depth = depth.saturating_sub(1),
                Token::OpenCurly if depth == 0 => break,
                Token::Semicolon if depth == 0 => {
                    self.pos += 1;
                    return Vec::new();
                },
                _ => {},
            }
            self.pos += 1;
        }
        if self.peek() != Some(&Token::OpenCurly) {
            return Vec::new();
        }
        let prelude_end = self.pos;
        self.pos += 1;
        let body_start = self.pos;
        let mut braces = 1usize;
        while let Some(token) = self.peek() {
            match token {
                Token::OpenCurly => braces += 1,
                Token::CloseCurly => {
                    braces -= 1;
                    if braces == 0 {
                        let body_end = self.pos;
                        self.pos += 1;
                        return if supports_condition(&self.toks[prelude_start..prelude_end]) {
                            parse_rule_list(&self.toks[body_start..body_end]).rules
                        } else {
                            Vec::new()
                        };
                    }
                },
                _ => {},
            }
            self.pos += 1;
        }
        Vec::new()
    }

    /// Skips an at-rule: through the next ';' or its balanced {} block.
    fn skip_at_rule(&mut self) {
        debug_assert!(matches!(self.peek(), Some(Token::AtKeyword(_))));
        self.pos += 1;
        let mut depth = 0usize;
        loop {
            match self.peek() {
                None => return,
                Some(Token::Semicolon) if depth == 0 => {
                    self.pos += 1;
                    return;
                },
                Some(Token::OpenCurly) => depth += 1,
                Some(Token::CloseCurly) => {
                    self.pos += 1;
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                    continue;
                },
                _ => {},
            }
            self.pos += 1;
        }
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
                _ => {},
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
                    let seg = serialize(&prelude_toks[current_start..i]);
                    if !seg.trim().is_empty() {
                        selector_srcs.push(seg);
                    }
                    current_start = i + 1;
                },
                _ => {},
            }
        }
        if current_start < prelude_toks.len() {
            let seg = serialize(&prelude_toks[current_start..]);
            if !seg.trim().is_empty() {
                selector_srcs.push(seg);
            }
        }

        Some(Rule { selector_srcs, declarations })
    }

    /// Reads declarations until the block's closing '}' (consumed) or EOF.
    fn block_declarations(&mut self) -> Vec<Declaration> {
        let mut out = Vec::new();
        let mut current: Vec<Token> = Vec::new();
        let mut depth = 0usize;
        loop {
            match self.peek().cloned() {
                None => {
                    out.extend(self.finish_declaration(&current));
                    return out;
                },
                // Parens belong to the value (rgba(…), var(…)); track nesting
                // but keep the tokens so serialization stays faithful.
                Some(tok @ (Token::OpenParen | Token::OpenSquare)) => {
                    depth += 1;
                    current.push(tok);
                    self.pos += 1;
                },
                Some(tok @ (Token::CloseParen | Token::CloseSquare)) => {
                    depth = depth.saturating_sub(1);
                    current.push(tok);
                    self.pos += 1;
                },
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
                            _ => {},
                        }
                        self.pos += 1;
                    }
                    return out;
                },
                Some(Token::CloseCurly) => {
                    self.pos += 1;
                    if depth == 0 {
                        out.extend(self.finish_declaration(&current));
                        return out;
                    }
                    depth -= 1; // stray closer inside parens — tolerate
                },
                Some(Token::Semicolon) if depth == 0 => {
                    self.pos += 1;
                    out.extend(self.finish_declaration(&current));
                    current.clear();
                },
                // Whitespace between declarations is not part of the next
                // declaration's fragment; inside values/parens it is kept
                // (serialize collapses runs to single spaces).
                Some(Token::Whitespace) if depth == 0 && current.is_empty() => {
                    self.pos += 1;
                },
                Some(tok) => {
                    current.push(tok);
                    self.pos += 1;
                },
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
        if start >= end {
            return None; // empty value
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

        if start >= end {
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
        if value.is_empty() {
            return None;
        }
        Some(Declaration { property, value, important })
    }
}

fn supports_condition(tokens: &[Token]) -> bool {
    let tokens = trim_ws(tokens);
    if tokens.is_empty() {
        return false;
    }
    if matches!(tokens.first(), Some(Token::Ident(name)) if name.eq_ignore_ascii_case("not")) {
        return !supports_condition(trim_ws(&tokens[1..]));
    }

    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    let mut operator: Option<bool> = None; // true = and, false = or
    let mut segments = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth > 0 => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Ident(name) if paren_depth == 0 && square_depth == 0 => {
                let current = if name.eq_ignore_ascii_case("and") {
                    Some(true)
                } else if name.eq_ignore_ascii_case("or") {
                    Some(false)
                } else {
                    None
                };
                if let Some(current) = current {
                    if operator.is_some_and(|operator| operator != current) {
                        return false;
                    }
                    operator = Some(current);
                    segments.push(&tokens[start..index]);
                    start = index + 1;
                }
            },
            _ => {},
        }
    }
    if let Some(and) = operator {
        segments.push(&tokens[start..]);
        return if and {
            segments.into_iter().all(supports_condition)
        } else {
            segments.into_iter().any(supports_condition)
        };
    }

    let Some(inner) = strip_outer_parentheses(tokens) else {
        return false;
    };
    if let Some(colon) = top_level_colon(inner) {
        return supports_declaration(&inner[..colon], &inner[colon + 1..]);
    }
    supports_condition(inner)
}

fn supports_declaration(name_tokens: &[Token], value_tokens: &[Token]) -> bool {
    let [Token::Ident(name)] = trim_ws(name_tokens) else {
        return false;
    };
    let value_tokens = trim_ws(value_tokens);
    if value_tokens.is_empty() || !crate::computed::valid_variable_syntax(value_tokens) {
        return false;
    }
    if name.starts_with("--") {
        return crate::computed::valid_custom_property_value(value_tokens);
    }
    crate::computed::is_supported_property(name)
        && value_tokens.iter().any(
            |token| matches!(token, Token::Function(function) if function.eq_ignore_ascii_case("var")),
        )
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

fn strip_outer_parentheses(tokens: &[Token]) -> Option<&[Token]> {
    if tokens.first() != Some(&Token::OpenParen) {
        return None;
    }
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => depth += 1,
            Token::CloseParen => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return (index == tokens.len() - 1).then_some(&tokens[1..index]);
                }
            },
            _ => {},
        }
    }
    None
}

fn top_level_colon(tokens: &[Token]) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth > 0 => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Colon if paren_depth == 0 && square_depth == 0 => return Some(index),
            _ => {},
        }
    }
    None
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
        assert_eq!(
            sheet.rules[0].selector_srcs,
            vec!["a", ".b > c"]
        );
    }

    #[test]
    fn important_flag_case_insensitive() {
        assert_eq!(decls("p{margin:0 !IMPORTANT}"), vec![("margin".into(), "0".into(), true)]);
        assert_eq!(decls("p{margin:0!important}"), vec![("margin".into(), "0".into(), true)]);
        assert_eq!(decls("p{margin:0}"), vec![("margin".into(), "0".into(), false)]);
    }

    #[test]
    fn invalid_declarations_dropped_with_recovery() {
        // empty value + missing colon both dropped; good one survives
        let d = decls("p{ color : ; color red ; color: green }");
        assert_eq!(d, vec![("color".into(), "green".into(), false)]);
    }

    #[test]
    fn at_rules_skipped_including_blocks() {
        let sheet = parse_stylesheet(
            "@import url(x.css); @media screen { p { color: red } } div { x:y }",
        );
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector_srcs, vec!["div"]);
        assert_eq!(sheet.rules[0].declarations[0].value, "y");
    }

    #[test]
    fn function_values_keep_function_syntax() {
        let d = decls("p{ background: rgba(255, 0, 0, 0.5) }");
        assert_eq!(
            d,
            vec![("background".into(), "rgba(255,0,0,.5)".into(), false)]
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
        let declarations = decls(
            "p{--a:green;--a:var(--b,!);--b:blue;--b:var(--c,;);color:var(--a,(;))}",
        );
        assert_eq!(
            declarations,
            vec![
                ("--a".into(), "green".into(), false),
                ("--b".into(), "blue".into(), false),
                ("color".into(), "var(--a,(;))".into(), false),
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
}
