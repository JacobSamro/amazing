//! Built-in counter representations from CSS Counter Styles 3.
use std::collections::HashMap;

use crate::media_queries::{MediaEnvironment, MediaQueryList};
use crate::parser::{CounterStyleRule, Stylesheet};
use crate::syntax::{Token, tokenize};

fn numeric(number: i32, digits: &str) -> String {
    let symbols: Vec<char> = digits.chars().collect();
    let magnitude = i64::from(number).abs().to_string();
    let body: String = magnitude
        .bytes()
        .map(|digit| symbols[usize::from(digit - b'0')])
        .collect();
    if number < 0 { format!("-{body}") } else { body }
}

fn alphabetic(mut number: i32, symbols: &str) -> Option<String> {
    if number <= 0 {
        return None;
    }
    let symbols: Vec<char> = symbols.chars().collect();
    let radix = i32::try_from(symbols.len()).ok()?;
    if radix < 2 {
        return None;
    }
    let mut result = String::new();
    while number > 0 {
        number -= 1;
        result.insert(0, symbols[usize::try_from(number % radix).ok()?]);
        number /= radix;
    }
    Some(result)
}

fn cyclic(number: i32, symbols: &str) -> Option<String> {
    let symbols: Vec<char> = symbols.chars().collect();
    let len = i64::try_from(symbols.len()).ok()?;
    if len == 0 {
        return None;
    }
    let index = (i64::from(number) - 1).rem_euclid(len);
    Some(symbols[usize::try_from(index).ok()?].to_string())
}

fn roman(mut number: i32, upper: bool) -> Option<String> {
    if !(1..4000).contains(&number) {
        return None;
    }
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
    Some(if upper {
        result.to_ascii_uppercase()
    } else {
        result
    })
}

pub(crate) fn format_counter(number: i32, style: &str) -> String {
    let style = style.to_ascii_lowercase();
    let representation = match style.as_str() {
        "decimal" => Some(number.to_string()),
        "decimal-leading-zero" if (-9..10).contains(&number) => Some(if number < 0 {
            format!("-0{}", number.abs())
        } else {
            format!("0{number}")
        }),
        "arabic-indic" => Some(numeric(number, "٠١٢٣٤٥٦٧٨٩")),
        "bengali" => Some(numeric(number, "০১২৩৪৫৬৭৮৯")),
        "cambodian" | "khmer" => Some(numeric(number, "០១២៣៤៥៦៧៨៩")),
        "cjk-decimal" => Some(numeric(number, "〇一二三四五六七八九")),
        "devanagari" => Some(numeric(number, "०१२३४५६७८९")),
        "gujarati" => Some(numeric(number, "૦૧૨૩૪૫૬૭૮૯")),
        "gurmukhi" => Some(numeric(number, "੦੧੨੩੪੫੬੭੮੯")),
        "kannada" => Some(numeric(number, "೦೧೨೩೪೫೬೭೮೯")),
        "lao" => Some(numeric(number, "໐໑໒໓໔໕໖໗໘໙")),
        "malayalam" => Some(numeric(number, "൦൧൨൩൪൫൬൭൮൯")),
        "mongolian" => Some(numeric(number, "᠐᠑᠒᠓᠔᠕᠖᠗᠘᠙")),
        "myanmar" => Some(numeric(number, "၀၁၂၃၄၅၆၇၈၉")),
        "oriya" => Some(numeric(number, "୦୧୨୩୪୫୬୭୮୯")),
        "persian" => Some(numeric(number, "۰۱۲۳۴۵۶۷۸۹")),
        "tamil" => Some(numeric(number, "௦௧௨௩௪௫௬௭௮௯")),
        "telugu" => Some(numeric(number, "౦౧౨౩౪౫౬౭౮౯")),
        "thai" => Some(numeric(number, "๐๑๒๓๔๕๖๗๘๙")),
        "tibetan" => Some(numeric(number, "༠༡༢༣༤༥༦༧༨༩")),
        "lower-roman" => roman(number, false),
        "upper-roman" => roman(number, true),
        "lower-alpha" | "lower-latin" => alphabetic(number, "abcdefghijklmnopqrstuvwxyz"),
        "upper-alpha" | "upper-latin" => alphabetic(number, "ABCDEFGHIJKLMNOPQRSTUVWXYZ"),
        "lower-greek" => alphabetic(number, "αβγδεζηθικλμνξοπρστυφχψω"),
        "hiragana" => alphabetic(
            number,
            "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわゐゑをん",
        ),
        "katakana" => alphabetic(
            number,
            "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヰヱヲン",
        ),
        "hiragana-iroha" => alphabetic(
            number,
            "いろはにほへとちりぬるをわかよたれそつねならむうゐのおくやまけふこえてあさきゆめみしゑひもせす",
        ),
        "katakana-iroha" => alphabetic(
            number,
            "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス",
        ),
        "cjk-heavenly-stem" => cyclic(number, "甲乙丙丁戊己庚辛壬癸"),
        "cjk-earthly-branch" => cyclic(number, "子丑寅卯辰巳午未申酉戌亥"),
        _ => None,
    };
    representation.unwrap_or_else(|| number.to_string())
}

