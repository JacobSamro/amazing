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
        ("(?<x>a)(?<x>b)", ""),
        ("(?<1a>a)", ""),
        (r"\p{Bogus_Property}", "u"),
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
fn unicode_property_escapes_general_category_script_and_binary() {
    let accepts = [
        (r"^\p{L}+$", "Hello"),
        (r"^\p{Letter}+$", "Hello"),
        (r"^\p{Lu}+$", "HELLO"),
        (r"^\p{Uppercase_Letter}+$", "HELLO"),
        (r"^\p{General_Category=Lu}+$", "HELLO"),
        (r"^\p{gc=Lu}+$", "HELLO"),
        (r"^\p{LC}+$", "Hello"),
        (r"^\p{Cased_Letter}+$", "Hello"),
        (r"^\p{Nd}+$", "0123"),
        (r"^\p{Script=Greek}+$", "\u{3b1}\u{3b2}\u{3b3}"),
        (r"^\p{Script=Latin}+$", "abc"),
        (r"^\p{sc=Latn}+$", "abc"),
        // Common has no Script_Extensions overrides for digits, so the
        // Script_Extensions escape must fall back to the plain Script value.
        (r"^\p{Script_Extensions=Common}+$", "0123"),
        // U+0300 COMBINING GRAVE ACCENT is explicitly listed with several
        // scripts in ScriptExtensions.txt, including Greek, though its own
        // Script is Inherited.
        (r"^\p{scx=Greek}+$", "\u{300}"),
        (r"^\p{Alphabetic}+$", "abcABC"),
        (r"^\p{Alpha}+$", "abcABC"),
        (r"^\p{White_Space}+$", " \t\n"),
        (r"^\p{Any}+$", "a1 \u{10ffff}"),
        (r"^\p{ASCII}+$", "abc\u{7f}"),
        (r"^\p{Assigned}+$", "abc"),
    ];
    for (pattern, input) in accepts {
        assert!(
            Regex::new(pattern, "u")
                .unwrap()
                .exec(input, 0)
                .unwrap()
                .is_some(),
            "{pattern} should match {input:?}"
        );
    }
    let rejects = [
        (r"^\P{L}+$", "Hello"),
        (r"^\p{Lu}+$", "hello"),
        (r"^\p{Script=Greek}+$", "abc"),
        (r"^\p{Assigned}+$", "\u{378}"),
        (r"^\p{ASCII}+$", "\u{80}"),
    ];
    for (pattern, input) in rejects {
        assert!(
            Regex::new(pattern, "u")
                .unwrap()
                .exec(input, 0)
                .unwrap()
                .is_none(),
            "{pattern} should not match {input:?}"
        );
    }
    for property in ["Alphabetic", "Script=Greek", "gc=Lu", "Letter"] {
        assert!(
            crate::unicode_props::supported(property),
            "{property} should be supported"
        );
    }
    assert!(!crate::unicode_props::supported("Not_A_Real_Property"));
}
#[test]
fn named_capture_groups_and_backreferences() {
    let regex = Regex::new(r"(?<year>\d{4})-(?<month>\d{2})-(?<day>\d{2})", "").unwrap();
    assert_eq!(
        regex.group_names(),
        &[
            ("year".into(), 1),
            ("month".into(), 2),
            ("day".into(), 3),
        ]
    );
    let m = regex.exec("2024-06-30", 0).unwrap().unwrap();
    assert_eq!(m.captures[1], Some(0..4));
    assert_eq!(m.captures[2], Some(5..7));
    assert_eq!(m.captures[3], Some(8..10));

    // `\k<name>` may reference a group declared later in the pattern.
    assert_eq!(
        captures(r"\k<a>(?<a>x)", "", "x"),
        vec![Some("x".into()), Some("x".into())]
    );
    assert!(Regex::new(r"(?<a>x)\k<a>", "")
        .unwrap()
        .exec("xx", 0)
        .unwrap()
        .is_some());
    assert!(Regex::new(r"(?<a>x)\k<a>", "")
        .unwrap()
        .exec("xy", 0)
        .unwrap()
        .is_none());

    // Names accept `$`, `_`, and Unicode ID_Start/ID_Continue (via `\u`
    // escapes too), but reject an undeclared backreference and duplicates.
    assert!(Regex::new(r"(?<$_é>a)\k<$_é>", "").is_ok());
    assert!(Regex::new(r"\k<undeclared>", "").is_err());
    assert!(Regex::new(r"(?<a>x)(?<a>y)", "").is_err());
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
