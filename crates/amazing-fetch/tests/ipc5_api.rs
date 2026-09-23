use amazing_fetch::{
    FetchError, HttpUrl, Origin, Site,
    api::{ApiRequest, ApiRequestInit, ApiRequestInput},
};

#[test]
fn opaque_origin_identity_survives_reconstruction() {
    let id = (1_u128 << 96) + 17;
    let first = Origin::opaque_with_id(id);
    let restored = Origin::opaque_with_id(id);
    assert_eq!(first, restored);
    assert_eq!(first.opaque_id(), Some(id));
    assert_eq!(first.serialize(), "null");
    assert_eq!(first.site(&NoDomains), Site::Opaque(id));
    assert_ne!(first, Origin::opaque_with_id(id + 1));
    assert_ne!(Origin::opaque(), Origin::opaque());
    assert_eq!(
        HttpUrl::parse("https://example.test/")
            .unwrap()
            .origin()
            .opaque_id(),
        None
    );
}

struct NoDomains;
impl amazing_fetch::RegistrableDomain for NoDomains {
    fn registrable_domain(&self, _: &str) -> Option<String> {
        None
    }
}

#[test]
fn response_limit_accepts_bounded_sizes_and_rejects_excess() {
    let base = HttpUrl::parse("https://example.test/").unwrap();
    let mut request = ApiRequest::new(
        ApiRequestInput::Url(&"/data".encode_utf16().collect::<Vec<_>>()),
        ApiRequestInit::default(),
        &base,
        base.origin().clone(),
    )
    .unwrap();
    request.set_response_body_limit(0).unwrap();
    request.set_response_body_limit(1024).unwrap();
    assert_eq!(
        request.set_response_body_limit(u64::MAX),
        Err(FetchError::ResponseTooLarge)
    );
    assert_eq!(
        request.set_response_body_limit(32 * 1024 * 1024 + 1),
        Err(FetchError::ResponseTooLarge)
    );
    request.set_response_body_limit(32 * 1024 * 1024).unwrap();
}
