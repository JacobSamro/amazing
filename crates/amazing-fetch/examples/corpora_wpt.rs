//! Run pinned WPT JSON vectors natively, without a JS binding or serde.
use amazing_fetch::{process_data_url, MimeType};
use amazing_net::ParsedUrl;

type MimeCase = (&'static str, Option<&'static str>);
type DataCase = (&'static str, Option<&'static str>, Option<&'static [u8]>);
type Base64Case = (&'static str, Option<&'static [u8]>);
const MIME: &[MimeCase] = include!("../tests/wpt/mime-types.rs");
const GENERATED_MIME: &[MimeCase] = include!("../tests/wpt/generated-mime-types.rs");
const DATA_URLS: &[DataCase] = include!("../tests/wpt/data-urls.rs");
const BASE64: &[Base64Case] = include!("../tests/wpt/base64.rs");
const REVISION: &str = "c48d58747e1f211527fb695fd60548a997fae617";

fn run_mime(name: &str, cases: &[MimeCase]) -> usize {
    let mut failed = 0;
    for (index, (input, expected)) in cases.iter().enumerate() {
        let actual = MimeType::parse(input).map(|record| record.serialize());
        if actual.as_deref() != *expected {
            failed += 1;
            println!("FAIL {name} {index} expected={expected:?} actual={actual:?} input={input:?}");
        } else {
            println!("PASS {name} {index} input={input:?}");
        }
    }
    println!(
        "WPT revision={REVISION} corpus={name} objects={} pass={} fail={failed}",
        cases.len(),
        cases.len() - failed
    );
    failed
}
fn run_data() -> usize {
    let mut failed = 0;
    for (index, (input, mime, body)) in DATA_URLS.iter().enumerate() {
        let actual = ParsedUrl::parse(input)
            .ok()
            .and_then(|url| process_data_url(&url));
        let actual_mime = actual.as_ref().map(|value| value.mime_type.serialize());
        let actual_body = actual.as_ref().map(|value| value.body.as_slice());
        if actual_mime.as_deref() != *mime || actual_body != *body {
            failed += 1;
            println!("FAIL data-urls {index} expected={mime:?}/{body:?} actual={actual_mime:?}/{actual_body:?} input={input:?}");
        } else {
            println!("PASS data-urls {index} input={input:?}");
        }
    }
    println!(
        "WPT revision={REVISION} corpus=data-urls objects={} pass={} fail={failed}",
        DATA_URLS.len(),
        DATA_URLS.len() - failed
    );
    failed
}
fn run_base64() -> usize {
    let mut failed = 0;
    for (index, (input, expected)) in BASE64.iter().enumerate() {
        let url = format!("data:;base64,{input}");
        let actual = ParsedUrl::parse(&url)
            .ok()
            .and_then(|url| process_data_url(&url))
            .map(|data| data.body);
        if actual.as_deref() != *expected {
            failed += 1;
            println!("FAIL base64 {index} expected={expected:?} actual={actual:?} input={input:?}");
        } else {
            println!("PASS base64 {index} input={input:?}");
        }
    }
    println!(
        "WPT revision={REVISION} corpus=base64 objects={} pass={} fail={failed}",
        BASE64.len(),
        BASE64.len() - failed
    );
    failed
}
fn main() {
    let failed = run_mime("mime-types", MIME)
        + run_mime("generated-mime-types", GENERATED_MIME)
        + run_data()
        + run_base64();
    if failed != 0 {
        std::process::exit(1);
    }
}
