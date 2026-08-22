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
//! - at-rules (@media/@import/…) are consumed and DISCARDED (their blocks
//!   still parse correctly as far as brace matching goes) — conditional
//!   rule processing is a later-phase feature
//! - nested rules / style attributes / custom properties not handled
//! - declaration values stay strings; property-specific value parsing lands
//!   when layout needs it

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
    let mut p = Parser { toks: &toks, pos: 0 };
    let mut sheet = Stylesheet::default();
    while let Some(tok) = p.peek() {
        match tok {
            Token::Whitespace | Token::Semicolon => {
                p.pos += 1;
            },
            Token::AtKeyword(_) => p.skip_at_rule(),
            _ => {
                if let Some(rule) = p.qualified_rule() {
                    if !rule.selector_srcs.is_empty() && !rule.declarations.is_empty() {
                        sheet.rules.push(rule);
                    }
                }
            },
        }
    }
    sheet
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
        if scan > start {
            if let Some(Token::Ident(name)) = toks.get(scan - 1) {
                if name.eq_ignore_ascii_case("important") {
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
            }
        }

        if start >= end {
            return None;
        }
        let value = serialize(&toks[start..end]);
        if value.is_empty() {
            return None;
        }
        Some(Declaration { property, value, important })
    }
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
}
