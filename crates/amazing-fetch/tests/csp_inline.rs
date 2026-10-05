use amazing_fetch::*;
fn policies(enforce: &str, report: &str) -> CspPolicyList {
    let mut headers = HeaderList::default();
    if !enforce.is_empty() {
        headers.append("content-security-policy", enforce).unwrap();
    }
    if !report.is_empty() {
        headers
            .append("content-security-policy-report-only", report)
            .unwrap();
    }
    CspPolicyList::from_headers(&headers).unwrap()
}
fn context(source: &[u16], kind: CspInlineKind) -> CspInlineContext<'_> {
    CspInlineContext {
        kind,
        source,
        nonce: None,
        nonce_attribute_present: false,
        attributes: &[],
        duplicate_attribute: false,
        parser_inserted: true,
        element: Some(17),
        document_url: "https://u:p@site.test/page?q=1#secret",
        referrer: "https://referrer.test/path#fragment",
        status_code: 200,
        source_file: "https://site.test/script.js#location",
        line_number: 7,
        column_number: 4,
    }
}
#[test]
fn fallback_and_independent_policies_and_report_data() {
    let source = "0123456789012345678901234567890123456789EXTRA"
        .encode_utf16()
        .collect::<Vec<_>>();
    let check = policies(
        "default-src 'none' 'report-sample', script-src-elem 'none'",
        "script-src 'none' 'report-sample'; report-to endpoint; report-uri /report",
    )
    .check_inline(&context(&source, CspInlineKind::Script));
    assert!(check.blocked);
    assert_eq!(check.violations.len(), 3);
    let v = &check.violations[0];
    assert_eq!(v.effective_directive, "script-src-elem");
    assert_eq!(v.violated_directive, "script-src-elem");
    assert_eq!(v.source_directive, "default-src");
    assert_eq!(v.original_policy, "default-src 'none' 'report-sample'");
    assert_eq!(v.sample, source[..40]);
    assert_eq!(v.element, Some(17));
    assert_eq!(v.blocked_uri, "inline");
    assert_eq!(v.document_uri, "https://site.test/page?q=1");
    assert_eq!(v.referrer, "https://referrer.test/path");
    assert_eq!(v.source_file, "https://site.test/script.js");
    assert_eq!((v.status_code, v.line_number, v.column_number), (200, 7, 4));
    assert!(check.violations[1].sample.is_empty());
    assert_eq!(check.violations[2].disposition, CspDisposition::ReportOnly);
    assert_eq!(check.violations[2].report_to.as_deref(), Some("endpoint"));
    assert_eq!(check.violations[2].report_uris, ["/report"]);
    for (directive, value) in [
        ("report-to", ""),
        ("report-to endpoint other", "endpoint other"),
    ] {
        let p = policies(
            &format!("script-src 'none'; {directive}; report-uri /legacy"),
            "",
        );
        let v = p
            .check_inline(&context(&source, CspInlineKind::Script))
            .violations
            .remove(0);
        assert_eq!(v.report_to.as_deref(), Some(value));
    }
    // Navigation's reporting integration (CSP3 4.2.4), unlike element behavior,
    // reports at the global and never attaches an element or report-sample.
    let v = policies("script-src 'none' 'report-sample'", "")
        .check_inline(&context(&source, CspInlineKind::Navigation))
        .violations
        .remove(0);
    assert_eq!(v.element, None);
    assert!(v.sample.is_empty());
    assert!(
        !policies("", "script-src 'none'")
            .check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
    assert_eq!(
        csp_strip_report_url("data:text/plain,private#secret"),
        "data"
    );
    assert_eq!(csp_strip_report_url("file:///private/path"), "file");
}
#[test]
fn all_inline_kinds_follow_only_their_own_fallback_chain() {
    let source = [65];
    let kinds = [
        (CspInlineKind::Script, "script-src-elem"),
        (CspInlineKind::ScriptAttribute, "script-src-attr"),
        (CspInlineKind::Navigation, "script-src-elem"),
        (CspInlineKind::Style, "style-src-elem"),
        (CspInlineKind::StyleAttribute, "style-src-attr"),
    ];
    for (kind, name) in kinds {
        let ctx = context(&source, kind);
        assert!(!policies("img-src 'none'", "").check_inline(&ctx).blocked);
        assert!(
            policies("default-src 'none'", "")
                .check_inline(&ctx)
                .blocked
        );
        assert!(
            !policies(&format!("default-src 'none'; {name} 'unsafe-inline'"), "")
                .check_inline(&ctx)
                .blocked
        );
        assert!(
            policies(&format!("default-src 'unsafe-inline'; {name}"), "")
                .check_inline(&ctx)
                .blocked
        );
    }
    let ctx = context(&source, CspInlineKind::ScriptAttribute);
    assert!(
        policies("script-src 'none'; script-src-elem 'unsafe-inline'", "")
            .check_inline(&ctx)
            .blocked
    );
}
#[test]
fn unsafe_inline_suppression_uses_valid_grammar_and_exact_nonce_text() {
    let source = [65];
    let nonce = "aB_+==".encode_utf16().collect::<Vec<_>>();
    for suppress in [
        "'nonce-abc'",
        "'sha256-abc'",
        "'SHA384-abc='",
        "'strict-dynamic'",
    ] {
        assert!(
            policies(&format!("script-src 'unsafe-inline' {suppress}"), "")
                .check_inline(&context(&source, CspInlineKind::Script))
                .blocked
        );
    }
    for invalid in [
        "'nonce-'",
        "'nonce-abc==='",
        "'sha256-!bad'",
        "'sha999-abc'",
        "'nonce-a=b'",
    ] {
        assert!(
            !policies(&format!("script-src 'unsafe-inline' {invalid}"), "")
                .check_inline(&context(&source, CspInlineKind::Script))
                .blocked
        );
    }
    let mut ctx = context(&source, CspInlineKind::Script);
    ctx.nonce = Some(&nonce);
    ctx.nonce_attribute_present = true;
    assert!(
        !policies("script-src 'NONCE-aB_+=='", "")
            .check_inline(&ctx)
            .blocked
    );
    assert!(
        policies("script-src 'nonce-ab_+=='", "")
            .check_inline(&ctx)
            .blocked
    );
    ctx.nonce_attribute_present = false;
    assert!(
        policies("script-src 'nonce-aB_+=='", "")
            .check_inline(&ctx)
            .blocked
    );
    ctx.nonce_attribute_present = true;
    for kind in [
        CspInlineKind::ScriptAttribute,
        CspInlineKind::Navigation,
        CspInlineKind::StyleAttribute,
    ] {
        ctx.kind = kind;
        assert!(
            policies("default-src 'nonce-aB_+=='", "")
                .check_inline(&ctx)
                .blocked
        );
    }
}
#[test]
fn nonceability_checks_actual_attributes_and_duplicate_flag() {
    let source = [65];
    let nonce = [97, 98, 99];
    let safe = [120];
    for needle in ["<ScRiPt", "<STYLE", "<LiNk"] {
        let bad = needle.encode_utf16().collect::<Vec<_>>();
        for attr in [
            CspInlineAttribute {
                name: &safe,
                value: &bad,
            },
            CspInlineAttribute {
                name: &bad,
                value: &safe,
            },
        ] {
            let attributes = [attr];
            let mut ctx = context(&source, CspInlineKind::Script);
            ctx.nonce = Some(&nonce);
            ctx.nonce_attribute_present = true;
            ctx.attributes = &attributes;
            assert!(!csp_inline_nonceable(&ctx));
            assert!(
                policies("script-src 'nonce-abc'", "")
                    .check_inline(&ctx)
                    .blocked
            );
            ctx.kind = CspInlineKind::Style;
            assert!(
                !policies("style-src 'nonce-abc'", "")
                    .check_inline(&ctx)
                    .blocked
            );
        }
    }
    let mut ctx = context(&source, CspInlineKind::Script);
    ctx.nonce = Some(&nonce);
    ctx.nonce_attribute_present = true;
    ctx.duplicate_attribute = true;
    assert!(
        policies("script-src 'nonce-abc'", "")
            .check_inline(&ctx)
            .blocked
    );
    ctx.duplicate_attribute = false;
    assert!(
        !policies("script-src 'nonce-abc'", "")
            .check_inline(&ctx)
            .blocked
    );
}
#[test]
fn cryptographic_hashes_all_algorithms_and_exact_utf16_source_conversion() {
    let hashes=["'sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='",
        "'sha384-ywB1P0WjXou1oD1pmsZQBycsMqsO3tFjGotgWkP/W+2AhgcroefMI1i67KE0yCWn'",
        "'sha512-3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw=='"];
    let source = "abc".encode_utf16().collect::<Vec<_>>();
    for hash in hashes {
        let policy = policies(&format!("script-src {hash}"), "");
        assert!(
            !policy
                .check_inline(&context(&source, CspInlineKind::Script))
                .blocked,
            "{hash}"
        );
        for changed in [" abc", "abc\n", "ABC"] {
            assert!(
                policy
                    .check_inline(&context(
                        &changed.encode_utf16().collect::<Vec<_>>(),
                        CspInlineKind::Script
                    ))
                    .blocked
            );
        }
        assert!(
            !policies(
                &format!("script-src {}", hash.replace('+', "-").replace('/', "_")),
                ""
            )
            .check_inline(&context(&source, CspInlineKind::Script))
            .blocked
        );
    }
    // Independent Python hashlib SHA256 of UTF8 U+FFFD, validating lone-surrogate handling.
    assert!(
        !policies(
            "script-src 'sha256-g9VEzMIjwFfSv4DT8qMpgsMsPA244mdIINpQZHg/sJc='",
            ""
        )
        .check_inline(&context(&[0xd800], CspInlineKind::Script))
        .blocked
    );
}
#[test]
fn unsafe_hashes_only_enables_attribute_and_navigation_hashes() {
    let source = "abc".encode_utf16().collect::<Vec<_>>();
    let hash = "'sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='";
    for kind in [
        CspInlineKind::ScriptAttribute,
        CspInlineKind::Navigation,
        CspInlineKind::StyleAttribute,
    ] {
        assert!(
            policies(&format!("default-src {hash}"), "")
                .check_inline(&context(&source, kind))
                .blocked
        );
        assert!(
            !policies(&format!("default-src {hash} 'UNSAFE-HASHES'"), "")
                .check_inline(&context(&source, kind))
                .blocked
        );
    }
}
#[test]
fn current_primary_strict_dynamic_uses_actual_parser_metadata() {
    let source = [65];
    let p = policies("default-src 'unsafe-inline' 'strict-dynamic'", "");
    let mut ctx = context(&source, CspInlineKind::Script);
    assert!(p.check_inline(&ctx).blocked);
    ctx.parser_inserted = false;
    assert!(!p.check_inline(&ctx).blocked);
    for kind in [CspInlineKind::ScriptAttribute, CspInlineKind::Navigation] {
        ctx.kind = kind;
        assert!(p.check_inline(&ctx).blocked);
    }
    ctx.kind = CspInlineKind::Style;
    assert!(!p.check_inline(&ctx).blocked);
}
#[test]
fn meta_delivery_preserves_policy_identity_and_removes_disallowed_directives() {
    let p = CspPolicyList::from_meta(
        "script-src 'none'; report-uri /r; frame-ancestors *; sandbox; report-to endpoint",
    )
    .unwrap();
    assert_eq!(p.policies.len(), 1);
    assert_eq!(p.policies[0].source, CspPolicySource::Meta);
    assert_eq!(p.policies[0].directives.len(), 2);
    let check = p.check_inline(&context(&[65], CspInlineKind::Script));
    assert!(check.blocked);
    assert!(check.violations[0].report_uris.is_empty());
    assert_eq!(check.violations[0].report_to.as_deref(), Some("endpoint"));
    let p = CspPolicyList::from_meta("script-src 'unsafe-inline', script-src 'none'").unwrap();
    assert_eq!(p.policies.len(), 1);
    assert_eq!(
        CspPolicyList::from_meta(&"x".repeat(65537)),
        Err(CspParseError::TooLong)
    );
}
#[test]
fn source_hashes_preserve_newlines_utf16_pairs_and_padding() {
    // Independent hashlib values, not expectations computed with engine hashing.
    let pairs = [
        (
            vec![0xd83d, 0xde00],
            "8EQ6NCxe9UeDoRG1G6Vsk45HTDIyTZDDpgycjjo34tk=",
        ),
        (
            vec![0xd800, 0x61, 0xdc00],
            "HzJmEeV50kqJFQKkTAmvn13N82VeNXw8/b4Vi0SW5hc=",
        ),
        (
            vec![0x61, 13, 10, 0x62],
            "GHRfNqBeKQcnCQQtYGLOVPGwj/NsJ7qAw5+B+wEMjOI=",
        ),
        (Vec::new(), "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="),
    ];
    for (source, hash) in pairs {
        let policy = format!("script-src 'sha256-{hash}'");
        assert!(
            !policies(&policy, "")
                .check_inline(&context(&source, CspInlineKind::Script))
                .blocked
        );
        assert!(
            policies(&policy.replace('=', ""), "")
                .check_inline(&context(&source, CspInlineKind::Script))
                .blocked
        );
    }
}
#[test]
fn report_sample_retains_exact_utf16_and_does_not_trim_source() {
    let mut source = vec![32; 39];
    source.extend([0xd83d, 0xde00, 0xd800]);
    let v = policies("default-src 'none' 'report-sample'", "")
        .check_inline(&context(&source, CspInlineKind::Script))
        .violations
        .remove(0);
    assert_eq!(v.sample.len(), 40);
    assert_eq!(v.sample[0], 32);
    assert_eq!(v.sample[39], 0xd83d);
}
#[test]
fn delivery_provenance_survives_inheritance_and_empty_header_policy_is_discarded() {
    let mut headers = HeaderList::default();
    headers.append("Content-Security-Policy", "").unwrap();
    let inherited = CspPolicyList::from_headers(&headers).unwrap().clone();
    assert!(!inherited.contains_header_delivered_policy());
    assert!(inherited.policies.is_empty());
    assert!(policies("script-src", "")
        .clone()
        .contains_header_delivered_policy());
    assert!(!CspPolicyList::from_meta("script-src 'none'")
        .unwrap()
        .contains_header_delivered_policy());
    assert!(!CspPolicyList::default().contains_header_delivered_policy());
    assert!(policies("", "script-src 'none'").contains_header_delivered_policy());
    let mut headers = HeaderList::default();
    headers
        .append("content-security-policy-report-only", "script-src 'none'")
        .unwrap();
    headers
        .append("content-security-policy", "script-src 'unsafe-inline'")
        .unwrap();
    let list = CspPolicyList::from_headers(&headers).unwrap();
    assert_eq!(list.policies[0].disposition, CspDisposition::Enforce);
    assert_eq!(list.policies[1].disposition, CspDisposition::ReportOnly);
}
#[test]
fn csp_hash_matching_is_any_algorithm_not_sri_strongest_and_is_case_sensitive() {
    let source = [97, 98, 99];
    let p = policies(
        "script-src 'sha512-AAAA' 'sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='",
        "",
    );
    assert!(
        !p.check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
    let p = policies(
        "style-src 'SHA256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='",
        "",
    );
    assert!(
        !p.check_inline(&context(&source, CspInlineKind::Style))
            .blocked
    );
    let p = policies(
        "style-src 'sha256-UNGWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='",
        "",
    );
    assert!(
        p.check_inline(&context(&source, CspInlineKind::Style))
            .blocked
    );
}
#[test]
fn policy_parsing_preserves_first_directive_and_uses_infra_ascii_whitespace() {
    let source = [65];
    let p = CspPolicyList::from_meta("SCRIPT-SRC\u{c}'unsafe-inline'; script-src 'none'").unwrap();
    assert!(
        !p.check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
    let p = CspPolicyList::from_meta("script-src\u{b}'none'").unwrap();
    assert!(
        !p.check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
    // A nonASCII serialized directive is skipped in its entirety, rather than
    // discarding its nonASCII value and enforcing a newly invented empty list.
    let p = CspPolicyList::from_meta("script-src 'unsafe-inline' é; script-src 'none'").unwrap();
    assert!(
        p.check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
    let p = CspPolicyList::from_meta("script-src 'none' é").unwrap();
    assert!(
        !p.check_inline(&context(&source, CspInlineKind::Script))
            .blocked
    );
}
#[test]
fn independent_nonce_and_hash_policies_require_each_enforcing_policy_to_match() {
    let source = [97, 98, 99];
    let nonce = [97, 98, 99];
    let mut ctx = context(&source, CspInlineKind::Script);
    ctx.nonce = Some(&nonce);
    ctx.nonce_attribute_present = true;
    let p = policies(
        "script-src 'nonce-abc', script-src 'sha256-ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0='",
        "script-src 'none'",
    );
    let check = p.check_inline(&ctx);
    assert!(!check.blocked);
    assert_eq!(check.violations.len(), 1);
    assert_eq!(check.violations[0].policy_index, 2);
    ctx.nonce = None;
    let check = p.check_inline(&ctx);
    assert!(check.blocked);
    assert_eq!(check.violations.len(), 2);
    assert_eq!(check.violations[0].policy_index, 0);
    assert_eq!(check.violations[1].policy_index, 2);
}
