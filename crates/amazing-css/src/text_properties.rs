//! CSS Text 4 keyword grammars and overlapping whitespace/wrapping shorthands.
use crate::syntax::{Token, tokenize};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const WHITE_AXES: &[&str] =
    &["white-space-collapse", "text-wrap-mode", "white-space-trim"];
pub(crate) const WRAP_AXES: &[&str] = &["text-wrap-mode", "text-wrap-style"];
pub(crate) const SPACING_AXES: &[&str] = &["text-spacing-trim", "text-autospace"];
pub(crate) fn axes(property: &str) -> Option<&'static [&'static str]> {
    match property {
        "white-space" => Some(WHITE_AXES),
        "text-wrap" => Some(WRAP_AXES),
        "text-spacing" => Some(SPACING_AXES),
        _ => None,
    }
}
pub(crate) fn supported(property: &str) -> bool {
    matches!(
        property,
        "white-space"
            | "white-space-collapse"
            | "white-space-trim"
            | "text-wrap"
            | "text-wrap-mode"
            | "text-wrap-style"
            | "text-transform"
            | "text-autospace"
            | "text-spacing"
            | "text-spacing-trim"
            | "word-break"
            | "line-break"
            | "overflow-wrap"
            | "word-wrap"
            | "hyphens"
            | "hanging-punctuation"
            | "wrap-before"
            | "wrap-after"
            | "wrap-inside"
            | "word-space-transform"
            | "text-justify"
            | "text-group-align"
    )
}
fn words(css: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    for token in tokenize(css) {
        match token {
            Token::Whitespace => {}
            Token::Ident(word) => words.push(word.to_ascii_lowercase()),
            _ => return None,
        }
    }
    (!words.is_empty()).then_some(words)
}
fn one(words: &[String], allowed: &[&str]) -> Option<String> {
    (words.len() == 1 && allowed.contains(&words[0].as_str())).then(|| words[0].clone())
}
fn flags(words: &[String], order: &[&str], exclusive: &[&str]) -> Option<String> {
    if let Some(single) = one(words, exclusive) {
        return Some(single);
    }
    let mut selected = BTreeSet::new();
    for word in words {
        if !order.contains(&word.as_str()) || !selected.insert(word.as_str()) {
            return None;
        }
    }
    Some(
        order
            .iter()
            .filter(|word| selected.contains(**word))
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
    )
}
const COLLAPSE: &[&str] = &[
    "collapse",
    "discard",
    "preserve",
    "preserve-breaks",
    "preserve-spaces",
    "break-spaces",
];
const TRIM: &[&str] = &["discard-before", "discard-after", "discard-inner"];
const WRAP_MODE: &[&str] = &["wrap", "nowrap"];
const WRAP_STYLE: &[&str] = &[
    "auto",
    "balance",
    "stable",
    "pretty",
    "avoid-short-last-line",
];
const SPACING_TRIM: &[&str] = &[
    "space-all",
    "normal",
    "space-first",
    "trim-start",
    "trim-both",
    "trim-all",
    "auto",
];
const AUTOSPACE: &[&str] = &[
    "ideograph-alpha",
    "ideograph-numeric",
    "punctuation",
    "insert",
    "replace",
];

