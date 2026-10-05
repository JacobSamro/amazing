use amazing_fetch::{
    CredentialsMode, EmbedderPolicy, EmbedderPolicyContainer, EmbedderPolicyOwner, FetchError,
    HeaderList, HttpUrl, Request, RequestMode, check_corp_with_credentials, psl::PublicSuffixList,
};
fn headers(value: &str) -> HeaderList {
    let mut headers = HeaderList::default();
    headers
        .append("cross-origin-embedder-policy", value)
        .unwrap();
    headers
}
fn policy(value: EmbedderPolicy) -> EmbedderPolicyContainer {
    EmbedderPolicyContainer {
        value,
        ..Default::default()
    }
}
#[test]
fn coep_actual_secure_response_single_item_and_complete_parameter_parsing() {
    for (header, value, endpoint) in [
        ("require-corp", EmbedderPolicy::RequireCorp, None),
        ("credentialless", EmbedderPolicy::Credentialless, None),
        (
            "credentialless;report-to=\"reports\"",
            EmbedderPolicy::Credentialless,
            Some("reports"),
        ),
        (
            "require-corp;report-to=\"a\";report-to=\"b\"",
            EmbedderPolicy::RequireCorp,
            Some("b"),
        ),
        (
            "credentialless;report-to=\"a\";report-to=?1",
            EmbedderPolicy::Credentialless,
            None,
        ),
        (
            "credentialless;ignored=:YWJj:;other=@12",
            EmbedderPolicy::Credentialless,
            None,
        ),
        (
            "unsafe-none;report-to=\"ignored\"",
            EmbedderPolicy::UnsafeNone,
            None,
        ),
        ("Credentialless", EmbedderPolicy::UnsafeNone, None),
        ("\"credentialless\"", EmbedderPolicy::UnsafeNone, None),
        (
            "credentialless, require-corp",
            EmbedderPolicy::UnsafeNone,
            None,
        ),
        (
            "credentialless;bad=(token)",
            EmbedderPolicy::UnsafeNone,
            None,
        ),
        ("credentialless trailing", EmbedderPolicy::UnsafeNone, None),
        ("", EmbedderPolicy::UnsafeNone, None),
    ] {
        let actual = EmbedderPolicyContainer::from_response(&headers(header), true);
        assert_eq!(actual.value, value, "{header}");
        assert_eq!(actual.reporting_endpoint.as_deref(), endpoint, "{header}");
        assert_eq!(
            EmbedderPolicyContainer::from_response(&headers(header), false),
            Default::default()
        );
    }
    let mut duplicate = headers("credentialless");
    duplicate
        .append("cross-origin-embedder-policy", "credentialless")
        .unwrap();
    assert_eq!(
        EmbedderPolicyContainer::from_response(&duplicate, true).value,
        EmbedderPolicy::UnsafeNone
    );
    let mut report_only = headers("require-corp;report-to=\"enforce\"");
    report_only
        .append(
            "cross-origin-embedder-policy-report-only",
            "credentialless;report-to=\"observe\"",
        )
        .unwrap();
    let actual = EmbedderPolicyContainer::from_response(&report_only, true);
    assert_eq!(actual.value, EmbedderPolicy::RequireCorp);
    assert_eq!(actual.report_only_value, EmbedderPolicy::Credentialless);
    assert_eq!(actual.reporting_endpoint.as_deref(), Some("enforce"));
    assert_eq!(
        actual.report_only_reporting_endpoint.as_deref(),
        Some("observe")
    );
}
#[test]
fn coep_nested_document_and_dedicated_worker_isolation_compatibility_matrix() {
    let values = [
        EmbedderPolicy::UnsafeNone,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ];
    for parent in values {
        for report_only in values {
            for child in values {
                let parent = EmbedderPolicyContainer {
                    report_only_value: report_only,
                    ..policy(parent)
                };
                for owner in [
                    EmbedderPolicyOwner::NestedNavigation,
                    EmbedderPolicyOwner::DedicatedWorker,
                    EmbedderPolicyOwner::TopLevelNavigation,
                    EmbedderPolicyOwner::SharedWorker,
                    EmbedderPolicyOwner::ServiceWorker,
                    EmbedderPolicyOwner::Worklet,
                ] {
                    let result = parent.check_inheritance(&policy(child), owner);
                    let incompatible =
                        owner.requires_owner_check() && child == EmbedderPolicy::UnsafeNone;
                    assert_eq!(
                        result.allowed,
                        !incompatible || parent.value == EmbedderPolicy::UnsafeNone
                    );
                    assert_eq!(
                        result.enforced_violation,
                        incompatible && parent.value != EmbedderPolicy::UnsafeNone
                    );
                    assert_eq!(
                        result.report_only_violation,
                        incompatible && report_only != EmbedderPolicy::UnsafeNone
                    );
                }
            }
        }
    }
}
#[test]
fn coep_effective_credentials_preserve_observable_mode_and_actual_redirect_taint() {
    let origin = HttpUrl::parse("https://origin.test/").unwrap();
    for value in [
        EmbedderPolicy::UnsafeNone,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ] {
        for mode in [
            RequestMode::NoCors,
            RequestMode::Cors,
            RequestMode::Navigate,
        ] {
            for credentials in [
                CredentialsMode::Omit,
                CredentialsMode::SameOrigin,
                CredentialsMode::Include,
            ] {
                for same_origin in [false, true] {
                    let target = HttpUrl::parse(if same_origin {
                        "https://origin.test/resource"
                    } else {
                        "https://other.test/resource"
                    })
                    .unwrap();
                    let mut request = Request::new(target, origin.origin().clone());
                    request.security.embedder_policy = value;
                    request.mode = mode;
                    request.credentials = credentials;
                    let allowed = request.prepare().unwrap().credentials;
                    let omitted_by_coep = value == EmbedderPolicy::Credentialless
                        && mode == RequestMode::NoCors
                        && !same_origin;
                    assert_eq!(
                        allowed,
                        !omitted_by_coep
                            && (credentials == CredentialsMode::Include
                                || (credentials == CredentialsMode::SameOrigin && same_origin))
                    );
                    assert_eq!(request.credentials, credentials);
                }
            }
        }
    }
}
#[test]
fn coep_corp_uses_response_credentials_and_nested_navigation_receipts() {
    let origin = HttpUrl::parse("https://origin.test/").unwrap();
    for value in [
        EmbedderPolicy::UnsafeNone,
        EmbedderPolicy::RequireCorp,
        EmbedderPolicy::Credentialless,
    ] {
        for credentials in [false, true] {
            for navigation in [false, true] {
                let mut request = Request::new(
                    HttpUrl::parse("https://other.test/image").unwrap(),
                    origin.origin().clone(),
                );
                request.mode = if navigation {
                    RequestMode::Navigate
                } else {
                    RequestMode::NoCors
                };
                request.security.embedder_policy = value;
                let result = check_corp_with_credentials(
                    &request,
                    &HeaderList::default(),
                    credentials,
                    navigation,
                    &PublicSuffixList,
                );
                let blocked = value == EmbedderPolicy::RequireCorp
                    || (value == EmbedderPolicy::Credentialless && (credentials || navigation));
                assert_eq!(
                    result,
                    if blocked {
                        Err(FetchError::CrossOriginResourcePolicy)
                    } else {
                        Ok(())
                    }
                );
                let mut corp = HeaderList::default();
                corp.append("cross-origin-resource-policy", "same-origin")
                    .unwrap();
                assert_eq!(
                    check_corp_with_credentials(
                        &request,
                        &corp,
                        credentials,
                        navigation,
                        &PublicSuffixList
                    ),
                    if navigation && value == EmbedderPolicy::UnsafeNone {
                        Ok(())
                    } else {
                        Err(FetchError::CrossOriginResourcePolicy)
                    }
                );
                corp.remove("cross-origin-resource-policy");
                corp.append("cross-origin-resource-policy", "cross-origin")
                    .unwrap();
                assert!(
                    check_corp_with_credentials(
                        &request,
                        &corp,
                        credentials,
                        navigation,
                        &PublicSuffixList
                    )
                    .is_ok()
                );
            }
        }
    }
}
