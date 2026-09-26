use super::*;
fn captures(pattern: &str, flags: &str, input: &str) -> Vec<Option<String>> {
    let r = Regex::new(pattern, flags)
        .unwrap()
        .exec(input, 0)
        .unwrap()
        .unwrap();
    let input: Vec<u16> = input.encode_utf16().collect();
    r.captures
        .into_iter()
        .map(|r| r.map(|r| String::from_utf16_lossy(&input[r])))
        .collect()
}
#[test]
fn alternation_and_backtracking() {
    assert_eq!(
        captures("(a|ab)c", "", "zabc"),
        vec![Some("abc".into()), Some("ab".into())]
    );
    assert_eq!(captures("a+?a", "", "aaaa")[0], Some("aa".into()));
    assert_eq!(captures("a+a", "", "aaaa")[0], Some("aaaa".into()));
    assert_eq!(captures("a{2,4}a", "", "aaaaaa")[0], Some("aaaaa".into()));
}
#[test]
fn capture_reset_and_empty_repetitions() {
    assert_eq!(
        captures("(z)((a+)?(b+)?(c))*", "", "zaacbbbcac"),
        [
            Some("zaacbbbcac"),
            Some("z"),
            Some("ac"),
            Some("a"),
            None,
            Some("c")
        ]
        .map(|s| s.map(str::to_string))
    );
    assert_eq!(captures("(a*)*", "", "b"), vec![Some("".into()), None]);
    assert_eq!(
        captures("(a?)+", "", ""),
        vec![Some("".into()), Some("".into())]
    );
}
#[test]
fn captures_and_lookahead_are_atomic() {
    assert_eq!(
        captures(r"(a+)b\1", "", "aaabaaa")[0],
        Some("aaabaaa".into())
    );
    assert_eq!(
        captures(r"(?=(a+))a*b\1", "", "baabac")[0],
        Some("aba".into())
    );
    assert!(Regex::new("a(?!b)", "")
        .unwrap()
        .exec("ab", 0)
        .unwrap()
        .is_none());
    assert_eq!(captures(r"(a|(b))\2c", "", "ac")[2], None);
}
#[test]
fn classes_and_anchors() {
    assert_eq!(
        captures(r"^[a-z\d_]+$", "i", "aZ_9")[0],
        Some("aZ_9".into())
    );
    assert!(Regex::new("^a$", "")
        .unwrap()
        .exec("a\n", 0)
        .unwrap()
        .is_none());
    assert!(Regex::new("^a$", "m")
        .unwrap()
        .exec("x\na\nx", 0)
        .unwrap()
        .is_some());
    assert!(Regex::new("a.b", "")
        .unwrap()
        .exec("a\nb", 0)
        .unwrap()
        .is_none());
    assert!(Regex::new("a.b", "s")
        .unwrap()
        .exec("a\nb", 0)
        .unwrap()
        .is_some());
    assert!(Regex::new("[]", "")
        .unwrap()
        .exec("a", 0)
        .unwrap()
        .is_none());
    assert!(Regex::new("[^]", "")
        .unwrap()
        .exec("\n", 0)
        .unwrap()
        .is_some());
}
#[test]
fn utf16_and_unicode_modes() {
    assert_eq!(
        Regex::new(".", "")
            .unwrap()
            .exec("😀", 0)
            .unwrap()
            .unwrap()
            .range,
        0..1
    );
    assert_eq!(
        Regex::new(".", "u")
            .unwrap()
            .exec("😀", 0)
            .unwrap()
            .unwrap()
            .range,
        0..2
    );
    assert_eq!(
        Regex::new(r"\uD83D\uDE00", "u")
            .unwrap()
            .exec("😀", 0)
            .unwrap()
            .unwrap()
            .range,
        0..2
    );
    assert_eq!(
        Regex::new(r"\u{1F600}", "u")
            .unwrap()
            .exec("a😀", 0)
            .unwrap()
            .unwrap()
            .range,
        1..3
    );
    assert!(Regex::new(r"[\ud800-\udbff](?![\udc00-\udfff])", "")
        .unwrap()
        .exec_utf16(&[0xD800], 0)
        .unwrap()
        .is_some());
}
#[test]
fn utf16_pattern_preserves_lone_surrogates() {
    let high = Regex::new_utf16(&[0xD800], "").unwrap();
    let low = Regex::new_utf16(&[0xDC00], "").unwrap();
    assert_eq!(high.pattern_utf16(), &[0xD800]);
    assert_eq!(low.pattern_utf16(), &[0xDC00]);
    assert_ne!(high, Regex::new("\u{FFFD}", "").unwrap());
    assert_eq!(high.exec_utf16(&[0xD800], 0).unwrap().unwrap().range, 0..1);
    assert_eq!(low.exec_utf16(&[0xDC00], 0).unwrap().unwrap().range, 0..1);
    assert!(high.exec_utf16(&[0xDC00], 0).unwrap().is_none());
    assert_eq!(
        high.exec_utf16(&[0xD800, 0xDC00], 0)
            .unwrap()
            .unwrap()
            .range,
        0..1
    );
}
#[test]
fn utf16_pattern_unicode_modes_combine_pairs_but_preserve_lone_units() {
    for flags in ["u", "v"] {
        let pair = Regex::new_utf16(&[0xD83D, 0xDE00], flags).unwrap();
        assert_eq!(pair.pattern_utf16(), &[0xD83D, 0xDE00]);
        assert_eq!(
            pair.exec_utf16(&[0xD83D, 0xDE00], 0)
                .unwrap()
                .unwrap()
                .range,
            0..2
        );
        let lone = Regex::new_utf16(&[0xD800], flags).unwrap();
        assert_eq!(lone.exec_utf16(&[0xD800], 0).unwrap().unwrap().range, 0..1);
        assert!(lone.exec_utf16(&[0xD800, 0xDC00], 0).unwrap().is_none());
        let escaped = Regex::new_utf16(
            r"\uD800".encode_utf16().collect::<Vec<_>>().as_slice(),
            flags,
        )
        .unwrap();
        assert_eq!(
            escaped.exec_utf16(&[0xD800], 0).unwrap().unwrap().range,
            0..1
        );
    }
    let legacy = Regex::new_utf16(&[0xD83D, 0xDE00], "").unwrap();
    assert_eq!(
        legacy
            .exec_utf16(&[0xD83D, 0xDE00], 0)
            .unwrap()
            .unwrap()
            .range,
        0..2
    );
    assert!(Regex::new_utf16(&[0xD800], "uv").is_err());
    assert!(Regex::new_utf16(&[0xD800], "vv").is_err());
}
#[test]
fn unicode_casefold_and_legacy_restrictions() {
    for (pattern, input) in [
        ("k", "K"),
        ("s", "ſ"),
        ("σ", "ς"),
        ("ß", "ẞ"),
        ("[A-Z]", "K"),
        ("[K]", "K"),
    ] {
        assert!(
            Regex::new(pattern, "iu")
                .unwrap()
                .exec(input, 0)
                .unwrap()
                .is_some(),
            "{pattern} {input}"
        );
    }
    assert!(Regex::new("k", "i")
        .unwrap()
        .exec("K", 0)
        .unwrap()
        .is_none());
    assert!(Regex::new(r"\b\w+\b", "iu")
        .unwrap()
        .exec("K", 0)
        .unwrap()
        .is_some());
    assert!(Regex::new("[a-z]", "i")
        .unwrap()
        .exec("A", 0)
        .unwrap()
        .is_some());
}
#[test]
fn sticky_and_search_indices() {
    assert!(Regex::new("a", "y")
        .unwrap()
        .exec("ba", 0)
        .unwrap()
        .is_none());
    assert_eq!(
        Regex::new("a", "y")
            .unwrap()
            .exec("ba", 1)
            .unwrap()
            .unwrap()
            .range,
        1..2
    );
    assert_eq!(
        Regex::new("a", "g")
            .unwrap()
            .exec("ba", 0)
            .unwrap()
            .unwrap()
            .range,
        1..2
    );
}
#[test]
fn syntax_errors_and_bounds() {
    for (p, f) in [
        ("(", ""),
        ("[z-a]", ""),
        ("a{3,2}", ""),
        ("a", "gg"),
        ("a", "uv"),
        (r"\a", "u"),
        ("(?<x>a)", ""),
        (r"\p{Letter}", "u"),
    ] {
        assert!(Regex::new(p, f).is_err(), "{p}/{f}");
    }
    assert!(matches!(
        Regex::new("a{999999999}", ""),
        Err(Error::Limit(_))
    ));
    assert!(matches!(
        Regex::new("(a+)+$", "")
            .unwrap()
            .exec(&("a".repeat(28) + "!"), 0),
        Err(Error::Limit(_))
    ));
}

