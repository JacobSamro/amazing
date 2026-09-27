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
// JSD-3: the `ignoreCase` fold-partner check for `\p{...}`/`\P{...}` and for
// `v`-flag `Node::Set` character ranges used to scan the whole 1512-entry
// `unicode_casefold::PAIRS` table per character; replaced with one binary
// search (`unicode_casefold::partners`). Both paths only exist under
// unicode-mode (`u`/`v`) patterns, so this locks in the exact fold-invert
// semantics documented at the `SetExpr::contains` call site, not just speed.
#[test]
fn property_escape_and_v_flag_range_ignore_case_use_the_fast_partner_lookup() {
    // `\P{Lu}` must match "A" under ignoreCase: invert is folded in before
    // the fold-partner check, so "A"'s lowercase partner "a" (not in Lu)
    // decides it, even though "A" itself is directly in Lu.
    assert!(Regex::new(r"^\P{Lu}$", "iu").unwrap().exec("A", 0).unwrap().is_some());
    assert!(Regex::new(r"^\p{Lu}$", "iu").unwrap().exec("a", 0).unwrap().is_some());
    assert!(Regex::new(r"^\p{Lu}$", "u").unwrap().exec("a", 0).unwrap().is_none());
    // `v`-flag classes always compile through `Node::Set`/`SetExpr`'s
    // `Member::Range` (not `FoldedClass`), even without nesting.
    assert!(Regex::new(r"^[A-Z]$", "iv").unwrap().exec("k", 0).unwrap().is_some());
    assert!(Regex::new(r"^[A-Z]$", "v").unwrap().exec("k", 0).unwrap().is_none());
    // `unicode_casefold::partners` itself: bidirectional, can be more than
    // one value (both plain `K` and Kelvin-sign `K` fold to `k`), and empty
    // for code points with no case pairing.
    assert_eq!(crate::unicode_casefold::partners(0x41).collect::<Vec<_>>(), vec![0x61]);
    assert_eq!(crate::unicode_casefold::partners(0x61).collect::<Vec<_>>(), vec![0x41]);
    let mut k_partners: Vec<_> = crate::unicode_casefold::partners(0x6B).collect();
    k_partners.sort_unstable();
    assert_eq!(k_partners, vec![0x4B, 0x212A]);
    assert_eq!(crate::unicode_casefold::partners('0' as u32).collect::<Vec<_>>(), Vec::<u32>::new());
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
    // Annex B: `\k<name>` is only forced into GroupName/backreference syntax
    // when the pattern has a named group SOMEWHERE (or is in unicode mode).
    // With no named groups anywhere, it's a plain identity escape for `k`.
    assert!(Regex::new(r"\k<undeclared>", "").is_ok());
    assert!(Regex::new(r"\k<undeclared>", "u").is_err());
    assert!(Regex::new(r"(?<b>x)\k<undeclared>", "").is_err());
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

#[test]
fn lookbehind_assertions() {
    assert_eq!(captures(r"(?<=\$)\d+", "", "$100")[0], Some("100".into()));
    assert!(Regex::new(r"(?<=\$)\d+", "").unwrap().exec("100", 0).unwrap().is_none());
    assert_eq!(captures(r"(?<!\$)\b\d+", "", "$100 200")[0], Some("200".into()));
    // The digits right after `$` fail the negative lookbehind, but the
    // engine keeps trying later start positions, so "00" (starting past
    // the first digit) still matches.
    assert_eq!(captures(r"(?<!\$)\d+", "", "$100")[0], Some("00".into()));
    // Variable-length lookbehind content.
    assert_eq!(captures(r"(?<=a+)b", "", "aaab")[0], Some("b".into()));
    assert!(Regex::new(r"(?<=a+)b", "").unwrap().exec("b", 0).unwrap().is_none());
    // Capturing groups inside a lookbehind populate the outer match.
    assert_eq!(
        captures(r"(?<=(\d+)-)\w+", "", "42-foo"),
        vec![Some("foo".into()), Some("42".into())]
    );
    // Quantifying a lookbehind is a syntax error, same as lookahead.
    assert!(matches!(Regex::new(r"(?<=a)+", ""), Err(Error::Syntax(_))));
}

#[test]
fn inline_modifier_groups() {
    // `i` scoped on: matches case-insensitively only inside the group.
    assert!(Regex::new(r"(?i:a)b", "").unwrap().exec("Ab", 0).unwrap().is_some());
    assert!(Regex::new(r"(?i:a)b", "").unwrap().exec("AB", 0).unwrap().is_none());
    // `i` scoped off inside an outer case-insensitive match.
    assert!(Regex::new(r"(?-i:a)b", "i").unwrap().exec("Ab", 0).unwrap().is_none());
    assert!(Regex::new(r"(?-i:a)b", "i").unwrap().exec("aB", 0).unwrap().is_some());
    // `s` (dotAll) scoped on: only inside the group does `.` match a newline.
    assert!(Regex::new(r"(?s:a.)b", "").unwrap().exec("a\nb", 0).unwrap().is_some());
    assert!(Regex::new(r"a.b", "").unwrap().exec("a\nb", 0).unwrap().is_none());
    // `m` scoped on: `^`/`$` become line-relative only inside the group.
    assert!(Regex::new(r"(?m:^b)", "").unwrap().exec("a\nb", 0).unwrap().is_some());
    assert!(Regex::new(r"^b", "").unwrap().exec("a\nb", 0).unwrap().is_none());
    // Combining add and remove, and nesting.
    assert!(Regex::new(r"(?i-s:a.)b", "s").unwrap().exec("A\nb", 0).unwrap().is_none());
    assert!(Regex::new(r"(?i-s:a.)b", "s").unwrap().exec("AXb", 0).unwrap().is_some());
    assert!(Regex::new(r"(?i:a(?-i:b))", "").unwrap().exec("Ab", 0).unwrap().is_some());
    assert!(Regex::new(r"(?i:a(?-i:b))", "").unwrap().exec("AB", 0).unwrap().is_none());
    // A quantified modifier group behaves like a quantified non-capturing group.
    assert!(Regex::new(r"(?i:a)+", "").unwrap().exec("aAa", 0).unwrap().is_some());
    // Syntax errors: unknown modifier letters, duplicates, empty modifier list.
    assert!(matches!(Regex::new(r"(?x:a)", ""), Err(Error::Syntax(_))));
    assert!(matches!(Regex::new(r"(?ii:a)", ""), Err(Error::Syntax(_))));
    assert!(matches!(Regex::new(r"(?i-i:a)", ""), Err(Error::Syntax(_))));
    assert!(matches!(Regex::new(r"(?-:a)", ""), Err(Error::Syntax(_))));
}

#[test]
fn has_indices_flag() {
    assert!(Flags::parse("d").unwrap().has_indices);
    assert_eq!(Flags::parse("d").unwrap().as_string(), "d");
    assert_eq!(Flags::parse("gid").unwrap().as_string(), "dgi");
    assert!(matches!(Flags::parse("dd"), Err(Error::Syntax(_))));
}

#[test]
fn many_ignorecase_classes_stay_well_under_the_compile_budget() {
    // Modelled on test262's `testIntl.js` BCP-47 identifier regex: many
    // distinct `[a-z]`/`[0-9]`/`[a-z0-9]` character classes combined into
    // one big alternation under the `i` flag. The old cost model charged a
    // flat, case-fold-table-sized budget per class regardless of content,
    // which alone exhausted `MAX_CODE` long before this pattern's actual
    // (now cheap) work would; this must compile and match cleanly.
    let group = "(?:[a-z]|[0-9]|[a-z0-9])";
    let pattern = format!("^{}$", group.repeat(60));
    let regex = Regex::new(&pattern, "i").expect("many-class ignoreCase pattern should compile");
    assert!(regex.exec(&"a1B".repeat(20), 0).unwrap().is_some());
}

#[test]
fn annex_b_class_escape_extends_control_letter_to_digits_and_underscore() {
    // `\cX` outside a class only accepts a real ControlLetter; `\c0`
    // doesn't form a control escape there, so it falls back to literal
    // "c0" (matching neither of the non-'c'/'0' bytes below).
    assert!(Regex::new(r"\c0", "").unwrap().exec("\x0f\x10\x11", 0).unwrap().is_none());
    // Inside a class, Annex B's `ClassControlLetter` also accepts a decimal
    // digit or `_`, yielding `chr(code % 32)`.
    assert_eq!(captures(r"[\c0]", "", "\x0f\x10\x11")[0], Some("\x10".into()));
    assert_eq!(captures(r"[\c9]", "", "\x18\x19\x1a")[0], Some("\x19".into()));
    assert_eq!(captures(r"[\c_]", "", "\x1e\x1f\x20")[0], Some("\x1f".into()));
    // In unicode mode neither extension applies: `\c` not followed by a
    // real ControlLetter is always a Syntax Error.
    assert!(matches!(Regex::new(r"[\c0]", "u"), Err(Error::Syntax(_))));
    assert!(matches!(Regex::new(r"\c0", "u"), Err(Error::Syntax(_))));
}

#[test]
fn annex_b_identity_escape_covers_p_and_out_of_range_digit_escapes() {
    // `\P`/`\p` aren't recognized outside unicode mode, so they fall back
    // to a literal identity escape instead of erroring.
    assert_eq!(captures(r"O\PQ", "", "MNOPQRS")[0], Some("OPQ".into()));
    // A `\N` with no matching capture group falls back through legacy
    // octal (8/9 aren't octal digits) to a plain literal digit, rather than
    // a Syntax Error.
    assert_eq!(captures(r"\8", "", "789")[0], Some("8".into()));
    assert_eq!(captures(r"7\89", "", "67890")[0], Some("789".into()));
    // DecimalEscape (backreference) still takes precedence once enough
    // capturing groups exist.
    assert_eq!(
        captures(r"(.)(.)(.)(.)(.)(.)(.)(.)\8\8", "", "0123456777")[0],
        Some("0123456777".into())
    );
    // In unicode mode there's no Annex B fallback: an out-of-range `\N` is
    // always a Syntax Error.
    assert!(matches!(Regex::new(r"\8", "u"), Err(Error::Syntax(_))));
}

#[test]
fn annex_b_legacy_octal_escape_sequence() {
    for (pattern, byte) in [
        (r"\1", 0x01u32),
        (r"\7", 0x07),
        (r"\00", 0x00),
        (r"\07", 0x07),
        (r"\40", 0x20),
        (r"\77", 0x3f),
        (r"\000", 0x00),
        (r"\007", 0x07),
        (r"\300", 0xc0),
        (r"\377", 0xff),
    ] {
        let ch = char::from_u32(byte).unwrap();
        assert_eq!(
            captures(pattern, "", &ch.to_string())[0],
            Some(ch.to_string()),
            "{pattern}"
        );
    }
    // Bounded to 3 digits including leading zeros: the trailing digit is a
    // separate literal-digit atom.
    assert_eq!(captures(r"\0111", "", "\x091")[0], Some("\x091".into()));
    // Sequence is bounded according to the resulting String Value, not the
    // number of digits written: `\400` is `\40` (0x20) followed by a
    // literal `0`.
    assert_eq!(captures(r"\400", "", "\x200")[0], Some("\x200".into()));
    // DecimalEscape still takes precedence when the referenced group exists.
    assert_eq!(captures(r"(.)\1", "", "a\x01 aa")[0], Some("aa".into()));
}

#[test]
fn annex_b_non_empty_class_ranges_fall_back_to_union() {
    // Neither side of a `-` is a single character (`\d` is a character
    // *class*), so Annex B's CharacterRangeOrUnion makes `-` a literal
    // member instead of a range operator.
    assert_eq!(captures(r"[--\d]+", "", ".-0123456789-.")[0], Some("-0123456789-".into()));
    assert_eq!(captures(r"[\d-a]+", "", ":a0123456789-:")[0], Some("a0123456789-".into()));
    assert_eq!(captures(r"[\s-\d]+", "", "& \t0123456789-&")[0], Some(" \t0123456789-".into()));
    // Both sides single characters, in order: an ordinary range, unaffected.
    assert_eq!(captures(r"[a-z]+", "", "!abc!")[0], Some("abc".into()));
    // Both sides single characters, reversed: always a Syntax Error, in
    // every mode -- this early error isn't one of the Annex B fallbacks.
    assert!(matches!(Regex::new(r"[z-a]", ""), Err(Error::Syntax(_))));
    assert!(matches!(Regex::new(r"[z-a]", "u"), Err(Error::Syntax(_))));
    // Not both single characters, but in unicode mode: still a Syntax Error.
    assert!(matches!(Regex::new(r"[--\d]", "u"), Err(Error::Syntax(_))));
}

#[test]
fn annex_b_quantifiable_lookahead_assertion() {
    // `(?=...)`/`(?!...)` may be quantified in non-unicode mode only; the
    // VM's existing zero-width no-progress guard makes each repetition
    // after the first (which makes no position progress) fail cleanly.
    assert_eq!(
        captures(r".(?=Z)*", "", "a bZ cZZ dZZZ eZZZZ")[0],
        Some("a".into())
    );
    assert_eq!(
        captures(r".(?=Z)+", "", "a bZ cZZ dZZZ eZZZZ")[0],
        Some("b".into())
    );
    assert_eq!(
        captures(r".(?=Z){2,3}", "", "a bZ cZZ dZZZ eZZZZ")[0],
        Some("b".into())
    );
    assert_eq!(
        captures(r"[a-e](?!Z)+", "", "aZZZZ bZZZ cZZ dZ e")[0],
        Some("e".into())
    );
    // Lookbehind is never quantifiable, in either mode (unchanged).
    assert!(matches!(Regex::new(r"(?<=a)+", ""), Err(Error::Syntax(_))));
    // In unicode mode, quantifying a lookahead is still a Syntax Error.
    assert!(matches!(Regex::new(r"(?=a)+", "u"), Err(Error::Syntax(_))));
}
