//! Native assertion ports from WPT url/urlsearchparams-{append,delete,get,getall,has,set,sort,size,stringifier}.any.js
//! at c48d58747e1f211527fb695fd60548a997fae617.
use amazing_net::url::{
    parse_form_urlencoded, serialize_form_urlencoded, UrlSearchParams, UrlWithSearchParams,
};
#[test]
fn form_urlencoded_parse_and_serialize() {
    assert_eq!(
        parse_form_urlencoded("a=b+c&&a=%E2%98%83&bad=%ff"),
        vec![
            ("a".into(), "b c".into()),
            ("a".into(), "☃".into()),
            ("bad".into(), "�".into())
        ]
    );
    assert_eq!(
        parse_form_urlencoded("?a=b"),
        vec![("?a".into(), "b".into())]
    );
    assert_eq!(
        serialize_form_urlencoded(&[("a b".into(), "+☃".into())]),
        "a+b=%2B%E2%98%83"
    );
}
#[test]
fn append_delete_get_getall_has_set() {
    let mut p = UrlSearchParams::new("a=b&c=d&a=e");
    assert_eq!(p.get("a").as_deref(), Some("b"));
    assert_eq!(p.get("z"), None);
    assert_eq!(p.get_all("a"), ["b", "e"]);
    assert!(p.has("a", Some("e")));
    assert!(!p.has("a", Some("x")));
    assert_eq!(p.size(), 3);
    p.append("a", "f");
    assert_eq!(p.to_string(), "a=b&c=d&a=e&a=f");
    p.delete("a", Some("e"));
    assert_eq!(p.to_string(), "a=b&c=d&a=f");
    p.set("a", "B");
    assert_eq!(p.to_string(), "a=B&c=d");
    p.delete("c", None);
    assert_eq!(p.to_string(), "a=B");
    p.set("z", "x");
    assert_eq!(p.to_string(), "a=B&z=x");
}
#[test]
fn stable_utf16_sort_and_stringification() {
    let mut p = UrlSearchParams::new("z=1&a=1&z=2&%F0%90%80%80=astral&%EE%80%80=bmp");
    p.sort();
    assert_eq!(p.get_all("z"), ["1", "2"]);
    assert_eq!(p.pairs()[0].0, "a");
    assert_eq!(p.pairs()[1].0, "z");
    assert_eq!(p.pairs()[3].0, "𐀀");
    let p = UrlSearchParams::new("a=b+c&a%2Bb=c&x=%26%3D");
    assert_eq!(p.to_string(), "a=b+c&a%2Bb=c&x=%26%3D");
}
#[test]
fn live_url_query_update() {
    let url = UrlWithSearchParams::parse("http://example.com/?a=b&c=d").unwrap();
    let mut first = url.search_params();
    let mut second = url.search_params();
    first.delete("a", None);
    assert_eq!(url.serialize(), "http://example.com/?c=d");
    assert_eq!(second.get("a"), None);
    second.append("x", "a b");
    assert_eq!(url.serialize(), "http://example.com/?c=d&x=a+b");
    url.set_search("?q=1");
    assert_eq!(first.get("q").as_deref(), Some("1"));
    first.delete("q", None);
    assert_eq!(url.serialize(), "http://example.com/");
}
