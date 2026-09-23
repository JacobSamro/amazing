//! Character reference tables (WHATWG §13.2.5 "named/numeric references").
//!
//! Phase 1 ships a pragmatic subset: the numeric syntax in full, plus the
//! named entities real-world pages actually use. The full 2 231-entry named
//! table is a data-generation task (mach `make_entities` from the spec's
//! JSON) deferred to a follow-up; missing names fall through as literal text,
//! which is exactly what browsers do for unknown-but-wellformed-looking refs.

/// Bounds the longest known named entity (e.g. `&thetasym;` is 10 chars) and
/// doubles as the tokenizer's matching-window size.
pub const MAX_NAME_LEN: usize = 12;

/// `(name including semicolon, replacement)` — probed longest-first.
pub const NAMED: &[(&str, &str)] = &[
    ("&nbsp;", "\u{00A0}"),
    ("&NewLine;", "\u{000A}"),
    ("&AMP;", "&"),
    ("&AElig;", "\u{00C6}"),
    ("&aacute;", "\u{00E1}"),
    ("&Acirc;", "\u{00C2}"),
    ("&acirc;", "\u{00E2}"),
    ("&agrave;", "\u{00E0}"),
    ("&amp;", "&"),
    ("&Aring;", "\u{00C5}"),
    ("&atilde;", "\u{00E3}"),
    ("&auml;", "\u{00E4}"),
    ("&brvbar;", "\u{00A6}"),
    ("&ccedil;", "\u{00E7}"),
    ("&cedil;", "\u{00B8}"),
    ("&cent;", "\u{00A2}"),
    ("&COPY;", "\u{00A9}"),
    ("&copy;", "\u{00A9}"),
    ("&curren;", "\u{00A4}"),
    ("&deg;", "\u{00B0}"),
    ("&divide;", "\u{00F7}"),
    ("&eacute;", "\u{00E9}"),
    ("&Eacute;", "\u{00C9}"),
    ("&egrave;", "\u{00E8}"),
    ("&Egrave;", "\u{00C8}"),
    ("&ecirc;", "\u{00EA}"),
    ("&euml;", "\u{00EB}"),
    ("&frac12;", "\u{00BD}"),
    ("&frac14;", "\u{00BC}"),
    ("&GT;", ">"),
    ("&gt;", ">"),
    ("&iacute;", "\u{00ED}"),
    ("&Igrave;", "\u{00CC}"),
    ("&iexcl;", "\u{00A1}"),
    ("&laquo;", "\u{00AB}"),
    ("&ldquo;", "\u{201C}"),
    ("&lsquo;", "\u{2018}"),
    ("&LT;", "<"),
    ("&lt;", "<"),
    ("&mdash;", "\u{2014}"),
    ("&middot;", "\u{00B7}"),
    ("&ndash;", "\u{2013}"),
    ("&not;", "\u{00AC}"),
    ("&notin;", "\u{2209}"),
    ("&ntilde;", "\u{00F1}"),
    ("&Ntilde;", "\u{00D1}"),
    ("&oacute;", "\u{00F3}"),
    ("&ordf;", "\u{00AA}"),
    ("&ordm;", "\u{00BA}"),
    ("&oslash;", "\u{00F8}"),
    ("&otilde;", "\u{00F5}"),
    ("&ouml;", "\u{00F6}"),
    ("&Ouml;", "\u{00D6}"),
    ("&para;", "\u{00B6}"),
    ("&plusmn;", "\u{00B1}"),
    ("&pound;", "\u{00A3}"),
    ("&prod;", "\u{220F}"),
    ("&QUOT;", "\""),
    ("&quot;", "\""),
    ("&raquo;", "\u{00BB}"),
    ("&rdquo;", "\u{201D}"),
    ("&REG;", "\u{00AE}"),
    ("&reg;", "\u{00AE}"),
    ("&rsquo;", "\u{2019}"),
    ("&sect;", "\u{00A7}"),
    ("&shy;", "\u{00AD}"),
    ("&sup2;", "\u{00B2}"),
    ("&sup3;", "\u{00B3}"),
    ("&szlig;", "\u{00DF}"),
    ("&Tab;", "\u{0009}"),
    ("&thetasym;", "\u{03D1}"),
    ("&times;", "\u{00D7}"),
    ("&trade;", "\u{2122}"),
    ("&Uuml;", "\u{00DC}"),
    ("&uuml;", "\u{00FC}"),
    ("&yacute;", "\u{00FD}"),
    ("&yen;", "\u{00A5}"),
    ("&zwnj;", "\u{200C}"),
    ("&zwj;", "\u{200D}"),
];

/// Legacy named references (WHATWG table of legacy entities). The tokenizer's
/// rule: legacy refs decode without a semicolon in text; in attribute values
/// they must NOT decode when immediately followed by `=` or an ASCII
/// alphanumeric.
pub const LEGACY_NO_SEMI: &[(&str, &str)] = &[
    ("&AElig", "\u{00C6}"),
    ("&AMP", "&"),
    ("&COPY", "\u{00A9}"),
    ("&GT", ">"),
    ("&LT", "<"),
    ("&REG", "\u{00AE}"),
    ("&QUOT", "\""),
    ("&amp", "&"),
    ("&copy", "\u{00A9}"),
    ("&gt", ">"),
    ("&lt", "<"),
    ("&not", "\u{00AC}"),
    ("&pound", "\u{00A3}"),
    ("&quot", "\""),
    ("&reg", "\u{00AE}"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_table_has_no_duplicate_names() {
        let mut seen: Vec<&str> = Vec::new();
        for (name, _) in NAMED.iter().chain(LEGACY_NO_SEMI) {
            assert!(!seen.contains(name), "duplicate entity {name}");
            seen.push(name);
        }
    }

    #[test]
    fn max_len_covers_every_name() {
        for (name, _) in NAMED.iter().chain(LEGACY_NO_SEMI) {
            assert!(name.len() <= MAX_NAME_LEN, "{name} exceeds MAX_NAME_LEN");
        }
    }

    #[test]
    fn semicolon_names_are_supersets_of_legacy() {
        // Every legacy name with ';' appended resolves via the main table.
        for (legacy, val) in LEGACY_NO_SEMI {
            let with_semi = format!("{legacy};");
            let hit = NAMED.iter().any(|(n, v)| *n == with_semi && *v == *val);
            assert!(hit, "{with_semi} missing from NAMED");
        }
    }
}
