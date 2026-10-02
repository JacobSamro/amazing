use amazing_net::url::{percent_encode, Host, IdnaProvider, PercentEncodeSet as Set};
use amazing_net::{ParseError, ParsedUrl, UrlOrigin};
fn u(input: &str) -> ParsedUrl {
    ParsedUrl::parse(input).unwrap()
}
#[test]
fn document_encoding_applies_to_whole_query_only() {
    use std::cell::RefCell;
    let calls = RefCell::new(Vec::new());
    let encoder = |text: &str| {
        calls.borrow_mut().push(text.to_owned());
        // A stateful legacy encoding's complete output, including reset bytes.
        b"\x1b$B$\"$$\x1b(B'%26%23128512%3B".to_vec()
    };
    let url = ParsedUrl::parse_with_query_encoder(
        "https://example.com/あ?あい'😀#あ", None, &encoder,
    ).unwrap();
    assert_eq!(calls.borrow().as_slice(), ["あい'😀"]);
    assert_eq!(url.pathname(), "/%E3%81%82");
    assert_eq!(url.query(), Some("%1B$B$%22$$%1B(B%27%26%23128512%3B"));
    assert_eq!(url.fragment(), Some("%E3%81%82"));
    calls.borrow_mut().clear();
    for input in ["ws://example.com/?あ", "wss://example.com/?あ", "custom:opaque?あ"] {
        let parsed = ParsedUrl::parse_with_query_encoder(input, None, &encoder).unwrap();
        assert_eq!(parsed.query(), Some("%E3%81%82"));
    }
    assert!(calls.borrow().is_empty());
    let inherited = url.resolve_with_query_encoder("#new", &encoder).unwrap();
    assert_eq!(inherited.query(), url.query());
    assert!(calls.borrow().is_empty());
    let replaced = url.resolve_with_query_encoder("?あい'😀", &encoder).unwrap();
    assert_eq!(replaced.query(), url.query());
    assert_eq!(calls.borrow().as_slice(), ["あい'😀"]);
}
#[test]
fn percent_encode_sets_match_their_distinct_boundaries() {
    let input = " \"#<>?^`{}/:;=@[\\]|$%&+,é\0";
    assert_eq!(
        percent_encode(input, Set::C0Control),
        " \"#<>?^`{}/:;=@[\\]|$%&+,%C3%A9%00"
    );
    assert_eq!(percent_encode(" #?'`", Set::Fragment), "%20#?'%60");
    assert_eq!(percent_encode(" #?'`", Set::Query), "%20%23?'`");
    assert_eq!(percent_encode(" #?'`", Set::SpecialQuery), "%20%23?%27`");
    assert_eq!(percent_encode("/?^`{}\\", Set::Path), "/%3F%5E%60%7B%7D\\");
    assert_eq!(
        percent_encode("/:;=@[\\]|", Set::Userinfo),
        "%2F%3A%3B%3D%40%5B%5C%5D%7C"
    );
    assert_eq!(percent_encode("$%&+,", Set::Component), "%24%25%26%2B%2C");
}
#[test]
fn special_scheme_default_ports_are_null_but_effective_ports_exist() {
    for (scheme, port) in [
        ("ftp", 21),
        ("http", 80),
        ("https", 443),
        ("ws", 80),
        ("wss", 443),
    ] {
        let url = u(&format!("{scheme}://EXAMPLE.com:{port}"));
        assert_eq!(url.port(), None);
        assert_eq!(url.effective_port(), Some(port));
        assert_eq!(url.serialize(), format!("{scheme}://example.com/"));
    }
    assert_eq!(u("file:///x").effective_port(), None);
    assert_eq!(u("custom://example:21/x").port(), Some(21));
}
#[test]
fn validation_error_recovery_is_distinct_from_failure() {
    assert_eq!(
        u(" \0h\tt\r\ntp:\\\\ExAmPlE.com\\a\\%2e%2E\\b \n").serialize(),
        "http://example.com/b"
    );
    assert_eq!(u("https:example.com").serialize(), "https://example.com/");
    assert_eq!(u("http://x/%zz").serialize(), "http://x/%zz");
    assert_eq!(ParsedUrl::parse("http://x:65536/"), Err(ParseError::Port));
    assert_eq!(ParsedUrl::parse("http://x:3x/"), Err(ParseError::Port));
    assert!(ParsedUrl::parse("http://").is_err());
}
#[test]
fn relative_reference_preserves_or_replaces_each_component() {
    let base = u("https://u:p@a.test:444/a/b?q#f");
    for (input, expected) in [
        ("", "https://u:p@a.test:444/a/b?q"),
        ("?", "https://u:p@a.test:444/a/b?"),
        ("#", "https://u:p@a.test:444/a/b?q#"),
        ("../c", "https://u:p@a.test:444/c"),
        ("//b.test/a", "https://b.test/a"),
        ("https:c", "https://u:p@a.test:444/a/c"),
    ] {
        assert_eq!(base.resolve(input).unwrap().serialize(), expected);
    }
}
#[test]
fn userinfo_is_encoded_and_not_lost_before_cors_policy() {
    let url = u("http://a@b:c:d@EXAMPLE.com/");
    assert_eq!(url.username(), "a%40b");
    assert_eq!(url.password(), "c%3Ad");
    assert!(url.includes_credentials());
    assert_eq!(url.origin().serialize(), "http://example.com");
    assert!(!u("http://:@example.com/").includes_credentials());
}
#[test]
fn ipv4_radix_short_forms_and_bounds() {
    for input in ["127.1", "0177.0.0.1", "0x7f000001", "2130706433"] {
        assert_eq!(u(&format!("http://{input}/")).hostname(), "127.0.0.1");
    }
    for input in [
        "1.2.3.256",
        "1.2.65536",
        "4294967296",
        "08",
        "1.2.3.4.5",
        "example.123",
    ] {
        assert_eq!(
            ParsedUrl::parse(&format!("http://{input}/")),
            Err(ParseError::Ipv4)
        );
    }
    assert_eq!(u("http://0x/").hostname(), "0.0.0.0");
    assert_eq!(
        u("http://0000000000000000000000000000000000000001/").hostname(),
        "0.0.0.1"
    );
}
#[test]
fn ipv6_uses_hex_for_mapped_addresses_and_first_longest_zero_run() {
    assert_eq!(
        u("http://[::ffff:192.0.2.1]/").hostname(),
        "[::ffff:c000:201]"
    );
    assert_eq!(u("http://[1:0:0:2:0:0:3:4]/").hostname(), "[1::2:0:0:3:4]");
    for input in [
        "[1::2::3]",
        "[::ffff:192.00.2.1]",
        "[::1%25eth0]",
        "[1:2:3:4:5:6:7:8:9]",
    ] {
        assert_eq!(
            ParsedUrl::parse(&format!("https://{input}/")),
            Err(ParseError::Ipv6)
        );
    }
}
#[test]
fn file_drive_root_survives_shortening_and_localhost_normalizes() {
    let url = u("file://localhost/C|/a/b");
    assert_eq!(url.serialize(), "file:///C:/a/b");
    assert_eq!(
        url.resolve("../../../c").unwrap().serialize(),
        "file:///C:/c"
    );
    assert_eq!(url.resolve("/c").unwrap().serialize(), "file:///C:/c");
    assert_eq!(u("file://D|/a").serialize(), "file:///D:/a");
    assert_eq!(u("file://server/share").hostname(), "server");
    assert!(ParsedUrl::parse("file://user@server/share").is_err());
    assert!(ParsedUrl::parse("file://server:80/share").is_err());
}
#[test]
fn opaque_paths_do_not_get_hierarchical_resolution() {
    let url = u("data:a/../b ?q#x");
    assert!(url.has_opaque_path());
    assert_eq!(url.pathname(), "a/../b%20");
    assert_eq!(
        url.resolve("#new").unwrap().serialize(),
        "data:a/../b%20?q#new"
    );
    assert_eq!(url.resolve(""), Err(ParseError::MissingSchemeOrBase));
    assert_eq!(url.resolve("c"), Err(ParseError::MissingSchemeOrBase));
    assert_eq!(u("custom:/a/..//b").serialize(), "custom:/.//b");
}
#[test]
fn opaque_host_is_not_lowercased_or_idna_processed() {
    let url = u("custom://HéLLo/%E9");
    assert_eq!(url.hostname(), "H%C3%A9LLo");
    assert!(matches!(url.host(), Some(Host::Opaque(_))));
    assert_eq!(
        ParsedUrl::parse("https://é/").unwrap().hostname(),
        "xn--9ca"
    );
    assert_eq!(
        ParsedUrl::parse("https://%C3%A9/").unwrap().hostname(),
        "xn--9ca"
    );
}
#[test]
fn idna_seam_receives_decoded_unicode_and_cannot_inject_authority() {
    struct Provider;
    impl IdnaProvider for Provider {
        fn domain_to_ascii(&self, domain: &str) -> Result<String, ParseError> {
            assert_eq!(domain, "é.example");
            Ok("xn--9ca.example".into())
        }
    }
    let url = ParsedUrl::parse_with_idna("https://%C3%A9.example/", None, &Provider).unwrap();
    assert_eq!(url.hostname(), "xn--9ca.example");
    struct Invalid;
    impl IdnaProvider for Invalid {
        fn domain_to_ascii(&self, _: &str) -> Result<String, ParseError> {
            Ok("evil.test/path".into())
        }
    }
    assert_eq!(
        ParsedUrl::parse_with_idna("https://é/", None, &Invalid),
        Err(ParseError::Host)
    );
}
#[test]
fn url_origin_projects_tuples_and_signals_fresh_opaque_identity() {
    let UrlOrigin::Tuple(origin) = u("https://user:pass@a.test:443/x?q#f").origin() else {
        panic!()
    };
    assert_eq!(origin.scheme(), "https");
    assert_eq!(origin.effective_port(), Some(443));
    assert_eq!(origin.serialize(), "https://a.test");
    assert_eq!(
        u("blob:https://a.test:8443/id").origin().serialize(),
        "https://a.test:8443"
    );
    for input in [
        "file:///x",
        "data:text/plain,x",
        "custom://host",
        "blob:ftp://a.test/id",
    ] {
        assert!(matches!(u(input).origin(), UrlOrigin::Opaque));
    }
}
#[test]
fn serialization_preserves_empty_query_fragment_and_roundtrips() {
    for input in [
        "https://a.test/?#",
        "custom://host",
        "custom:/a/..//b",
        "file://c:/a/../b",
        "data:text/plain,hello world?x#y",
        "https://[::ffff:127.0.0.1]/",
    ] {
        let url = u(input);
        assert_eq!(u(&url.serialize()), url);
    }
    let url = u("https://a.test/?#");
    assert_eq!(url.query(), Some(""));
    assert_eq!(url.fragment(), Some(""));
    assert_eq!(url.serialize_without_fragment(), "https://a.test/?");
}