pub(crate) fn canonical(property: &str, css: &str) -> Option<String> {
    let words = words(css)?;
    if let Some(wide) = one(
        &words,
        &["initial", "inherit", "unset", "revert", "revert-layer"],
    ) {
        return Some(wide);
    }
    match property {
        "white-space" | "text-wrap" | "text-spacing" => {
            let expanded = expand_static(property, css)?;
            let props = expanded.into_iter().collect();
            Some(shorthand(property, &props))
        }
        "white-space-collapse" => one(&words, COLLAPSE),
        "white-space-trim" => flags(&words, TRIM, &["none"]),
        "text-wrap-mode" => one(&words, WRAP_MODE),
        "text-wrap-style" => one(&words, WRAP_STYLE),
        "text-spacing-trim" => one(&words, SPACING_TRIM),
        "text-autospace" => {
            let result = flags(&words, AUTOSPACE, &["auto", "normal", "no-autospace"])?;
            if words
                .iter()
                .filter(|word| matches!(word.as_str(), "insert" | "replace"))
                .count()
                > 1
            {
                return None;
            }
            Some(result)
        }
        "text-transform" => {
            let result = flags(
                &words,
                &[
                    "capitalize",
                    "uppercase",
                    "lowercase",
                    "full-width",
                    "full-size-kana",
                ],
                &["none", "math-auto"],
            )?;
            if words
                .iter()
                .filter(|word| matches!(word.as_str(), "capitalize" | "uppercase" | "lowercase"))
                .count()
                > 1
            {
                return None;
            }
            Some(result)
        }
        "word-break" => one(
            &words,
            &[
                "normal",
                "break-all",
                "keep-all",
                "manual",
                "auto-phrase",
                "break-word",
            ],
        ),
        "line-break" => one(&words, &["auto", "loose", "normal", "strict", "anywhere"]),
        "overflow-wrap" | "word-wrap" => one(&words, &["normal", "break-word", "anywhere"]),
        "hyphens" => one(&words, &["none", "manual", "auto"]),
        "text-group-align" => one(&words, &["none", "start", "end", "left", "right", "center"]),
        "wrap-before" | "wrap-after" => one(
            &words,
            &["auto", "avoid", "avoid-line", "avoid-flex", "line", "flex"],
        ),
        "wrap-inside" => one(&words, &["auto", "avoid"]),
        "hanging-punctuation" => {
            let result = flags(
                &words,
                &["first", "force-end", "allow-end", "last"],
                &["none"],
            )?;
            if words.contains(&"force-end".into()) && words.contains(&"allow-end".into()) {
                return None;
            }
            Some(result)
        }
        "word-space-transform" => {
            if words == ["none"] {
                return Some("none".into());
            }
            let result = flags(&words, &["space", "ideographic-space", "auto-phrase"], &[])?;
            if words
                .iter()
                .filter(|word| matches!(word.as_str(), "space" | "ideographic-space"))
                .count()
                != 1
            {
                return None;
            }
            Some(result)
        }
        "text-justify" => {
            if words == ["distribute"] {
                return Some("inter-character".into());
            }
            let result = flags(
                &words,
                &[
                    "auto",
                    "none",
                    "inter-word",
                    "inter-character",
                    "ruby",
                    "no-compress",
                ],
                &[],
            )?;
            if words
                .iter()
                .filter(|word| word.as_str() != "no-compress")
                .count()
                > 1
            {
                return None;
            }
            Some(result)
        }
        _ => None,
    }
}

