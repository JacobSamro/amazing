use amazing_fetch::*;
use amazing_net::ParsedUrl;
fn units(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}
fn context(source: &[u16]) -> CspInlineContext<'_> {
    CspInlineContext {
        kind: CspInlineKind::Script,
        source,
        nonce: None,
        nonce_attribute_present: false,
        attributes: &[],
        duplicate_attribute: false,
        parser_inserted: false,
        element: Some(17),
        document_url: "https://u:p@site.test/new-path#live",
        creation_url: "https://u:p@site.test/original?q=1#creation",
        referrer: "https://referrer.test/path#fragment",
        status_code: 200,
        source_file: "https://site.test/a.js#source",
        line_number: 4,
        column_number: 9,
    }
}
fn check(policy: &[u16]) -> CspInlineCheck {
    CspPolicyList::from_meta_utf16(policy)
        .unwrap()
        .check_inline(&context(&[65]))
}
#[test]
fn ascii_meta_acceptance_empty_duplicates_and_removed_directives() {
    let cases = [
        ("", false, 0),
        (" \t\r\n\u{c}", false, 0),
        ("script-src 'none'", true, 1),
        ("SCRIPT-SRC\u{c}'unsafe-inline';script-src 'none'", false, 1),
        ("script-src 'none';SCRIPT-SRC 'unsafe-inline'", true, 1),
        ("script-src\u{b}'none'", false, 0),
        ("script_src 'none'", false, 0),
        ("script-src 'unsafe-inline', script-src 'none'", true, 1),
        ("report-uri /r; frame-ancestors *; sandbox", false, 0),
        (
            "unknown value; default-src 'none'; report-to endpoint",
            true,
            3,
        ),
    ];
    for (serialized, blocked, directives) in cases {
        let raw = units(serialized);
        let policy = CspPolicyList::from_meta_utf16(&raw).unwrap();
        assert_eq!(policy.policies.len(), 1);
        assert_eq!(policy.policies[0].source, CspPolicySource::Meta);
        assert_eq!(policy.policies[0].disposition, CspDisposition::Enforce);
        assert_eq!(
            policy.policies[0].directives.len(),
            directives,
            "{serialized:?}"
        );
        assert_eq!(policy.policies[0].original_policy_units, raw);
        assert_eq!(
            policy.check_inline(&context(&[65])).blocked,
            blocked,
            "{serialized:?}"
        );
        assert_eq!(
            CspPolicyList::from_meta(serialized).unwrap().policies[0].directives,
            policy.policies[0].directives
        );
        assert!(!policy.contains_header_delivered_policy());
    }
}
#[test]
fn every_non_ascii_unit_skips_whole_directive_without_forging_empty_enforcement() {
    // Entire BMP nonASCII range, including every isolated surrogate: one whole
    // cluster control, no lossy conversion or newly invented empty source list.
    for unit in 0x80..=0xffff {
        let mut raw = units("script-src 'none' ");
        raw.push(unit);
        assert!(!check(&raw).blocked, "unit {unit:04x}");
        raw.extend(units("; script-src 'unsafe-inline'"));
        assert!(!check(&raw).blocked, "unit {unit:04x}");
        let mut raw = units("script-src 'unsafe-inline' ");
        raw.push(unit);
        raw.extend(units("; script-src 'none'"));
        let result = check(&raw);
        assert!(result.blocked, "unit {unit:04x}");
        assert_eq!(result.violations[0].original_policy_units, raw);
    }
}
#[test]
fn utf16_original_policy_custody_survives_clone_and_report_snapshot() {
    let mut raw = units(" \tdefault-src 'none' 'report-sample'; unknown ");
    raw.extend([0xd800, 0xdc00, 0xd800, 0x00e9, 0xdc00, 0x0000]);
    raw.extend(units("; default-src 'unsafe-inline' \r\n"));
    let policy = CspPolicyList::from_meta_utf16(&raw).unwrap();
    let copy = policy.clone();
    let source = [0x61, 0xd800, 0xd83d, 0xde00];
    let result = copy.check_inline(&context(&source));
    assert!(result.blocked);
    let mut violation = result.violations[0].clone();
    assert_eq!(violation.original_policy_units, raw);
    assert!(violation.original_policy.contains('\u{fffd}'));
    let snapshot = violation.report_snapshot();
    assert_eq!(snapshot.report_type, units("csp-violation"));
    assert_eq!(snapshot.visibility, CspReportVisibility::ObserverVisible);
    assert_eq!(snapshot.url, units("https://site.test/original?q=1"));
    let get = |key: &str| {
        &snapshot
            .body
            .iter()
            .find(|(name, _)| *name == units(key))
            .unwrap()
            .1
    };
    assert_eq!(
        get("documentURL"),
        &CspReportValue::String(units("https://site.test/new-path"))
    );
    assert_eq!(get("originalPolicy"), &CspReportValue::String(raw.clone()));
    assert_eq!(get("sample"), &CspReportValue::String(source.to_vec()));
    assert_eq!(
        get("sourceFile"),
        &CspReportValue::String(units("https://site.test/a.js"))
    );
    assert_eq!(get("lineNumber"), &CspReportValue::Number(4.));
    assert_eq!(get("columnNumber"), &CspReportValue::Number(9.));
    violation.original_policy_units.clear();
    violation.sample.clear();
    violation.document_uri = "https://unrelated.test/".into();
    assert_eq!(get("originalPolicy"), &CspReportValue::String(raw));
    assert_eq!(snapshot.url, units("https://site.test/original?q=1"));
}
#[test]
fn header_decoding_serialization_and_provenance_are_unchanged() {
    let mut headers = HeaderList::default();
    headers
        .append("Content-Security-Policy-Report-Only", " style-src 'none' ")
        .unwrap();
    headers
        .append(
            "Content-Security-Policy",
            "  default-src 'none' ; unknown é, script-src 'unsafe-inline' ",
        )
        .unwrap();
    let policy = CspPolicyList::from_headers(&headers).unwrap();
    assert_eq!(policy.policies.len(), 3);
    for policy in &policy.policies {
        assert_eq!(policy.source, CspPolicySource::Header);
        assert_eq!(policy.original_policy_units, units(&policy.original_policy));
    }
    assert_eq!(
        policy.policies[0].original_policy,
        "default-src 'none' ; unknown é"
    );
    assert_eq!(policy.policies[0].disposition, CspDisposition::Enforce);
    assert_eq!(policy.policies[2].disposition, CspDisposition::ReportOnly);
    assert!(policy.contains_header_delivered_policy());
    let result = policy.check_inline(&context(&[65]));
    assert!(result.blocked);
    assert_eq!(result.violations.len(), 1);
    assert_eq!(
        result.violations[0].original_policy_units,
        units("default-src 'none' ; unknown é")
    );
}
#[test]
fn utf16_nonce_case_and_duplicate_attributes_keep_native_rules() {
    let policy = CspPolicyList::from_meta_utf16(&units("script-src 'nonce-aB_+=='")).unwrap();
    let source = [65];
    let mut ctx = context(&source);
    let nonce = units("aB_+==");
    ctx.nonce = Some(&nonce);
    ctx.nonce_attribute_present = true;
    assert!(!policy.check_inline(&ctx).blocked);
    let wrong = units("ab_+==");
    ctx.nonce = Some(&wrong);
    assert!(policy.check_inline(&ctx).blocked);
    let malformed = [0x61, 0xd800];
    ctx.nonce = Some(&malformed);
    assert!(policy.check_inline(&ctx).blocked);
    ctx.nonce = Some(&nonce);
    ctx.nonce_attribute_present = false;
    assert!(policy.check_inline(&ctx).blocked);
    ctx.nonce_attribute_present = true;
    ctx.duplicate_attribute = true;
    assert!(policy.check_inline(&ctx).blocked);
    ctx.duplicate_attribute = false;
    let raw = units("script-src 'nonce-aB_+=='");
    let mut forged = raw.clone();
    forged.insert(0, 0xd800);
    assert!(
        !CspPolicyList::from_meta_utf16(&forged)
            .unwrap()
            .check_inline(&ctx)
            .blocked
    ); // invalid whole directive, no fabricated nonce restriction
}
#[test]
fn snapshot_nullable_fields_report_only_and_empty_sample_are_exact() {
    let mut headers = HeaderList::default();
    headers
        .append("Content-Security-Policy-Report-Only", "script-src 'none'")
        .unwrap();
    let policy = CspPolicyList::from_headers(&headers).unwrap();
    let source = [65];
    let mut ctx = context(&source);
    ctx.source_file = "";
    ctx.creation_url = "about:srcdoc";
    let result = policy.check_inline(&ctx);
    assert!(!result.blocked);
    let snapshot = result.violations[0].report_snapshot();
    assert_eq!(snapshot.url, units("about"));
    for key in ["sourceFile", "lineNumber", "columnNumber"] {
        assert_eq!(
            snapshot
                .body
                .iter()
                .find(|(name, _)| *name == units(key))
                .unwrap()
                .1,
            CspReportValue::Null
        );
    }
    assert_eq!(
        snapshot
            .body
            .iter()
            .find(|(name, _)| *name == units("sample"))
            .unwrap()
            .1,
        CspReportValue::String(vec![])
    );
    assert_eq!(
        snapshot
            .body
            .iter()
            .find(|(name, _)| *name == units("disposition"))
            .unwrap()
            .1,
        CspReportValue::String(units("report"))
    );
}
#[test]
fn bounds_are_checked_before_policy_allocation_and_keep_legacy_byte_limit() {
    assert_eq!(
        CspPolicyList::from_meta_utf16(&vec![0xd800; 65537]),
        Err(CspParseError::TooLong)
    );
    assert!(
        CspPolicyList::from_meta_utf16(&vec![0xd800; 65536])
            .unwrap()
            .policies[0]
            .directives
            .is_empty()
    );
    assert!(CspPolicyList::from_meta_utf16(&units(&"x;".repeat(255))).is_ok());
    assert_eq!(
        CspPolicyList::from_meta_utf16(&units(&"x;".repeat(256))),
        Err(CspParseError::TooManyDirectives)
    );
    assert_eq!(
        CspPolicyList::from_meta(&"é".repeat(32769)),
        Err(CspParseError::TooLong)
    );
    assert!(CspPolicyList::from_meta_utf16(&units(&"é".repeat(32769))).is_ok());
}
#[test]
fn native_receiving_client_factory_requires_actual_identity_and_keeps_inherited_origins() {
    #[derive(Clone, PartialEq, Eq, Debug)]
    struct NativeIdentity {
        document: u64,
        generation: u64,
        window: u64,
    }
    struct Factory {
        saved: ReceivingDocumentClient<NativeIdentity>,
    }
    impl ReceivingDocumentClientFactory<NativeIdentity> for Factory {
        type Error = &'static str;
        fn receiving_document_client(
            &self,
            id: &NativeIdentity,
        ) -> Result<ReceivingDocumentClient<NativeIdentity>, Self::Error> {
            if *id != self.saved.identity {
                return Err("stale native receiving document");
            }
            Ok(self.saved.clone())
        }
    }
    let identity = NativeIdentity {
        document: 17,
        generation: 4,
        window: 3,
    };
    let parent = HttpUrl::parse("https://creator.test/").unwrap();
    let opaque = Origin::opaque();
    let factory = Factory {
        saved: ReceivingDocumentClient {
            identity: identity.clone(),
            creation_url: ParsedUrl::parse("about:blank").unwrap(),
            document_url: ParsedUrl::parse("about:blank#document").unwrap(),
            base_url: ParsedUrl::parse("https://creator.test/base/").unwrap(),
            origin: opaque.clone(),
            csp_self_origin: parent.origin().clone(),
            context: ClientContext {
                cookie_site: amazing_fetch::cookies::CookieSite::CrossSite,
                now_unix: 1800000000,
                ..Default::default()
            },
            referrer_policy: ReferrerPolicy::NoReferrer,
        },
    };
    let client = factory.receiving_document_client(&identity).unwrap();
    assert_eq!(client.origin, opaque);
    assert_eq!(client.csp_self_origin, *parent.origin());
    assert_ne!(client.origin, client.csp_self_origin);
    assert_eq!(client.creation_url.serialize(), "about:blank");
    assert_eq!(client.base_url.serialize(), "https://creator.test/base/");
    assert_eq!(client.referrer_policy, ReferrerPolicy::NoReferrer);
    let request = client.policy_request(
        HttpUrl::parse("https://unrelated.test/target").unwrap(),
        CspDestination::ScriptElement,
    );
    assert_eq!(request.origin, opaque);
    assert_eq!(request.csp_self_origin, *parent.origin());
    assert_eq!(request.csp_destination, CspDestination::ScriptElement);
    assert_eq!(request.referrer_policy, ReferrerPolicy::NoReferrer);
    assert!(request.referrer.is_none());

    let stale = NativeIdentity {
        generation: 5,
        ..identity
    };
    assert!(factory.receiving_document_client(&stale).is_err());
}

