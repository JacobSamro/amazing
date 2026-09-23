//! Native runner for the pinned WPT IDNA JSON selections.
use amazing_net::url::{domain_to_ascii, AsciiOnly, IdnaProvider};

struct Case {
    index: usize,
    input: Option<&'static str>,
    output: Option<&'static str>,
    url_only: bool,
}

const TOASCII: &[Case] = &include!("../tests/wpt/idna/toascii_cases.rs");
const IDNA_V2: &[Case] = &include!("../tests/wpt/idna/IdnaTestV2_cases.rs");

fn forbidden(c: char) -> bool {
    c <= '\u{1f}'
        || c == '\u{7f}'
        || matches!(
            c,
            ' ' | '#' | '/' | ':' | '<' | '>' | '?' | '@' | '[' | '\\' | ']' | '^' | '|' | '%'
        )
}

fn run(name: &str, cases: &[Case], baseline: bool) -> (usize, usize, usize) {
    let (mut pass, mut fail, mut unsupported) = (0, 0, 0);
    for case in cases {
        let input = case.input.expect("WPT input");
        let result = if baseline && !input.is_ascii() {
            AsciiOnly.domain_to_ascii(input).ok()
        } else if baseline {
            Some(input.to_ascii_lowercase())
        } else {
            domain_to_ascii(input).ok()
        };
        if baseline && !input.is_ascii() {
            unsupported += 1;
            continue;
        }
        let result = result.filter(|s| !s.chars().any(forbidden));
        if result.as_deref() == case.output {
            pass += 1;
        } else {
            fail += 1;
            println!(
                "FAIL {name} {} url_only={} input={input:?} expected={:?} actual={result:?}",
                case.index, case.url_only, case.output
            );
        }
    }
    println!("WPT revision=c48d58747e1f211527fb695fd60548a997fae617 selection=url/resources/{name}.json objects={} pass={pass} fail={fail} timeout=0 unsupported={unsupported}",cases.len());
    (pass, fail, unsupported)
}

fn main() {
    let baseline = std::env::args().any(|a| a == "--baseline");
    let a = run("toascii", TOASCII, baseline);
    let b = run("IdnaTestV2", IDNA_V2, baseline);
    if a.1 + b.1 > 0 {
        std::process::exit(1);
    }
}
