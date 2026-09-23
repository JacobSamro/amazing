//! Native HTTP Set-Cookie cases extracted from pinned WPT cookies/ HTML.
#[path = "../tests/wpt/cookies/cases.rs"]
mod cases;

use amazing_fetch::cookies::{CookieAccess, CookieLimits, CookieSite, CookieStore};
use amazing_fetch::{HttpUrl, RegistrableDomain};

const REVISION: &str = "c48d58747e1f211527fb695fd60548a997fae617";
const NOW: i64 = 1_790_121_600; // 2026-09-23 UTC; WPT dates straddle this time.

#[derive(Clone, Copy)]
struct WptDomains;
impl RegistrableDomain for WptDomains {
    fn registrable_domain(&self, host: &str) -> Option<String> {
        host.ends_with(".web-platform.test")
            .then(|| "web-platform.test".to_owned())
    }
    fn is_public_suffix(&self, host: &str) -> Option<bool> {
        Some(host == "test")
    }
}

fn access<'a>(url: &'a HttpUrl, http_api: bool) -> CookieAccess<'a> {
    CookieAccess {
        url,
        site_for_cookies: CookieSite::NoClient,
        http_api,
        top_level_navigation: false,
        method: "GET",
    }
}

fn run(
    source: &str,
    name: &str,
    headers: &[&str],
    expected: &str,
    default_path: bool,
    location: Option<&str>,
) -> bool {
    let set_url =
        HttpUrl::parse("http://www.web-platform.test/cookies/resources/cookie.py").unwrap();
    let get_path = if default_path {
        "/cookies/resources/echo-cookie.html".to_owned()
    } else {
        match source {
            "name.html" => "/cookies/name/name.html".to_owned(),
            "name-and-value.html" | "attributes.www.sub.html" => {
                format!("/cookies/size/{source}")
            }
            _ => format!("/cookies/attributes/{source}"),
        }
    };
    let get_url = HttpUrl::parse(
        &location
            .map(str::to_owned)
            .unwrap_or_else(|| format!("http://www.web-platform.test{get_path}")),
    )
    .unwrap();
    let mut store = CookieStore::new(WptDomains, CookieLimits::default()).unwrap();
    for header in headers {
        // WPT processes each Set-Cookie field independently. A rejected field
        // does not cancel other fields on the same response.
        let _ = store.set_cookie(header.as_bytes(), &access(&set_url, true), NOW);
    }
    let actual = store.cookie_header(&access(&get_url, false), NOW).unwrap();
    if actual == expected.as_bytes() {
        true
    } else {
        println!(
            "FAIL {source}: {name} expected={expected:?} actual={:?}",
            String::from_utf8_lossy(&actual)
        );
        false
    }
}

fn main() {
    let mut failed = 0;
    for &(source, name, headers, expected, default_path, location) in cases::CASES {
        if !run(source, name, headers, expected, default_path, location) {
            failed += 1;
        }
    }
    println!(
        "WPT revision={REVISION} corpus=cookies/http objects={} pass={} fail={failed}",
        cases::CASES.len(),
        cases::CASES.len() - failed
    );
    if failed != 0 {
        std::process::exit(1);
    }
}
