//! Native ports of WPT subresource-integrity/subresource-integrity.html script cases.
//! WPT revision c48d58747e1f211527fb695fd60548a997fae617.
//! Source SHA-256: 49afc676663f757ddbbc26c8c6c8ae29efab1650d9b33460d4e28e96666d9030.
//! script.js SHA-256: 06eebcd4a327435e5160716fb18756ba6c1e785030d212434c5b7db1e12b8210.
use amazing_fetch::{
    api::{fetch_api, ApiError, ApiRequest, ApiRequestInit, ApiRequestInput},
    FetchError, FetchPlan, HeaderList, HttpUrl, InternalResponse, NetworkOrCache, Request, Site,
};

const SCRIPT: &[u8] = b"// nothing important.\n";
const SHA256: &str = "sha256-Bu681KMnQ15RYHFvsYdWumweeFAw0hJDTFt9seErghA=";
const SHA384: &str = "sha384-cINXh+nCzEHPWzXS7eoT+vYMBpyqczOybRLNU3XAButFWCRhHT5hLByIbPRqIm2f";
const SHA512: &str = "sha512-KZdenhzBd7X7Q/vmaOSyvFz1CGdoVt26xzCZjlkU9lfBEK+V/ougGys7iYDi0+tOHIQSQa87bIqx95R7GU7I9Q==";

struct Stub {
    cors: bool,
}
impl NetworkOrCache for Stub {
    fn exchange(
        &mut self,
        _request: &Request,
        _plan: &FetchPlan,
        _site: &Site,
    ) -> Result<InternalResponse, FetchError> {
        let mut headers = HeaderList::default();
        if self.cors {
            headers.append("access-control-allow-origin", "*").unwrap();
        }
        Ok(InternalResponse {
            status: 200,
            status_text: "OK".into(),
            headers,
            body: Some(SCRIPT.to_vec()),
        })
    }
}

#[test]
fn pinned_wpt_script_integrity_matrix() {
    let cases: &[(&str, &str, bool, bool, bool)] = &[
        ("correct sha256", SHA256, false, false, true),
        ("correct sha384", SHA384, false, false, true),
        ("correct sha512", SHA512, false, false, true),
        ("empty", "", false, false, true),
        ("non-Base64", "sha256-...", false, false, true),
        ("malformed question before dash", "?-=" , false, false, true),
        ("malformed no hash", "sha256-", false, false, true),
        ("malformed no dash", "sha256", false, false, true),
        ("incorrect", "sha256-deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead", false, false, false),
        ("same algorithm one matching", "sha256-deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead sha256-Bu681KMnQ15RYHFvsYdWumweeFAw0hJDTFt9seErghA=", false, false, true),
        ("unknown algorithm ignored", "foo666-U9WYDtBWkcHx13+9UKk/3Q5eoqDc4YGxYb07EPWzb9E=", false, false, true),
        ("strongest matching", "sha256-deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdead sha512-KZdenhzBd7X7Q/vmaOSyvFz1CGdoVt26xzCZjlkU9lfBEK+V/ougGys7iYDi0+tOHIQSQa87bIqx95R7GU7I9Q==", false, false, true),
        ("strongest incorrect", "sha512-LZdenhzBd7X7Q/vmaOSyvFz1CGdoVt26xzCZjlkU9lfBEK+V/ougGys7iYDi0+tOHIQSQa87bIqx95R7GU7I9Q== sha256-Bu681KMnQ15RYHFvsYdWumweeFAw0hJDTFt9seErghA=", false, false, false),
        ("options ignored", "sha256-Bu681KMnQ15RYHFvsYdWumweeFAw0hJDTFt9seErghA=?foo=bar?spam=eggs", false, false, true),
        ("cross CORS pass", SHA256, true, true, true),
        ("cross CORS fail", SHA256, true, false, false),
        ("cross empty CORS pass", "", true, true, true),
    ];
    let base = HttpUrl::parse("http://site.example/page").unwrap();
    for (name, integrity, cross, cors, expected) in cases {
        let url = if *cross {
            "http://other.example/script.js"
        } else {
            "http://site.example/script.js"
        };
        let mut init = ApiRequestInit::default();
        init.integrity = Some((*integrity).into());
        let mut request = ApiRequest::new(
            ApiRequestInput::Url(&url.encode_utf16().collect::<Vec<_>>()),
            init,
            &base,
            base.origin().clone(),
        )
        .unwrap();
        let result = fetch_api(
            &mut request,
            &mut Stub { cors: *cors },
            &Site::Tuple {
                scheme: "http".into(),
                domain: "site.example".into(),
            },
            &false,
        );
        assert_eq!(result.is_ok(), *expected, "{name}: {result:?}");
        if *cross && !cors {
            assert!(
                matches!(result, Err(ApiError::Network(FetchError::Cors))),
                "{name}"
            );
        }
        if *expected {
            assert_eq!(result.unwrap().body.bytes().unwrap(), SCRIPT, "{name}");
        }
    }
}

#[test]
fn cross_origin_no_cors_cannot_verify_integrity() {
    let base = HttpUrl::parse("http://site.example/page").unwrap();
    let url: Vec<u16> = "http://other.example/script.js".encode_utf16().collect();
    let mut init = ApiRequestInit::default();
    init.mode = Some("no-cors".into());
    init.integrity = Some(SHA256.into());
    let mut request = ApiRequest::new(
        ApiRequestInput::Url(&url),
        init,
        &base,
        base.origin().clone(),
    )
    .unwrap();
    let site = Site::Tuple {
        scheme: "http".into(),
        domain: "site.example".into(),
    };
    assert!(matches!(
        fetch_api(&mut request, &mut Stub { cors: true }, &site, &false),
        Err(ApiError::Network(FetchError::Integrity))
    ));
}
