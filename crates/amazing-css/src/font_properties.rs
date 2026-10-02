//! CSS Fonts 4 longhand grammars and the font-synthesis shorthand.
use crate::syntax::{serialize, tokenize, Token};

pub(crate) fn is_audited_longhand(name: &str) -> bool {
    matches!(
        name,
        "font-kerning"
            | "font-language-override"
            | "font-optical-sizing"
            | "font-palette"
            | "font-size-adjust"
            | "font-variant-alternates"
            | "font-variant-caps"
            | "font-variant-east-asian"
            | "font-variant-emoji"
            | "font-variant-numeric"
            | "font-variant-position"
            | "font-variation-settings"
            | "font-synthesis-weight"
            | "font-synthesis-style"
            | "font-synthesis-small-caps"
            | "font-synthesis-position"
    )
}

pub(crate) fn canonical(name: &str, css: &str) -> Option<String> {
    let tokens = tokenize(css);
    let t: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let idents: Option<Vec<_>> = t
        .iter()
        .map(|t| match t {
            Token::Ident(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let single = match t.as_slice() {
        [Token::Ident(s)] => Some(s.to_ascii_lowercase()),
        _ => None,
    };
    let keyword = |allowed: &str| {
        single
            .as_deref()
            .filter(|s| allowed.split(' ').any(|k| k == *s))
            .map(str::to_owned)
    };
    match name {
        "font-kerning" => keyword("auto normal none"),
        "font-optical-sizing" => keyword("auto none"),
        "font-synthesis-style" => keyword("auto none oblique-only"),
        "font-synthesis-weight"
        | "font-synthesis-small-caps"
        | "font-synthesis-position" => keyword("auto none"),
        "font-variant-caps" => keyword(
            "normal small-caps all-small-caps petite-caps all-petite-caps unicase titling-caps",
        ),
        "font-variant-position" => keyword("normal sub super"),
        "font-variant-emoji" => keyword("normal text emoji unicode"),
        "font-language-override" => keyword("normal").or_else(|| match t.as_slice() {
            [Token::Str(s)] => Some(format!("\"{}\"", s.replace('"', "\\\""))),
            _ => None,
        }),
        "font-size-adjust" => crate::font_size_adjust::FontSizeAdjust::parse(css).map(|value| value.to_css()),
        "font-palette" => keyword("normal light dark").or_else(|| match t.as_slice() {
            [Token::Ident(s)] if !s.starts_with("--") && !s.is_empty() => Some(s.clone()),
            [Token::Function(f), Token::OpenParen, ..]
                if f.eq_ignore_ascii_case("palette-mix")
                    && matches!(t.last(), Some(Token::CloseParen)) =>
            {
                Some(serialize(&tokens))
            }
            _ => None,
        }),
        "font-variant-numeric" => keyword("normal").or_else(|| {
            grouped_keywords(
                idents?,
                &[
                    &["lining-nums", "oldstyle-nums"],
                    &["proportional-nums", "tabular-nums"],
                    &["diagonal-fractions", "stacked-fractions"],
                    &["ordinal"],
                    &["slashed-zero"],
                ],
            )
        }),
        "font-variant-east-asian" => keyword("normal").or_else(|| {
            grouped_keywords(
                idents?,
                &[
                    &[
                        "jis78",
                        "jis83",
                        "jis90",
                        "jis04",
                        "simplified",
                        "traditional",
                    ],
                    &["full-width", "proportional-width"],
                    &["ruby"],
                ],
            )
        }),
        "font-variant-alternates" => {
            keyword("normal").or_else(|| valid_alternates(&t).then(|| serialize(&tokens)))
        }
        "font-variation-settings" => {
            keyword("normal").or_else(|| valid_variation(&t).then(|| serialize(&tokens)))
        }
        _ => None,
    }
}

fn grouped_keywords(words: Vec<String>, groups: &[&[&str]]) -> Option<String> {
    if words.is_empty() {
        return None;
    }
    let mut seen = 0u32;
    for word in &words {
        let group = groups.iter().position(|g| g.contains(&word.as_str()))?;
        if seen & (1 << group) != 0 {
            return None;
        }
        seen |= 1 << group;
    }
    Some(words.join(" "))
}

fn valid_variation(t: &[&Token]) -> bool {
    let mut pos = 0;
    if t.is_empty() {
        return false;
    }
    while pos < t.len() {
        let Some(Token::Str(tag)) = t.get(pos).copied() else {
            return false;
        };
        if tag.len() != 4 || !tag.bytes().all(|c| (0x20..=0x7e).contains(&c)) {
            return false;
        }
        let Some(Token::Number(n)) = t.get(pos + 1).copied() else {
            return false;
        };
        if !n.is_finite() {
            return false;
        }
        pos += 2;
        if pos == t.len() {
            return true;
        }
        if !matches!(t.get(pos), Some(Token::Comma)) {
            return false;
        }
        pos += 1;
    }
    false
}

fn valid_alternates(t: &[&Token]) -> bool {
    if t.is_empty() {
        return false;
    }
    let mut pos = 0;
    let mut seen = 0u8;
    while pos < t.len() {
        let (category, needs_args, multiple) = match t[pos] {
            Token::Ident(s) if s.eq_ignore_ascii_case("historical-forms") => (0, false, false),
            Token::Function(s) => match s.to_ascii_lowercase().as_str() {
                "stylistic" => (1, true, false),
                "styleset" => (2, true, true),
                "character-variant" => (3, true, true),
                "swash" => (4, true, false),
                "ornaments" => (5, true, false),
                "annotation" => (6, true, false),
                _ => return false,
            },
            _ => return false,
        };
        if seen & (1 << category) != 0 {
            return false;
        }
        seen |= 1 << category;
        pos += 1;
        if needs_args {
            if !matches!(t.get(pos), Some(Token::OpenParen)) {
                return false;
            }
            pos += 1;
            let mut count = 0;
            loop {
                if !matches!(t.get(pos), Some(Token::Ident(_))) {
                    return false;
                }
                count += 1;
                pos += 1;
                if matches!(t.get(pos), Some(Token::CloseParen)) {
                    pos += 1;
                    break;
                }
                if !multiple || !matches!(t.get(pos), Some(Token::Comma)) {
                    return false;
                }
                pos += 1;
            }
            if count == 0 {
                return false;
            }
        }
    }
    true
}

pub(crate) fn expand_synthesis(value: &str) -> Option<Vec<(String, String)>> {
    let tokens = tokenize(value);
    let t: Vec<_> = tokens
        .iter()
        .filter(|t| !matches!(t, Token::Whitespace))
        .collect();
    let names = ["weight", "style", "small-caps", "position"];
    let wide = match t.as_slice() {
        [Token::Ident(s)]
            if matches!(
                s.to_ascii_lowercase().as_str(),
                "inherit" | "initial" | "unset" | "revert" | "revert-layer"
            ) =>
        {
            Some(s.as_str())
        }
        _ => None,
    };
    if let Some(wide) = wide {
        return Some(
            names
                .iter()
                .map(|name| (format!("font-synthesis-{name}"), wide.to_owned()))
                .collect(),
        );
    }
    let mut seen = 0u8;
    let mut oblique_only = false;
    if t.is_empty() {
        return None;
    }
    if matches!(t.as_slice(), [Token::Ident(s)] if s.eq_ignore_ascii_case("none")) {
        return Some(
            names
                .iter()
                .map(|name| (format!("font-synthesis-{name}"), "none".into()))
                .collect(),
        );
    }
    for token in t {
        let Token::Ident(s) = token else { return None };
        let index = if s.eq_ignore_ascii_case("oblique-only") {
            oblique_only = true;
            1
        } else { names.iter().position(|name| s.eq_ignore_ascii_case(name))? };
        if seen & (1 << index) != 0 {
            return None;
        }
        seen |= 1 << index;
    }
    Some(
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                (
                    format!("font-synthesis-{name}"),
                    if i == 1 && oblique_only { "oblique-only" }
                    else if seen & (1 << i) != 0 { "auto" } else { "none" }.into(),
                )
            })
            .collect(),
    )
}