#[derive(Default)]
pub(crate) struct CounterStyles {
    rules: HashMap<String, CounterStyleRule>,
}

impl CounterStyles {
    pub(crate) fn new(sheets: &[Stylesheet], environment: &MediaEnvironment) -> Self {
        let mut rules = HashMap::new();
        for sheet in sheets {
            for rule in &sheet.counter_styles {
                if rule
                    .media
                    .iter()
                    .all(|query| MediaQueryList::parse(query).evaluate(environment))
                {
                    rules.insert(rule.name.clone(), rule.clone());
                }
            }
        }
        Self { rules }
    }

    pub(crate) fn format(&self, number: i32, style: &str) -> String {
        self.representation(number, style, 0)
            .unwrap_or_else(|| format_counter(number, "decimal"))
    }

    pub(crate) fn marker(&self, number: i32, style: &str) -> String {
        let mut prefix = None;
        let mut suffix = None;
        let mut current = style;
        for _ in 0..16 {
            let Some(rule) = self.rules.get(current) else {
                break;
            };
            if prefix.is_none() {
                prefix = descriptor(rule, "prefix").and_then(single_symbol);
            }
            if suffix.is_none() {
                suffix = descriptor(rule, "suffix").and_then(single_symbol);
            }
            let Some(parent) =
                descriptor(rule, "system").and_then(|value| value.trim().strip_prefix("extends "))
            else {
                break;
            };
            current = parent.trim();
        }
        format!(
            "{}{}{}",
            prefix.unwrap_or_default(),
            self.format(number, style),
            suffix.unwrap_or_else(|| ". ".to_owned())
        )
    }

