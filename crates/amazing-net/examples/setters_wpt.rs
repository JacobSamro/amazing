//! Native runner for pinned WPT URL setter vectors.
use amazing_net::ParsedUrl;

struct SetterCase {
    property: &'static str,
    index: usize,
    comment: &'static str,
    href: &'static str,
    new_value: &'static str,
    expected: &'static [(&'static str, &'static str)],
}
const CASES: &[SetterCase] = include!("../tests/wpt/setters_cases.rs");
const REVISION: &str = "c48d58747e1f211527fb695fd60548a997fae617";

fn apply(url: &mut ParsedUrl, property: &str, new_value: &str) {
    match property {
        "protocol" => url.set_protocol(new_value),
        "username" => url.set_username(new_value),
        "password" => url.set_password(new_value),
        "host" => url.set_host(new_value),
        "hostname" => url.set_hostname(new_value),
        "port" => url.set_port(new_value),
        "pathname" => url.set_pathname(new_value),
        "search" => url.set_search(new_value),
        "hash" => url.set_hash(new_value),
        "href" => {
            let _ = url.set_href(new_value);
        }
        other => panic!("unknown setter {other}"),
    }
}

fn value(url: &ParsedUrl, property: &str) -> String {
    match property {
        "href" => url.serialize(),
        "protocol" => format!("{}:", url.scheme()),
        "username" => url.username().to_owned(),
        "password" => url.password().to_owned(),
        "host" => url.host_and_port(),
        "hostname" => url.hostname(),
        "port" => url.port().map(|p| p.to_string()).unwrap_or_default(),
        "pathname" => url.pathname(),
        "search" => url
            .query()
            .filter(|q| !q.is_empty())
            .map(|q| format!("?{q}"))
            .unwrap_or_default(),
        "hash" => url
            .fragment()
            .filter(|f| !f.is_empty())
            .map(|f| format!("#{f}"))
            .unwrap_or_default(),
        other => panic!("unknown URL property: {other}"),
    }
}

fn main() {
    let baseline = std::env::args().any(|a| a == "--baseline");
    let mut failed = 0;
    for case in CASES {
        let mut url = match ParsedUrl::parse(case.href) {
            Ok(url) => url,
            Err(error) => {
                failed += 1;
                println!(
                    "FAIL {}[{}] {:?}: initial URL parse {error:?} href={:?}",
                    case.property, case.index, case.comment, case.href
                );
                continue;
            }
        };
        if !baseline {
            apply(&mut url, case.property, case.new_value);
        }
        let mut mismatch = false;
        for &(name, expected) in case.expected {
            let actual = value(&url, name);
            if actual != expected {
                mismatch = true;
                println!("FAIL {}[{}] {:?}: {name} expected={expected:?} actual={actual:?} href={:?} new_value={:?}", case.property, case.index, case.comment, case.href, case.new_value);
            }
        }
        if mismatch {
            failed += 1;
        }
    }
    println!("WPT revision={REVISION} selection=url/resources/setters_tests.json mode={} objects={} pass={} fail={failed}", if baseline {"baseline"} else {"setters"}, CASES.len(), CASES.len() - failed);
    if failed != 0 {
        std::process::exit(1);
    }
}
