use amazing_net::url::{domain_to_ascii, domain_to_unicode, punycode_decode, punycode_encode};
use amazing_net::ParsedUrl;

#[test]
fn punycode_rfc3492_roundtrips_and_rejects_overflow() {
    for (unicode, payload) in [
        ("bücher", "bcher-kva"),
        ("mañana", "maana-pta"),
        ("faß", "fa-hia"),
        ("你好", "6qq79v"),
    ] {
        assert_eq!(punycode_encode(unicode).unwrap(), payload);
        assert_eq!(punycode_decode(payload).unwrap(), unicode);
    }
    assert!(punycode_decode("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
    assert!(punycode_decode("e!a").is_err());
}

#[test]
fn uts46_maps_normalizes_and_preserves_deviations() {
    assert_eq!(domain_to_ascii("Ｂücher。de").unwrap(), "xn--bcher-kva.de");
    assert_eq!(domain_to_ascii("a\u{0300}.de").unwrap(), "xn--0ca.de");
    assert_eq!(domain_to_ascii("faß.de").unwrap(), "xn--fa-hia.de");
    assert_eq!(domain_to_unicode("xn--bcher-kva.de"), "bücher.de");
    assert_eq!(
        ParsedUrl::parse("https://faß.de/").unwrap().hostname(),
        "xn--fa-hia.de"
    );
}

#[test]
fn joiner_bidi_and_punycode_label_validation() {
    assert!(domain_to_ascii("a\u{200c}b.de").is_err());
    assert!(domain_to_ascii("\u{200d}.de").is_err());
    assert!(domain_to_ascii("يa.de").is_err());
    assert!(domain_to_ascii("xn--a.ß").is_err());
    assert_eq!(domain_to_unicode("xn--8i7caa"), "xn--8i7caa");
}