pub(crate) fn expand_static(property: &str, css: &str) -> Option<Vec<(String, String)>> {
    let names = axes(property)?;
    let words = words(css)?;
    if let Some(wide) = one(
        &words,
        &["initial", "inherit", "unset", "revert", "revert-layer"],
    ) {
        return Some(
            names
                .iter()
                .map(|name| ((*name).into(), wide.clone()))
                .collect(),
        );
    }
    let values = match property {
        "white-space" => {
            let legacy = match words.as_slice() {
                [word] => match word.as_str() {
                    "normal" => Some(("collapse", "wrap")),
                    "pre" => Some(("preserve", "nowrap")),
                    "pre-wrap" => Some(("preserve", "wrap")),
                    "pre-line" => Some(("preserve-breaks", "wrap")),
                    _ => None,
                },
                _ => None,
            };
            if let Some((collapse, wrap)) = legacy {
                vec![collapse.into(), wrap.into(), "none".into()]
            } else {
                let mut collapse = None;
                let mut wrap = None;
                let mut trim = Vec::new();
                for word in words {
                    if COLLAPSE.contains(&word.as_str()) {
                        if collapse.replace(word).is_some() {
                            return None;
                        }
                    } else if WRAP_MODE.contains(&word.as_str()) {
                        if wrap.replace(word).is_some() {
                            return None;
                        }
                    } else {
                        trim.push(word);
                    }
                }
                let trim = if trim.is_empty() {
                    "none".into()
                } else {
                    flags(&trim, TRIM, &["none"])?
                };
                vec![
                    collapse.unwrap_or("collapse".into()),
                    wrap.unwrap_or("wrap".into()),
                    trim,
                ]
            }
        }
        "text-wrap" => {
            let mut mode = None;
            let mut style = None;
            for word in words {
                if WRAP_MODE.contains(&word.as_str()) {
                    if mode.replace(word).is_some() {
                        return None;
                    }
                } else if WRAP_STYLE.contains(&word.as_str()) {
                    if style.replace(word).is_some() {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            vec![
                mode.unwrap_or("wrap".into()),
                style.unwrap_or("auto".into()),
            ]
        }
        "text-spacing" => {
            if words == ["none"] {
                vec!["space-all".into(), "no-autospace".into()]
            } else if words == ["auto"] {
                vec!["auto".into(), "auto".into()]
            } else {
                let mut trim = None;
                let mut autospace = Vec::new();
                let mut normal = 0;
                for word in words {
                    if word == "normal" {
                        normal += 1;
                    } else if SPACING_TRIM.contains(&word.as_str()) && word != "auto" {
                        if trim.replace(word).is_some() {
                            return None;
                        }
                    } else {
                        autospace.push(word);
                    }
                }
                if normal > 2 || (trim.is_some() && normal > 1) {
                    return None;
                }
                if normal > 0 && (trim.is_some() || normal > 1) {
                    autospace.push("normal".into());
                }
                let autospace = if autospace.is_empty() {
                    "normal".into()
                } else {
                    let canonical = canonical("text-autospace", &autospace.join(" "))?;
                    if canonical == "auto" {
                        return None;
                    }
                    canonical
                };
                vec![trim.unwrap_or("normal".into()), autospace]
            }
        }
        _ => return None,
    };
    Some(
        names
            .iter()
            .zip(values)
            .map(|(name, value)| ((*name).into(), value))
            .collect(),
    )
}

fn shorthand(property: &str, props: &BTreeMap<String, String>) -> String {
    let get = |name: &str, initial: &str| props.get(name).cloned().unwrap_or(initial.into());
    match property {
        "white-space" => {
            let collapse = get("white-space-collapse", "collapse");
            let mode = get("text-wrap-mode", "wrap");
            let trim = get("white-space-trim", "none");
            if trim == "none" {
                match (collapse.as_str(), mode.as_str()) {
                    ("collapse", "wrap") => return "normal".into(),
                    ("collapse", "nowrap") => return "nowrap".into(),
                    ("preserve", "nowrap") => return "pre".into(),
                    ("preserve", "wrap") => return "pre-wrap".into(),
                    ("preserve-breaks", "wrap") => return "pre-line".into(),
                    ("break-spaces", "wrap") => return "break-spaces".into(),
                    _ => {}
                }
            }
            let mut result = vec![collapse];
            if mode != "wrap" {
                result.push(mode);
            }
            if trim != "none" {
                result.push(trim);
            }
            result.join(" ")
        }
        "text-wrap" => {
            let mode = get("text-wrap-mode", "wrap");
            let style = get("text-wrap-style", "auto");
            if style == "auto" {
                mode
            } else if mode == "wrap" {
                style
            } else {
                format!("{mode} {style}")
            }
        }
        "text-spacing" => {
            let trim = get("text-spacing-trim", "normal");
            let autospace = get("text-autospace", "normal");
            if trim == "auto" && autospace == "auto" {
                return "auto".into();
            }
            if trim == "space-all" && autospace == "no-autospace" {
                return "none".into();
            }
            if trim == "auto" || autospace == "auto" {
                return String::new();
            }
            if autospace == "normal" {
                trim
            } else if trim == "normal" {
                autospace
            } else {
                format!("{trim} {autospace}")
            }
        }
        _ => String::new(),
    }
}

pub(crate) fn serialize(props: &mut BTreeMap<String, String>) {
    // Preserve older synthetic styles that carry only the legacy shorthand.
    if let Some(legacy) = props
        .get("white-space")
        .cloned()
        .and_then(|value| expand_static("white-space", &value))
    {
        for (name, value) in legacy {
            props.entry(name).or_insert(value);
        }
    }
    if let Some(value) = props.get("overflow-wrap").cloned() {
        props.insert("word-wrap".into(), value);
    }
    for property in ["white-space", "text-wrap", "text-spacing"] {
        props.insert(property.into(), shorthand(property, props));
    }
}
