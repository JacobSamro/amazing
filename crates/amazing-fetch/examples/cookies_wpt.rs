//! Native HTTP Set-Cookie cases extracted from pinned WPT cookies/ HTML.
#[path = "../tests/wpt/cookies/cases.rs"]
mod cases;

use amazing_fetch::cookies::{CookieAccess, CookieLimits, CookieSite, CookieStore};
use amazing_fetch::{HttpUrl, RegistrableDomain};

// The original 8 sources (max-age/expires/path/invalid/name/name-and-value/
// attributes.www.sub/domain-child.sub) were pulled at c48d5874. NET-35 widened
// the corpus with 8 more httpCookieTest/httpRedirectCookieTest-shaped files
// (name-ctl, secure.https, secure-non-secure-child, path-redirect, charset,
// ordering-child.sub, value, value-ctl) from the project's separately pinned
// WPT checkout at c2a2fce (the same revision the fetch/xhr corpus uses); RFC
// 6265 test vectors are stable enough that mixing the two is not a
// correctness concern, but re-running generate.mjs against a single revision
// would remove this note.
const REVISION: &str = "c48d58747e1f211527fb695fd60548a997fae617 + c2a2fce69e5c2509d9dc28440ec8b42532e5c293";
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
        partition_key: None,
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
    // WPT's `.https.html` naming convention means the whole page (and every
    // fetch()/Set-Cookie exchange it makes) runs in a secure context; e.g.
    // `secure.https.html`'s "Secure" attribute only takes effect over https,
    // so exercising it over http would just fail every case regardless of
    // engine correctness.
    let scheme = if source.contains(".https.") { "https" } else { "http" };
    let set_url =
        HttpUrl::parse(&format!("{scheme}://www.web-platform.test/cookies/resources/cookie.py"))
            .unwrap();
    let get_path = if default_path {
        "/cookies/resources/echo-cookie.html".to_owned()
    } else {
        match source {
            "name.html" | "name-ctl.html" => format!("/cookies/name/{source}"),
            "name-and-value.html" | "attributes.www.sub.html" => {
                format!("/cookies/size/{source}")
            }
            "secure-non-secure-child.html" => format!("/cookies/attributes/resources/{source}"),
            "charset.html" => format!("/cookies/encoding/{source}"),
            _ => format!("/cookies/attributes/{source}"),
        }
    };
    // `httpRedirectCookieTest`-derived cases carry `location` either as an
    // already-absolute URL (domain-child.sub.html, testing a redirect across
    // hosts) or as the root-relative path extracted straight from the WPT
    // source (path-redirect.html, ordering-child.sub.html); only the latter
    // needs the origin prepended.
    let get_url = HttpUrl::parse(&match location {
        Some(location) if location.starts_with('/') => {
            format!("{scheme}://www.web-platform.test{location}")
        }
        Some(location) => location.to_owned(),
        None => format!("{scheme}://www.web-platform.test{get_path}"),
    })
    .unwrap();
    let mut store = CookieStore::new(WptDomains, CookieLimits::default()).unwrap();
    for header in headers {
        // WPT processes each Set-Cookie field independently. A rejected field
        // does not cancel other fields on the same response.
        //
        // `value.html`'s "ignore value after LF" case models a raw octet
        // string handed straight to `cookie.py`'s response writer; on the
        // wire a bare CR/LF is itself the header-line terminator, so real
        // HTTP response parsing splits the line there before cookie-string
        // parsing ever sees it (the remainder becomes a separate, colon-less
        // and therefore discarded, header line). `CookieStore::set_cookie`
        // is fed the raw test string with no HTTP layer in front of it here,
        // so mirror that line split rather than asserting a policy on
        // embedded CTLs that production code never has to make.
        let line = header
            .split(|b: char| b == '\n' || b == '\r')
            .next()
            .unwrap_or(header);
        let _ = store.set_cookie(line.as_bytes(), &access(&set_url, true), NOW);
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
