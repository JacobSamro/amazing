//! CSS Sizing 4 preferred-aspect-ratio property grammar and CSSOM values.
//! Layout owns used sizing and replaced-element intrinsic-ratio selection.
use crate::syntax::{Token, tokenize, serialize};

#[derive(Clone, Debug, PartialEq)]
enum Number { Literal(f64), Math(String) }
impl Number {
    fn specified(&self) -> String {
        match self { Self::Literal(n) => if *n == 0.0 { "0".into() } else { n.to_string() }, Self::Math(s) => s.clone() }
    }
    fn computed(&self) -> String {
        match self {
            Self::Math(s) => crate::math::computed_number(s).map(|n| if n.max(0.0) == 0.0 { "0".into() } else { n.max(0.0).to_string() }).unwrap_or_else(|| s.clone()),
            _ => self.specified(),
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
struct Ratio { auto: bool, numbers: Option<(Number, Number)> }
fn number(tokens: &[Token], pos: &mut usize) -> Option<Number> {
    match tokens.get(*pos)? {
        Token::Number(n) if n.is_finite() && *n >= 0.0 => { *pos += 1; Some(Number::Literal(*n)) }
        Token::Function(_) => {
            let start = *pos; *pos += 1;
            if !matches!(tokens.get(*pos), Some(Token::OpenParen)) { return None; }
            let mut depth = 0usize;
            while let Some(token) = tokens.get(*pos) {
                match token { Token::OpenParen => depth += 1, Token::CloseParen => { depth = depth.checked_sub(1)?; }, _ => {} }
                *pos += 1;
                if depth == 0 { break; }
            }
            if depth != 0 { return None; }
            let source = serialize(&tokens[start..*pos]);
            crate::math::number_percentage(&source, false).then_some(Number::Math(source))
        }
        _ => None,
    }
}
fn parse(css: &str) -> Option<Ratio> {
    let tokens = tokenize(css);
    fn skip(tokens: &[Token], pos: &mut usize) { while matches!(tokens.get(*pos), Some(Token::Whitespace)) { *pos += 1; } }
    let mut pos = 0; let mut auto = false; skip(&tokens, &mut pos);
    if matches!(tokens.get(pos), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto")) { auto = true; pos += 1; skip(&tokens, &mut pos); }
    if pos == tokens.len() { return auto.then_some(Ratio { auto, numbers: None }); }
    let numerator = number(&tokens, &mut pos)?; skip(&tokens, &mut pos);
    let denominator = if matches!(tokens.get(pos), Some(Token::Delim('/'))) { pos += 1; skip(&tokens, &mut pos); number(&tokens, &mut pos)? } else { Number::Literal(1.0) };
    skip(&tokens, &mut pos);
    if matches!(tokens.get(pos), Some(Token::Ident(s)) if s.eq_ignore_ascii_case("auto")) {
        if auto { return None; } auto = true; pos += 1; skip(&tokens, &mut pos);
    }
    (pos == tokens.len()).then_some(Ratio { auto, numbers: Some((numerator, denominator)) })
}
fn canonical(css: &str, computed: bool) -> Option<String> {
    let tokens = tokenize(css);
    let words: Vec<_> = tokens.iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    if let [Token::Ident(s)] = words.as_slice() {
        if matches!(s.to_ascii_lowercase().as_str(), "initial" | "inherit" | "unset" | "revert" | "revert-layer") { return Some(s.to_ascii_lowercase()); }
    }
    let ratio = parse(css)?;
    Some(match ratio.numbers {
        None => "auto".into(),
        Some((n,d)) => format!("{}{} / {}", if ratio.auto { "auto " } else { "" }, if computed { n.computed() } else { n.specified() }, if computed { d.computed() } else { d.specified() }),
    })
}
pub(crate) fn specified(css: &str) -> Option<String> { canonical(css, false) }
pub(crate) fn computed(css: &str) -> Option<String> { canonical(css, true) }

#[cfg(test)]
mod tests {
    use super::*;
    fn style(css: &str) -> crate::ComputedStyle {
        let mut dom = amazing_dom::Dom::new(); let node = dom.create_element(amazing_dom::NodeIdx(0), "div");
        crate::compute(&dom, &[crate::parse_stylesheet(&format!("div{{{css}}}"))]).get(node).unwrap().clone()
    }
    #[test]
    fn every_numeric_and_auto_order_retains_the_pair_without_reducing_it() {
        for n in [0.0,0.25,1.0,1.3,16.0,16300.0] {
            for d in [0.0,0.5,1.0,9.4,1000.0] {
                for input in [format!("{n}/{d}"),format!("auto {n} / {d}"),format!("{n} / {d} auto")] {
                    let expected = format!("{}{n} / {d}", if input.contains("auto") { "auto " } else { "" });
                    assert_eq!(specified(&input).as_deref(),Some(expected.as_str()));
                    assert_eq!(style(&format!("aspect-ratio:{input}")).get("aspect-ratio"),Some(expected.as_str()));
                }
            }
            assert_eq!(computed(&n.to_string()).as_deref(), Some(format!("{n} / 1").as_str()));
        }
    }
    #[test]
    fn invalid_atomic_syntax_keeps_the_previous_declaration() {
        for value in ["", "auto auto", "-1", "16 / -9", "16px / 9px", "16 9", "16 /", "16 / auto", "auto / 5", "auto 16 /", "1 invalid", "invalid 1.5", "auto 1 / 1 auto", "1 / 2 / 3", "'1'", "50%", "calc(1px) / 1", "calc(1) auto auto"] {
            assert_eq!(specified(value), None, "{value}"); assert!(!crate::values::parses("aspect-ratio",value));
            assert_eq!(style(&format!("aspect-ratio:3/2;aspect-ratio:{value}")).get("aspect-ratio"),Some("3 / 2"),"{value}");
        }
    }
    #[test]
    fn syntax_boundaries_and_typed_finite_math_compute_independently() {
        for value in ["AUTO /**/ 16/9", "16/9 a\\75 to"] { assert_eq!(computed(value).as_deref(),Some("auto 16 / 9")); }
        for (value,expected) in [("calc(4 / 2) / 3", "2 / 3"), ("min(4, 2) / clamp(1, 3, 4)", "2 / 3"), ("calc(-2) / 1", "0 / 1")] {
            assert_eq!(style(&format!("aspect-ratio:{value}")).get("aspect-ratio"),Some(expected));
        }
    }
    #[test]
    fn cascade_css_wide_variables_importance_and_real_presentational_hints() {
        let mut dom = amazing_dom::Dom::new(); let parent = dom.create_element(amazing_dom::NodeIdx(0),"section");
        let a = dom.create_element(parent,"div"); let b = dom.create_element(parent,"span");
        let styles=crate::compute(&dom,&[crate::parse_stylesheet("section{aspect-ratio:16/9;--r:4 / 3}div{aspect-ratio:inherit}span{aspect-ratio:var(--r)}")]);
        assert_eq!(styles.get(a).unwrap().get("aspect-ratio"),Some("16 / 9")); assert_eq!(styles.get(b).unwrap().get("aspect-ratio"),Some("4 / 3"));
        dom.set_attrs(a,vec![("data-ratio".into(),"2.5".into())]);
        let styles=crate::compute(&dom,&[crate::parse_stylesheet("div{aspect-ratio:attr(data-ratio type(<number>)) / 2}span{aspect-ratio:attr(missing type(<number>), 4) / 3}")]);
        assert_eq!(styles.get(a).unwrap().get("aspect-ratio"),Some("2.5 / 2")); assert_eq!(styles.get(b).unwrap().get("aspect-ratio"),Some("4 / 3"));
        for value in ["initial", "unset", "var(--missing)", "var(--bad)"] {
            assert_eq!(style(&format!("--bad:1 / auto;aspect-ratio:3/2;aspect-ratio:{value}")).get("aspect-ratio"),Some("auto"));
        }
        assert_eq!(style("aspect-ratio:2!important;aspect-ratio:3").get("aspect-ratio"),Some("2 / 1"));
        assert_eq!(style("aspect-ratio:3;all:initial").get("aspect-ratio"),Some("auto"));
        assert_eq!(crate::classify_style_property("aspect-ratio"),Some(crate::StyleChangeKind::Layout));
        let image=dom.create_element(parent,"img"); dom.set_attrs(image,vec![("width".into(),"160".into()),("height".into(),"90".into())]);
        let styles=crate::compute(&dom,&[]); assert_eq!(styles.get(image).unwrap().get("aspect-ratio"),Some("auto 160 / 90"));
    }
}