#[test]
fn atomic_meta_admission_retains_header_and_inherited_policy_limits() {
    let mut headers = HeaderList::default();
    headers
        .append("Content-Security-Policy", "script-src 'unsafe-inline'")
        .unwrap();
    let inherited = CspPolicyList::from_headers(&headers).unwrap();
    let mut active = inherited.clone();
    active
        .append_meta_utf16(&units("script-src 'none'"))
        .unwrap();
    assert_eq!(active.policies[0], inherited.policies[0]);
    assert_eq!(active.policies[1].source, CspPolicySource::Meta);
    assert!(active.check_inline(&context(&[65])).blocked);
    assert!(!inherited.check_inline(&context(&[65])).blocked);
    let before = active.clone();
    assert_eq!(
        active.append_meta_utf16(&vec![0xd800; 65536]),
        Err(CspParseError::TooLong)
    );
    assert_eq!(active, before);
    let mut active = CspPolicyList::default();
    for _ in 0..64 {
        active.append_meta_utf16(&units("unknown")).unwrap();
    }
    let before = active.clone();
    assert_eq!(
        active.append_meta_utf16(&units("script-src 'none'")),
        Err(CspParseError::TooManyPolicies)
    );
    assert_eq!(active, before);
    let raw = (0..256)
        .map(|index| format!("unknown-{index}"))
        .collect::<Vec<_>>()
        .join(";");
    let mut active = CspPolicyList::from_meta_utf16(&units(&raw)).unwrap();
    let before = active.clone();
    assert_eq!(
        active.append_meta_utf16(&units("script-src 'none'")),
        Err(CspParseError::TooManyDirectives)
    );
    assert_eq!(active, before);
}