    fn representation(&self, number: i32, style: &str, depth: usize) -> Option<String> {
        if depth >= 16 {
            return None;
        }
        let Some(rule) = self.rules.get(style) else {
            return Some(format_counter(number, style));
        };
        let system = descriptor(rule, "system").unwrap_or("symbolic").trim();
        let symbols = descriptor(rule, "symbols")
            .map(parse_symbols)
            .unwrap_or_default();
        let negative = number < 0;
        let magnitude = i64::from(number).abs();
        let body = if let Some(parent) = system.strip_prefix("extends ") {
            return self.representation(number, parent.trim(), depth + 1);
        } else if system == "numeric" && symbols.len() >= 2 {
            let radix = i64::try_from(symbols.len()).ok()?;
            let mut value = magnitude;
            let mut out = Vec::new();
            loop {
                out.push(symbols[usize::try_from(value % radix).ok()?].clone());
                value /= radix;
                if value == 0 {
                    break;
                }
            }
            out.into_iter().rev().collect::<String>()
        } else if system == "alphabetic" && symbols.len() >= 2 && number > 0 {
            let radix = i64::try_from(symbols.len()).ok()?;
            let mut value = magnitude;
            let mut out = Vec::new();
            while value > 0 {
                value -= 1;
                out.push(symbols[usize::try_from(value % radix).ok()?].clone());
                value /= radix;
            }
            out.into_iter().rev().collect::<String>()
        } else if system == "cyclic" && !symbols.is_empty() {
            let radix = i64::try_from(symbols.len()).ok()?;
            symbols[usize::try_from((i64::from(number) - 1).rem_euclid(radix)).ok()?].clone()
        } else if system == "symbolic" && !symbols.is_empty() && number > 0 {
            let radix = i64::try_from(symbols.len()).ok()?;
            let repeats = usize::try_from((magnitude - 1) / radix + 1).ok()?;
            if repeats > 1024 {
                return None;
            }
            symbols[usize::try_from((magnitude - 1) % radix).ok()?].repeat(repeats)
        } else if system.starts_with("fixed") && !symbols.is_empty() {
            let start = system
                .strip_prefix("fixed")
                .unwrap_or("")
                .trim()
                .parse::<i64>()
                .unwrap_or(1);
            symbols
                .get(usize::try_from(i64::from(number) - start).ok()?)?
                .clone()
        } else {
            let fallback = descriptor(rule, "fallback").unwrap_or("decimal").trim();
            return self.representation(number, fallback, depth + 1);
        };
        let padded = if let Some((minimum, pad)) = descriptor(rule, "pad").and_then(parse_pad) {
            format!(
                "{}{body}",
                pad.repeat(minimum.saturating_sub(body.chars().count()).min(1024))
            )
        } else {
            body
        };
        if negative {
            let signs = descriptor(rule, "negative")
                .map(parse_symbols)
                .unwrap_or_else(|| vec!["-".into()]);
            Some(format!(
                "{}{}{}",
                signs.first().map(String::as_str).unwrap_or("-"),
                padded,
                signs.get(1).map(String::as_str).unwrap_or("")
            ))
        } else {
            Some(padded)
        }
    }
}

fn descriptor<'a>(rule: &'a CounterStyleRule, name: &str) -> Option<&'a str> {
    rule.descriptors
        .iter()
        .rev()
        .find(|declaration| declaration.property == name)
        .map(|declaration| declaration.value.as_str())
}

fn parse_symbols(input: &str) -> Vec<String> {
    tokenize(input)
        .into_iter()
        .filter_map(|token| match token {
            Token::Str(value) | Token::Ident(value) => Some(value),
            _ => None,
        })
        .collect()
}

fn single_symbol(input: &str) -> Option<String> {
    let symbols = parse_symbols(input);
    (symbols.len() == 1).then(|| symbols[0].clone())
}

fn parse_pad(input: &str) -> Option<(usize, String)> {
    let tokens: Vec<_> = tokenize(input)
        .into_iter()
        .filter(|token| *token != Token::Whitespace)
        .collect();
    let [
        Token::Number(width),
        Token::Str(symbol) | Token::Ident(symbol),
    ] = tokens.as_slice()
    else {
        return None;
    };
    if *width < 0.0 || width.fract() != 0.0 {
        return None;
    }
    Some(((*width as usize).min(1024), symbol.clone()))
}

#[cfg(test)]
mod tests {
    use super::format_counter;

    #[test]
    fn numeric_and_alphabetic_predefined_styles() {
        for (number, style, expected) in [
            (120, "bengali", "১২০"),
            (-12, "devanagari", "-१२"),
            (12, "malayalam", "൧൨"),
            (28, "upper-latin", "AB"),
            (25, "lower-greek", "αα"),
            (2, "cjk-heavenly-stem", "乙"),
            (40, "upper-roman", "XL"),
        ] {
            assert_eq!(format_counter(number, style), expected);
        }
    }
}
