//! Counter representations from CSS Counter Styles 3 (§6 simple, §7 complex).
use std::collections::HashMap;

use crate::media_queries::{MediaEnvironment, MediaQueryList};
use crate::parser::{CounterStyleRule, Stylesheet};
use crate::syntax::{Token, tokenize};

fn numeric(number: i64, digits: &str) -> String {
    let symbols: Vec<char> = digits.chars().collect();
    let magnitude = number.unsigned_abs().to_string();
    let body: String = magnitude
        .bytes()
        .map(|digit| symbols[usize::from(digit - b'0')])
        .collect();
    if number < 0 { format!("-{body}") } else { body }
}

fn alphabetic(mut number: i64, symbols: &str) -> Option<String> {
    if number <= 0 {
        return None;
    }
    let symbols: Vec<char> = symbols.chars().collect();
    let radix = i64::try_from(symbols.len()).ok()?;
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

fn cyclic(number: i64, symbols: &str) -> Option<String> {
    let symbols: Vec<char> = symbols.chars().collect();
    let len = i64::try_from(symbols.len()).ok()?;
    if len == 0 {
        return None;
    }
    let index = (number - 1).rem_euclid(len);
    Some(symbols[usize::try_from(index).ok()?].to_string())
}

/// `fixed` starts at 1 and does not wrap: values past the symbol list fall back.
fn fixed(number: i64, symbols: &str) -> Option<String> {
    let symbols: Vec<char> = symbols.chars().collect();
    if number < 1 {
        return None;
    }
    symbols.get(usize::try_from(number - 1).ok()?).map(char::to_string)
}

fn roman(mut number: i64, upper: bool) -> Option<String> {
    if !(1..4000).contains(&number) {
        return None;
    }
    let mut result = String::new();
    for (value, numeral) in [
        (1000i64, "m"),
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

/// Additive ("sign-value") formatting. `tuples` must be decreasing by weight.
fn additive(number: i64, tuples: &[(i32, &str)], negative: &str) -> Option<String> {
    let body = additive_body(number, tuples.iter().map(|(w, s)| (*w, *s)).collect())?;
    Some(if number < 0 {
        format!("{negative}{body}")
    } else {
        body
    })
}

fn additive_body(number: i64, tuples: Vec<(i32, &str)>) -> Option<String> {
    let magnitude = number.unsigned_abs();
    if magnitude == 0 {
        return tuples
            .into_iter()
            .find(|(weight, _)| *weight == 0)
            .map(|(_, symbol)| symbol.to_owned());
    }
    let mut value = magnitude;
    let mut out = String::new();
    for (weight, symbol) in tuples {
        let weight = i64::from(weight).unsigned_abs();
        if weight == 0 || weight > value {
            continue;
        }
        let reps = value / weight;
        if reps > 1024 {
            return None;
        }
        for _ in 0..reps {
            out.push_str(symbol);
        }
        value -= weight * reps;
        if value == 0 {
            return Some(out);
        }
    }
    None
}

const ARMENIAN: &[(i32, &str)] = &[
    (9000, "Ք"), (8000, "Փ"), (7000, "Ւ"), (6000, "Ց"), (5000, "Ր"), (4000, "Տ"),
    (3000, "Վ"), (2000, "Ս"), (1000, "Ռ"), (900, "Ջ"), (800, "Պ"), (700, "Չ"),
    (600, "Ո"), (500, "Շ"), (400, "Ն"), (300, "Յ"), (200, "Մ"), (100, "Ճ"),
    (90, "Ղ"), (80, "Ձ"), (70, "Հ"), (60, "Կ"), (50, "Ծ"), (40, "Խ"),
    (30, "Լ"), (20, "Ի"), (10, "Ժ"), (9, "Թ"), (8, "Ը"), (7, "Է"),
    (6, "Զ"), (5, "Ե"), (4, "Դ"), (3, "Գ"), (2, "Բ"), (1, "Ա"),
];

const LOWER_ARMENIAN: &[(i32, &str)] = &[
    (9000, "ք"), (8000, "փ"), (7000, "ւ"), (6000, "ց"), (5000, "ր"), (4000, "տ"),
    (3000, "վ"), (2000, "ս"), (1000, "ռ"), (900, "ջ"), (800, "պ"), (700, "չ"),
    (600, "ո"), (500, "շ"), (400, "ն"), (300, "յ"), (200, "մ"), (100, "ճ"),
    (90, "ղ"), (80, "ձ"), (70, "հ"), (60, "կ"), (50, "ծ"), (40, "խ"),
    (30, "լ"), (20, "ի"), (10, "ժ"), (9, "թ"), (8, "ը"), (7, "է"),
    (6, "զ"), (5, "ե"), (4, "դ"), (3, "գ"), (2, "բ"), (1, "ա"),
];

const GEORGIAN: &[(i32, &str)] = &[
    (10000, "ჵ"), (9000, "ჰ"), (8000, "ჯ"), (7000, "ჴ"), (6000, "ხ"), (5000, "ჭ"),
    (4000, "წ"), (3000, "ძ"), (2000, "ც"), (1000, "ჩ"), (900, "შ"), (800, "ყ"),
    (700, "ღ"), (600, "ქ"), (500, "ფ"), (400, "ჳ"), (300, "ტ"), (200, "ს"),
    (100, "რ"), (90, "ჟ"), (80, "პ"), (70, "ო"), (60, "ჲ"), (50, "ნ"),
    (40, "მ"), (30, "ლ"), (20, "კ"), (10, "ი"), (9, "თ"), (8, "ჱ"),
    (7, "ზ"), (6, "ვ"), (5, "ე"), (4, "დ"), (3, "გ"), (2, "ბ"), (1, "ა"),
];

const HEBREW: &[(i32, &str)] = &[
    (10000, "י׳"), (9000, "ט׳"), (8000, "ח׳"), (7000, "ז׳"), (6000, "ו׳"), (5000, "ה׳"),
    (4000, "ד׳"), (3000, "ג׳"), (2000, "ב׳"), (1000, "א׳"), (400, "ת"), (300, "ש"),
    (200, "ר"), (100, "ק"), (90, "צ"), (80, "פ"), (70, "ע"), (60, "ס"),
    (50, "נ"), (40, "מ"), (30, "ל"), (20, "כ"), (19, "יט"), (18, "יח"),
    (17, "יז"), (16, "טז"), (15, "טו"), (10, "י"), (9, "ט"), (8, "ח"),
    (7, "ז"), (6, "ו"), (5, "ה"), (4, "ד"), (3, "ג"), (2, "ב"), (1, "א"),
];

const JAPANESE_INFORMAL: &[(i32, &str)] = &[
    (9000, "九千"), (8000, "八千"), (7000, "七千"), (6000, "六千"), (5000, "五千"),
    (4000, "四千"), (3000, "三千"), (2000, "二千"), (1000, "千"), (900, "九百"),
    (800, "八百"), (700, "七百"), (600, "六百"), (500, "五百"), (400, "四百"),
    (300, "三百"), (200, "二百"), (100, "百"), (90, "九十"), (80, "八十"),
    (70, "七十"), (60, "六十"), (50, "五十"), (40, "四十"), (30, "三十"),
    (20, "二十"), (10, "十"), (9, "九"), (8, "八"), (7, "七"), (6, "六"),
    (5, "五"), (4, "四"), (3, "三"), (2, "二"), (1, "一"), (0, "〇"),
];

const JAPANESE_FORMAL: &[(i32, &str)] = &[
    (9000, "九阡"), (8000, "八阡"), (7000, "七阡"), (6000, "六阡"), (5000, "伍阡"),
    (4000, "四阡"), (3000, "参阡"), (2000, "弐阡"), (1000, "壱阡"), (900, "九百"),
    (800, "八百"), (700, "七百"), (600, "六百"), (500, "伍百"), (400, "四百"),
    (300, "参百"), (200, "弐百"), (100, "壱百"), (90, "九拾"), (80, "八拾"),
    (70, "七拾"), (60, "六拾"), (50, "伍拾"), (40, "四拾"), (30, "参拾"),
    (20, "弐拾"), (10, "壱拾"), (9, "九"), (8, "八"), (7, "七"), (6, "六"),
    (5, "伍"), (4, "四"), (3, "参"), (2, "弐"), (1, "壱"), (0, "零"),
];

const KOREAN_HANGUL_FORMAL: &[(i32, &str)] = &[
    (9000, "구천"), (8000, "팔천"), (7000, "칠천"), (6000, "육천"), (5000, "오천"),
    (4000, "사천"), (3000, "삼천"), (2000, "이천"), (1000, "일천"), (900, "구백"),
    (800, "팔백"), (700, "칠백"), (600, "육백"), (500, "오백"), (400, "사백"),
    (300, "삼백"), (200, "이백"), (100, "일백"), (90, "구십"), (80, "팔십"),
    (70, "칠십"), (60, "육십"), (50, "오십"), (40, "사십"), (30, "삼십"),
    (20, "이십"), (10, "일십"), (9, "구"), (8, "팔"), (7, "칠"), (6, "육"),
    (5, "오"), (4, "사"), (3, "삼"), (2, "이"), (1, "일"), (0, "영"),
];

const KOREAN_HANJA_INFORMAL: &[(i32, &str)] = &[
    (9000, "九千"), (8000, "八千"), (7000, "七千"), (6000, "六千"), (5000, "五千"),
    (4000, "四千"), (3000, "三千"), (2000, "二千"), (1000, "千"), (900, "九百"),
    (800, "八百"), (700, "七百"), (600, "六百"), (500, "五百"), (400, "四百"),
    (300, "三百"), (200, "二百"), (100, "百"), (90, "九十"), (80, "八十"),
    (70, "七十"), (60, "六十"), (50, "五十"), (40, "四十"), (30, "三十"),
    (20, "二十"), (10, "十"), (9, "九"), (8, "八"), (7, "七"), (6, "六"),
    (5, "五"), (4, "四"), (3, "三"), (2, "二"), (1, "一"), (0, "零"),
];

const KOREAN_HANJA_FORMAL: &[(i32, &str)] = &[
    (9000, "九仟"), (8000, "八仟"), (7000, "七仟"), (6000, "六仟"), (5000, "五仟"),
    (4000, "四仟"), (3000, "參仟"), (2000, "貳仟"), (1000, "壹仟"), (900, "九百"),
    (800, "八百"), (700, "七百"), (600, "六百"), (500, "五百"), (400, "四百"),
    (300, "參百"), (200, "貳百"), (100, "壹百"), (90, "九拾"), (80, "八拾"),
    (70, "七拾"), (60, "六拾"), (50, "五拾"), (40, "四拾"), (30, "參拾"),
    (20, "貳拾"), (10, "壹拾"), (9, "九"), (8, "八"), (7, "七"), (6, "六"),
    (5, "五"), (4, "四"), (3, "參"), (2, "貳"), (1, "壹"), (0, "零"),
];

struct Chinese {
    digits: [char; 10],
    tens: char,
    hundreds: char,
    thousands: char,
    negative: char,
    informal: bool,
    /// Group markers for the ones, ten-thousands, hundred-millions and
    /// trillions groups (the fourth is two characters in simplified Chinese).
    group_markers: [&'static str; 4],
}

const SIMP_INFORMAL: Chinese = Chinese {
    digits: ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
    tens: '十',
    hundreds: '百',
    thousands: '千',
    negative: '负',
    informal: true,
    group_markers: ["", "万", "亿", "万亿"],
};

const SIMP_FORMAL: Chinese = Chinese {
    digits: ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'],
    tens: '拾',
    hundreds: '佰',
    thousands: '仟',
    negative: '负',
    informal: false,
    group_markers: ["", "万", "亿", "万亿"],
};

const TRAD_INFORMAL: Chinese = Chinese {
    digits: ['零', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
    tens: '十',
    hundreds: '百',
    thousands: '千',
    negative: '負',
    informal: true,
    group_markers: ["", "萬", "億", "兆"],
};

const TRAD_FORMAL: Chinese = Chinese {
    digits: ['零', '壹', '貳', '參', '肆', '伍', '陸', '柒', '捌', '玖'],
    tens: '拾',
    hundreds: '佰',
    thousands: '仟',
    negative: '負',
    informal: false,
    group_markers: ["", "萬", "億", "兆"],
};

/// CSS Counter Styles 3 §7.1.2 extended longhand CJK algorithm (|value| < 10^16).
fn chinese(number: i64, style: &Chinese) -> Option<String> {
    if number.unsigned_abs() >= 10u64.pow(16) {
        return None;
    }
    if number == 0 {
        return Some(style.digits[0].to_string());
    }
    let negative = number < 0;
    let magnitude = number.unsigned_abs();
    let mut groups = Vec::new();
    let mut rest = magnitude;
    while rest > 0 {
        groups.push((rest % 10_000) as u32);
        rest /= 10_000;
    }
    let units = [style.thousands, style.hundreds, style.tens, '\0'];
    let divisors = [1000u32, 100, 10, 1];
    let mut out = String::new();
    let mut zero_pending = false;
    for index in (0..groups.len()).rev() {
        let group = groups[index];
        if group == 0 {
            zero_pending = true;
            continue;
        }
        // A zero group, or a non-zero group with leading zeros, contributes a
        // single 零 separator.
        if !out.is_empty() && (zero_pending || group < 1000) {
            out.push(style.digits[0]);
        }
        zero_pending = false;
        let mut text = String::new();
        let mut inner_zero = false;
        for position in 0..4 {
            let digit = (group / divisors[position]) % 10;
            if digit == 0 {
                if !text.is_empty() && group % divisors[position] != 0 {
                    inner_zero = true;
                }
                continue;
            }
            if inner_zero {
                text.push(style.digits[0]);
                inner_zero = false;
            }
            text.push(style.digits[digit as usize]);
            if position < 3 {
                text.push(units[position]);
            }
        }
        // Informal styles drop the tens digit for any group in 10..=19.
        if style.informal && (10..=19).contains(&group) {
            text.remove(0);
        }
        out.push_str(&text);
        if index > 0 {
            out.push_str(style.group_markers[index]);
        }
    }
    Some(if negative {
        format!("{}{out}", style.negative)
    } else {
        out
    })
}

/// The suffix a predefined counter style appends to a `::marker`.
pub(crate) fn predefined_suffix(style: &str) -> &'static str {
    match style {
        "cjk-decimal" | "cjk-earthly-branch" | "cjk-heavenly-stem" | "hiragana"
        | "hiragana-iroha" | "katakana" | "katakana-iroha" | "japanese-informal"
        | "japanese-formal" | "simp-chinese-informal" | "simp-chinese-formal"
        | "trad-chinese-informal" | "trad-chinese-formal" | "cjk-ideographic" => "、",
        "korean-hangul-formal" | "korean-hanja-informal" | "korean-hanja-formal" => ", ",
        _ => ". ",
    }
}

/// The fallback a predefined counter style uses when a value is out of range.
fn predefined_fallback(style: &str) -> &'static str {
    match style {
        "cjk-decimal" | "cjk-earthly-branch" | "cjk-heavenly-stem" | "japanese-informal"
        | "japanese-formal" | "korean-hangul-formal" | "korean-hanja-informal"
        | "korean-hanja-formal" | "simp-chinese-informal" | "simp-chinese-formal"
        | "trad-chinese-informal" | "trad-chinese-formal" | "cjk-ideographic" => {
            "cjk-decimal"
        }
        _ => "decimal",
    }
}

/// Representation for a predefined style, or `None` when the value is outside the
/// style's range (the caller then uses [`predefined_fallback`]).
fn predefined_representation(number: i64, style: &str) -> Option<String> {
    match style {
        "decimal" => Some(number.to_string()),
        "decimal-leading-zero" => Some(if (-9..10).contains(&number) {
            if number < 0 {
                format!("-0{}", number.abs())
            } else {
                format!("0{number}")
            }
        } else {
            number.to_string()
        }),
        "arabic-indic" => Some(numeric(number, "٠١٢٣٤٥٦٧٨٩")),
        "bengali" => Some(numeric(number, "০১২৩৪৫৬৭৮৯")),
        "cambodian" | "khmer" => Some(numeric(number, "០១២៣៤៥៦៧៨៩")),
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
        "cjk-decimal" if number >= 0 => Some(numeric(number, "〇一二三四五六七八九")),
        "cjk-decimal" => None,
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
        "cjk-earthly-branch" => fixed(number, "子丑寅卯辰巳午未申酉戌亥"),
        "cjk-heavenly-stem" => fixed(number, "甲乙丙丁戊己庚辛壬癸"),
        "armenian" | "upper-armenian" if (1..=9999).contains(&number) => {
            additive(number, ARMENIAN, "-")
        }
        "lower-armenian" if (1..=9999).contains(&number) => {
            additive(number, LOWER_ARMENIAN, "-")
        }
        "georgian" if (1..=19999).contains(&number) => additive(number, GEORGIAN, "-"),
        "hebrew" if (1..=10999).contains(&number) => additive(number, HEBREW, "-"),
        "japanese-informal" if (-9999..=9999).contains(&number) => {
            additive(number, JAPANESE_INFORMAL, "マイナス")
        }
        "japanese-formal" if (-9999..=9999).contains(&number) => {
            additive(number, JAPANESE_FORMAL, "マイナス")
        }
        "korean-hangul-formal" if (-9999..=9999).contains(&number) => {
            additive(number, KOREAN_HANGUL_FORMAL, "마이너스  ")
        }
        "korean-hanja-informal" if (-9999..=9999).contains(&number) => {
            additive(number, KOREAN_HANJA_INFORMAL, "마이너스  ")
        }
        "korean-hanja-formal" if (-9999..=9999).contains(&number) => {
            additive(number, KOREAN_HANJA_FORMAL, "마이너스  ")
        }
        "simp-chinese-informal" => chinese(number, &SIMP_INFORMAL),
        "simp-chinese-formal" => chinese(number, &SIMP_FORMAL),
        "trad-chinese-informal" | "cjk-ideographic" => chinese(number, &TRAD_INFORMAL),
        "trad-chinese-formal" => chinese(number, &TRAD_FORMAL),
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn format_counter(number: i64, style: &str) -> String {
    let style = style.to_ascii_lowercase();
    predefined_representation(number, &style)
        .or_else(|| predefined_representation(number, predefined_fallback(&style)))
        .unwrap_or_else(|| number.to_string())
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

    pub(crate) fn format(&self, number: i64, style: &str) -> String {
        self.representation(number, style, 0)
            .or_else(|| {
                let fallback = match self.rules.get(style) {
                    Some(rule) => descriptor(rule, "fallback").unwrap_or("decimal").trim().to_owned(),
                    None => predefined_fallback(&style.to_ascii_lowercase()).to_owned(),
                };
                self.representation(number, &fallback, 0)
            })
            .unwrap_or_else(|| number.to_string())
    }

    pub(crate) fn marker(&self, number: i64, style: &str) -> String {
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
            let Some(parent) = descriptor(rule, "system")
                .and_then(|value| value.trim().strip_prefix("extends "))
            else {
                break;
            };
            current = parent.trim();
        }
        let suffix = suffix
            .unwrap_or_else(|| predefined_suffix(&style.to_ascii_lowercase()).to_owned());
        format!("{}{}{}", prefix.unwrap_or_default(), self.format(number, style), suffix)
    }

    fn representation(&self, number: i64, style: &str, depth: usize) -> Option<String> {
        if depth >= 16 {
            return None;
        }
        let Some(rule) = self.rules.get(style) else {
            return predefined_representation(number, &style.to_ascii_lowercase());
        };
        let system = descriptor(rule, "system").unwrap_or("symbolic").trim();
        if let Some(parent) = system.strip_prefix("extends ") {
            return self.representation(number, parent.trim(), depth + 1);
        }
        let fallback = descriptor(rule, "fallback").unwrap_or("decimal").trim().to_owned();
        let symbols = descriptor(rule, "symbols")
            .map(parse_symbols)
            .unwrap_or_default();
        let magnitude = number.abs();
        let body = if system == "numeric" && symbols.len() >= 2 {
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
            Some(out.into_iter().rev().collect::<String>())
        } else if system == "alphabetic" && symbols.len() >= 2 && number > 0 {
            let radix = i64::try_from(symbols.len()).ok()?;
            let mut value = magnitude;
            let mut out = Vec::new();
            while value > 0 {
                value -= 1;
                out.push(symbols[usize::try_from(value % radix).ok()?].clone());
                value /= radix;
            }
            Some(out.into_iter().rev().collect::<String>())
        } else if system == "cyclic" && !symbols.is_empty() {
            let radix = i64::try_from(symbols.len()).ok()?;
            Some(symbols[usize::try_from((number - 1).rem_euclid(radix)).ok()?].clone())
        } else if system == "symbolic" && !symbols.is_empty() && number > 0 {
            let radix = i64::try_from(symbols.len()).ok()?;
            let repeats = usize::try_from((magnitude - 1) / radix + 1).ok()?;
            if repeats > 1024 {
                return None;
            }
            Some(symbols[usize::try_from((magnitude - 1) % radix).ok()?].repeat(repeats))
        } else if system.starts_with("fixed") && !symbols.is_empty() {
            let start = system
                .strip_prefix("fixed")
                .unwrap_or("")
                .trim()
                .parse::<i64>()
                .unwrap_or(1);
            symbols
                .get(usize::try_from(number - start).ok()?)?
                .clone()
                .into()
        } else if system == "additive" {
            let tuples = descriptor(rule, "additive-symbols")
                .map(parse_additive)
                .unwrap_or_default();
            if tuples.is_empty() {
                return self.representation(number, &fallback, depth + 1);
            }
            additive_body(
                number,
                tuples.iter().map(|(w, s)| (*w, s.as_str())).collect(),
            )
        } else {
            None
        };
        let Some(body) = body else {
            return self.representation(number, &fallback, depth + 1);
        };
        let padded = if let Some((minimum, pad)) = descriptor(rule, "pad").and_then(parse_pad) {
            format!(
                "{}{body}",
                pad.repeat(minimum.saturating_sub(body.chars().count()).min(1024))
            )
        } else {
            body
        };
        if number < 0 {
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

/// Parse `additive-symbols: 1000 m, 900 cm, ...` preserving source order.
fn parse_additive(input: &str) -> Vec<(i32, String)> {
    let mut out = Vec::new();
    let mut pending: Option<i32> = None;
    for token in tokenize(input) {
        match token {
            Token::Whitespace | Token::Comma => {}
            Token::Number(weight) if pending.is_none() => {
                pending = Some(weight as i32);
            }
            Token::Str(value) | Token::Ident(value) => {
                if let Some(weight) = pending.take() {
                    out.push((weight, value));
                }
            }
            _ => {}
        }
    }
    out
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
    use super::{format_counter, predefined_suffix};

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

    #[test]
    fn additive_predefined_styles_match_the_spec() {
        for (number, style, expected) in [
            (1, "armenian", "Ա"),
            (9999, "armenian", "ՔՋՂԹ"),
            (10000, "armenian", "10000"),
            (2, "lower-armenian", "բ"),
            (1, "georgian", "ა"),
            (15, "hebrew", "טו"),
            (16, "hebrew", "טז"),
            (20, "hebrew", "כ"),
        ] {
            assert_eq!(format_counter(number, style), expected);
        }
    }

    #[test]
    fn cjk_longhand_styles_match_the_spec_examples() {
        for (number, style, expected) in [
            (0, "japanese-informal", "〇"),
            (10, "japanese-informal", "十"),
            (11, "japanese-informal", "十一"),
            (99, "japanese-informal", "九十九"),
            (100, "japanese-informal", "百"),
            (101, "japanese-informal", "百一"),
            (6001, "japanese-informal", "六千一"),
            (1000, "japanese-formal", "壱阡"),
            (1, "simp-chinese-informal", "一"),
            (10, "simp-chinese-informal", "十"),
            (11, "simp-chinese-informal", "十一"),
            (20, "simp-chinese-informal", "二十"),
            (100, "simp-chinese-informal", "一百"),
            (101, "simp-chinese-informal", "一百零一"),
            (110, "simp-chinese-informal", "一百一十"),
            (1000, "simp-chinese-informal", "一千"),
            (1010, "simp-chinese-informal", "一千零一十"),
            (6001, "simp-chinese-informal", "六千零一"),
            (10, "simp-chinese-formal", "壹拾"),
            (11, "simp-chinese-formal", "壹拾壹"),
            (99, "simp-chinese-formal", "玖拾玖"),
            (101, "simp-chinese-formal", "壹佰零壹"),
            (10, "trad-chinese-formal", "壹拾"),
            (-20, "simp-chinese-informal", "负二十"),
            // Extended algorithm (§7.1.2) from counter-trad-chinese-informal-ref.html.
            (10000, "trad-chinese-informal", "一萬"),
            (10001, "trad-chinese-informal", "一萬零一"),
            (10010, "trad-chinese-informal", "一萬零十"),
            (10100, "trad-chinese-informal", "一萬零一百"),
            (11000, "trad-chinese-informal", "一萬一千"),
            (100000000, "trad-chinese-informal", "一億"),
            (100000001, "trad-chinese-informal", "一億零一"),
            (1000000000, "trad-chinese-informal", "十億"),
            (123456789, "trad-chinese-informal", "一億二千三百四十五萬六千七百八十九"),
            (1000000000000, "trad-chinese-informal", "一兆"),
            (10000000000000, "trad-chinese-informal", "十兆"),
            (
                9999999999999999,
                "trad-chinese-informal",
                "九千九百九十九兆九千九百九十九億九千九百九十九萬九千九百九十九",
            ),
            (-10000, "trad-chinese-informal", "負一萬"),
            (10000, "simp-chinese-formal", "壹万"),
            (123456789, "simp-chinese-formal", "壹亿贰仟叁佰肆拾伍万陆仟柒佰捌拾玖"),
            (1000000000000, "simp-chinese-formal", "壹万亿"),
            (10000000000000000, "trad-chinese-informal", "一〇〇〇〇〇〇〇〇〇〇〇〇〇〇〇〇"),
        ] {
            assert_eq!(format_counter(number, style), expected, "{number} {style}");
        }
    }

    #[test]
    fn cjk_styles_use_their_spec_suffixes() {
        assert_eq!(predefined_suffix("cjk-decimal"), "、");
        assert_eq!(predefined_suffix("hiragana"), "、");
        assert_eq!(predefined_suffix("cjk-earthly-branch"), "、");
        assert_eq!(predefined_suffix("simp-chinese-formal"), "、");
        assert_eq!(predefined_suffix("korean-hangul-formal"), ", ");
        assert_eq!(predefined_suffix("decimal"), ". ");
        assert_eq!(predefined_suffix("lower-roman"), ". ");
    }
}
