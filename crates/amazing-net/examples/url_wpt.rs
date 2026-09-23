//! Run the pinned URL data without serde, testharness.js, or network access.
//! Exit nonzero on any mismatch; print every unsupported case independently.
use amazing_net::url::AsciiOnly;
use amazing_net::{ParseError, ParsedUrl};
struct Case {
    index: usize,
    input: &'static str,
    base: Option<&'static str>,
    failure: bool,
    expected: [Option<&'static str>; 11],
}
const CASES: &[Case] = include!("../tests/wpt/cases.rs");
fn parse(case: &Case, baseline: bool) -> Result<ParsedUrl, ParseError> {
    let base = case
        .base
        .map(|input| {
            if baseline {
                ParsedUrl::parse_with_idna(input, None, &AsciiOnly)
            } else {
                ParsedUrl::parse(input)
            }
        })
        .transpose()?;
    if baseline {
        ParsedUrl::parse_with_idna(case.input, base.as_ref(), &AsciiOnly)
    } else {
        ParsedUrl::parse_with_base(case.input, base.as_ref())
    }
}
fn main() {
    let baseline = std::env::args().any(|arg| arg == "--baseline");
    let (mut passed, mut failed, mut unsupported) = (0, 0, 0);
    for case in CASES {
        let actual = parse(case, baseline);
        if matches!(actual, Err(ParseError::IdnaRequired)) {
            unsupported += 1;
            println!(
                "UNSUPPORTED {} IDNA input={:?} base={:?}",
                case.index, case.input, case.base
            );
            continue;
        }
        match actual {
            Err(_) if case.failure => passed += 1,
            Err(e) => {
                failed += 1;
                println!(
                    "FAIL {} expected URL, got {e:?} input={:?} base={:?}",
                    case.index, case.input, case.base
                );
            }
            Ok(url) if case.failure => {
                failed += 1;
                println!(
                    "FAIL {} expected failure, got {} input={:?}",
                    case.index, url, case.input
                );
            }
            Ok(url) => {
                let fields = [
                    url.serialize(),
                    url.origin().serialize(),
                    format!("{}:", url.scheme()),
                    url.username().into(),
                    url.password().into(),
                    url.host_and_port(),
                    url.hostname(),
                    url.port().map(|p| p.to_string()).unwrap_or_default(),
                    url.pathname(),
                    url.query()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("?{s}"))
                        .unwrap_or_default(),
                    url.fragment()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("#{s}"))
                        .unwrap_or_default(),
                ];
                let names = [
                    "href", "origin", "protocol", "username", "password", "host", "hostname",
                    "port", "pathname", "search", "hash",
                ];
                let mut mismatch = false;
                for (i, expected) in case.expected.iter().enumerate() {
                    if let Some(expected) = expected {
                        if fields[i] != *expected {
                            mismatch = true;
                            println!(
                                "FAIL {} {} expected={expected:?} actual={:?} input={:?} base={:?}",
                                case.index, names[i], fields[i], case.input, case.base
                            );
                        }
                    }
                }
                if mismatch {
                    failed += 1;
                } else {
                    passed += 1;
                }
            }
        }
    }
    println!("WPT revision=c48d58747e1f211527fb695fd60548a997fae617 selection=url/resources/urltestdata.json objects={} pass={passed} fail={failed} timeout=0 unsupported={unsupported}",CASES.len());
    if failed != 0 {
        std::process::exit(1);
    }
}
