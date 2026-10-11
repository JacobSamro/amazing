//! Std-only CSS Text 3 `text-transform` character mapping (language independent).
//! Word starts for `capitalize` use a simple alphanumeric-run rule that keeps an
//! internal apostrophe inside the word; no segmentation tables are needed.
use crate::{TextTransform, TextTransformCase};

pub fn map_text_transform(text: &str, transform: TextTransform) -> String {
    if transform == TextTransform::default() {
        return text.to_string();
    }
    let mut out = match transform.case {
        TextTransformCase::None => text.to_string(),
        TextTransformCase::Uppercase => text.chars().flat_map(char::to_uppercase).collect(),
        TextTransformCase::Lowercase => lowercase(text),
        TextTransformCase::Capitalize => capitalize(text),
    };
    if transform.full_width {
        out = out
            .chars()
            .map(|ch| match ch {
                ' ' => '\u{3000}',
                '\u{21}'..='\u{7e}' => char::from_u32(ch as u32 + 0xFEE0).unwrap_or(ch),
                _ => ch,
            })
            .collect();
    }
    if transform.full_size_kana {
        out = full_size_kana(&out);
    }
    out
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn lowercase(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '\u{03A3}' {
            // Final sigma when preceded by a letter and not followed by one.
            let before = i > 0 && chars[i - 1].is_alphabetic();
            let after = chars.get(i + 1).is_some_and(|c| c.is_alphabetic());
            out.push(if before && !after { '\u{03C2}' } else { '\u{03C3}' });
        } else {
            out.extend(ch.to_lowercase());
        }
    }
    out
}

fn capitalize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_word = false;
    for (i, &ch) in chars.iter().enumerate() {
        if in_word {
            let joiner = matches!(ch, '\'' | '\u{2019}')
                && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
            if !is_word_char(ch) && !joiner {
                in_word = false;
            }
            out.push(ch);
        } else if ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            in_word = true;
        } else {
            in_word = is_word_char(ch);
            out.push(ch);
        }
    }
    out
}

/// CSS Text 3 `full-size-kana`: small kana become the corresponding
/// full-size kana (language independent, one scalar to one scalar).
fn full_size_kana(text: &str) -> String {
    text.chars().map(full_size_kana_char).collect()
}

fn full_size_kana_char(ch: char) -> char {
    match ch {
        'ぁ' => 'あ', 'ぃ' => 'い', 'ぅ' => 'う', 'ぇ' => 'え', 'ぉ' => 'お',
        'っ' => 'つ', 'ゃ' => 'や', 'ゅ' => 'ゆ', 'ょ' => 'よ', 'ゎ' => 'わ',
        'ゕ' => 'か', 'ゖ' => 'け',
        'ァ' => 'ア', 'ィ' => 'イ', 'ゥ' => 'ウ', 'ェ' => 'エ', 'ォ' => 'オ',
        'ッ' => 'ツ', 'ャ' => 'ヤ', 'ュ' => 'ユ', 'ョ' => 'ヨ', 'ヮ' => 'ワ',
        'ヵ' => 'カ', 'ヶ' => 'ケ',
        'ㇰ' => 'ク', 'ㇱ' => 'シ', 'ㇲ' => 'ス', 'ㇳ' => 'ト', 'ㇴ' => 'ヌ',
        'ㇵ' => 'ハ', 'ㇶ' => 'ヒ', 'ㇷ' => 'フ', 'ㇸ' => 'ヘ', 'ㇹ' => 'ホ',
        'ㇺ' => 'ム', 'ㇻ' => 'ラ', 'ㇼ' => 'リ', 'ㇽ' => 'ル', 'ㇾ' => 'レ', 'ㇿ' => 'ロ',
        'ｧ' => 'ｱ', 'ｨ' => 'ｲ', 'ｩ' => 'ｳ', 'ｪ' => 'ｴ', 'ｫ' => 'ｵ',
        'ｬ' => 'ﾔ', 'ｭ' => 'ﾕ', 'ｮ' => 'ﾖ', 'ｯ' => 'ﾂ',
        '\u{1B132}' => 'こ', '\u{1B150}' => 'ゐ', '\u{1B151}' => 'ゑ', '\u{1B152}' => 'を',
        '\u{1B155}' => 'コ', '\u{1B164}' => 'ヰ', '\u{1B165}' => 'ヱ', '\u{1B166}' => 'ヲ', '\u{1B167}' => 'ン',
        _ => ch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(case: TextTransformCase, w: bool, k: bool) -> TextTransform {
        TextTransform { case, full_width: w, full_size_kana: k }
    }
    #[test]
    fn kana_and_width() {
        assert_eq!(map_text_transform("ぁっゕゖァッㇰｯあA", t(TextTransformCase::None, false, true)), "あつかけアツクﾂあA");
        assert_eq!(map_text_transform("a b", t(TextTransformCase::None, true, false)), "ａ\u{3000}ｂ");
    }
    #[test]
    fn cases() {
        assert_eq!(map_text_transform("don't stop-me ΟΔΟΣ", t(TextTransformCase::Capitalize, false, false)), "Don't Stop-Me ΟΔΟΣ");
        assert_eq!(map_text_transform("ΟΔΟΣ ΣΑ", t(TextTransformCase::Lowercase, false, false)), "οδος σα");
        assert_eq!(map_text_transform("straße", t(TextTransformCase::Uppercase, false, false)), "STRASSE");
    }
}