#[test]
fn unicode_sets_operations_strings_and_properties() {
    for property in [
        "Basic_Emoji",
        "Emoji_Keycap_Sequence",
        "RGI_Emoji",
        "RGI_Emoji_Flag_Sequence",
        "RGI_Emoji_Modifier_Sequence",
        "RGI_Emoji_Tag_Sequence",
        "RGI_Emoji_ZWJ_Sequence",
    ] {
        assert!(Regex::new(&format!(r"\p{{{property}}}"), "u").is_err());
    }
    let accepts = [
        (r"^[a-z--[aeiou]]+$", "bcdf"),
        (r"^[[a-z]&&[def]]+$", "fed"),
        (r"^[\q{ab|a}]+$", "aba"),
        (r"^[\q{9\uFE0F\u20E3|9}]+$", "9\u{fe0f}\u{20e3}"),
        (r"^\p{Emoji_Keycap_Sequence}$", "9\u{fe0f}\u{20e3}"),
        (r"^[\p{ASCII_Hex_Digit}--[0-9]]+$", "ABcd"),
        (r"^\p{RGI_Emoji}$", "👩‍💻"),
        (r"^[\q{ab|a}]b$", "ab"),
    ];
    for (pattern, input) in accepts {
        assert!(
            Regex::new(pattern, "v")
                .unwrap()
                .exec(input, 0)
                .unwrap()
                .is_some(),
            "{pattern}"
        );
    }
    let rejects = [
        (r"^[a-z--[aeiou]]+$", "abc"),
        (r"^[[a-z]&&[def]]+$", "abc"),
        (r"^[\q{ab|a}]+$", "ac"),
        (r"^\p{Emoji_Keycap_Sequence}$", "9"),
        (r"^[\p{ASCII_Hex_Digit}--[0-9]]+$", "9"),
        (r"^[\q{ab}&&\P{ASCII_Hex_Digit}]$", "ab"),
    ];
    for (pattern, input) in rejects {
        assert!(
            Regex::new(pattern, "v")
                .unwrap()
                .exec(input, 0)
                .unwrap()
                .is_none(),
            "{pattern}"
        );
    }
    for pattern in [
        r"[a&&]",
        r"[a--]",
        r"[ab&&c]",
        r"[a&&b--c]",
        r"[^\q{ab}]",
        r"[a-b-c]",
        r"[!!]",
        r"[$$]",
        r"[**]",
        r"[++]",
        r"[..]",
        r"[::]",
        r"[??]",
        r"[^^^]",
        r"[_^^]",
    ] {
        assert!(Regex::new(pattern, "v").is_err(), "{pattern}");
    }
}
#[test]
fn upstream_testharness_patterns() {
    assert_eq!(
        captures(r"^\(\)\s*=>\s*(?:{(.*)}\s*|(.*))$", "", "() => { pass(); }")[1],
        Some(" pass(); ".into())
    );
    assert_eq!(
        captures(r"^([^;]*)(;\s*)+$", "", "pass();  ")[1],
        Some("pass()".into())
    );
    assert!(Regex::new(r#"@?\w+:\/\/[^ "\/]+(?::\d+)?"#, "g")
        .unwrap()
        .exec("https://example.test:443", 0)
        .unwrap()
        .is_some());
}

#[test]
fn shared_budget_and_compile_limits_fail_closed() {
    assert!(matches!(
        Regex::new("(?:(?:){32768}){32768}", ""),
        Err(Error::Limit(_))
    ));
    let regex = Regex::new("(?:)", "g").unwrap();
    let mut matcher = regex.matcher_utf16(&[]).unwrap();
    let mut exhausted = false;
    for _ in 0..MAX_STEPS {
        if matcher.exec(0).is_err() {
            exhausted = true;
            break;
        }
    }
    assert!(exhausted);
    assert!(matches!(matcher.exec(0), Err(Error::Limit(_))));
    assert!(Regex::new(r"\u{D83D}\uDE00", "u")
        .unwrap()
        .exec("😀", 0)
        .unwrap()
        .is_none());
}

#[test]
fn case_insensitive_class_sequences_fit_the_compiler_budget() {
    let pattern = format!("^{}$", "[a-z]".repeat(32));
    for flags in ["i", "iu", "iv"] {
        let regex = Regex::new(&pattern, flags).unwrap();
        assert!(regex.exec(&"A".repeat(32), 0).unwrap().is_some());
        assert!(regex.exec(&format!("{}0", "A".repeat(31)), 0).unwrap().is_none());
    }
}

#[test]
fn indexed_class_folding_preserves_unicode_and_legacy_membership() {
    for flags in ["i", "iu"] {
        for (pattern, input, expected) in [
            ("^[a-z]+$", "AZaz", true),
            ("^[a-z]+$", "Kſ", flags == "iu"),
            ("^[^a-z]+$", "Kſ", flags == "i"),
            ("^[Σ]+$", "Σσς", true),
            ("^[\\uD800]+$", "A", false),
            ("^[a-zA-Z\\d]+$", "aA0", true),
            ("^[\\D]+$", "A", true),
            ("^[\\D]+$", "0", false),
            ("^(?=[a-z])[a-z]$", "A", true),
        ] {
            let regex = Regex::new(pattern, flags).unwrap();
            assert_eq!(
                regex.exec(input, 0).unwrap().is_some(),
                expected,
                "{pattern:?} / {flags} / {input:?}"
            );
        }
    }
    // Large repeated ranges still consume a bounded compilation budget.
    let expensive = "[\\u0000-\\uFFFF]".repeat(32);
    assert!(matches!(Regex::new(&expensive, "iu"), Err(Error::Limit(_))));
}

#[test]
fn annex_b_numeric_escapes_choose_backrefs_before_octal() {
    for (pattern, text) in [(r"\1", "\x01"), (r"\07", "\x07"), (r"\377", "\u{ff}"),
        (r"\400", " 0"), (r"\770", "?0"), (r"\0111", "\t1"), (r"\8", "8"), (r"\90", "90"),
        (r"[\1]", "\x01"), (r"[\8]", "8")] {
        assert_eq!(captures(pattern, "", text)[0], Some(text.into()), "{pattern}");
    }
    assert_eq!(captures(r"(.)\1", "", "aa")[0], Some("aa".into()));
    assert_eq!(captures(r"\1(a)", "", "a")[0], Some("a".into()));
    assert_eq!(captures(r"(.)(.)(.)(.)(.)(.)(.)(.)\8", "", "123456788")[0], Some("123456788".into()));
    assert_eq!(captures(r"\1[(]", "", "\x01(")[0], Some("\x01(".into()));
    assert_eq!(captures(r"\1\(", "", "\x01(")[0], Some("\x01(".into()));
    for flags in ["u", "v"] {
        for pattern in [r"\1", r"\07", r"\8", r"[\1]", r"[\8]"] {
            assert!(Regex::new(pattern, flags).is_err(), "{pattern}/{flags}");
        }
        assert_eq!(captures(r"\1(a)", flags, "a")[0], Some("a".into()));
    }
    let long_identity = format!("\\{}", "9".repeat(100));
    assert!(Regex::new(&long_identity, "").is_ok());
}

#[test]
fn annex_b_identity_control_and_class_range_fallbacks() {
    for (pattern, text) in [(r"\xQ", "xQ"), (r"\u12Q", "u12Q"), (r"\p\P\k", "pPk"),
        (r"\c0", r"\c0"), (r"\c_", r"\c_"), (r"[\c0]", "\x10"), (r"[\c_]", "\x1f"),
        (r"[\c!]", "\\"), (r"[\c!]", "c"), (r"[\c!]", "!")] {
        assert_eq!(captures(pattern, "", text)[0], Some(text.into()), "{pattern}");
    }
    for pattern in [r"[\d-%]+", r"[%-\d]+", r"[\s-\d]+", r"[--\d]+"] {
        assert!(Regex::new(pattern, "").is_ok(), "{pattern}");
        assert!(Regex::new(pattern, "u").is_err(), "{pattern}");
    }
    assert_eq!(captures(r"[\s-\d]+", "", "x \t-012x")[0], Some(" \t-012".into()));
    for flags in ["", "u", "v"] { assert!(Regex::new("[z-a]", flags).is_err()); }
    for pattern in [r"\xQ", r"\u12Q", r"\c0", r"[\c0]", r"[\c_]", r"[\c!]"] {
        assert!(Regex::new(pattern, "u").is_err(), "{pattern}");
    }
}

#[test]
fn annex_b_quantified_assertions_preserve_zero_width_semantics() {
    for quantifier in ["*", "+", "?", "{2}", "{2,}", "{2,3}", "*?", "+?", "??", "{2}?", "{2,}?", "{2,3}?"] {
        let optional = quantifier.starts_with('*') || quantifier.starts_with('?');
        let positive = format!(".(?=Z){quantifier}");
        let negative = format!("[a-e](?!Z){quantifier}");
        assert_eq!(captures(&positive, "", "a bZ cZZ")[0], Some(if optional { "a" } else { "b" }.into()));
        assert_eq!(captures(&negative, "", "aZZ bZ e")[0], Some(if optional { "a" } else { "e" }.into()));
        assert!(Regex::new(&positive, "u").is_err());
        assert!(Regex::new(&negative, "v").is_err());
    }
    assert_eq!(captures(r"(?=(a))?a", "", "a"), vec![Some("a".into()), None]);
    assert_eq!(captures(r"(?=(a)){2}a", "", "a"), vec![Some("a".into()), Some("a".into())]);
    assert_eq!(captures(r"(a)(?=(b))+b", "", "ab"),
        vec![Some("ab".into()), Some("a".into()), Some("b".into())]);
    // A noncapturing group is an Atom even when its contents are assertions.
    for flags in ["", "u", "v"] {
        assert_eq!(captures(r"(?:(?=(a)))+a", flags, "a"),
            vec![Some("a".into()), Some("a".into())]);
        assert_eq!(captures(r"(?:^){2}a", flags, "a")[0], Some("a".into()));
        assert_eq!(captures(r"(?:\b){2}a", flags, "a")[0], Some("a".into()));
    }
    for pattern in ["^*", "$+", r"\b?", r"\B{2}"] { assert!(Regex::new(pattern, "").is_err()); }
}
