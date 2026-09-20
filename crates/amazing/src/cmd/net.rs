//! Fetch and fetch-to-parser commands.
use std::process::ExitCode;
/// Phase 5 demo: `amazing fetch <url|file|path> [-i]`. Performs an HTTP/1.1
/// GET (or reads a file:// / bare path), prints status + headers + body.
/// `-i` includes the response headers.
pub(crate) fn cmd_fetch(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(p) if !p.starts_with('-') => p,
        _ => {
            eprintln!("usage: amazing fetch <url|file> [-i]");
            return ExitCode::FAILURE;
        }
    };
    let show_headers = args.iter().any(|a| a == "-i");

    match amazing_net::fetch(target, &amazing_net::FetchOptions::default()) {
        Ok(resp) => {
            println!("{} {} ({})", resp.status, resp.reason, resp.url);
            if show_headers {
                for (name, value) in resp.headers.iter() {
                    println!("{name}: {value}");
                }
                println!();
            }
            print!("{}", resp.text().unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("amazing fetch: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 5 integration artifact: fetch an HTTP response and feed its decoded
/// body directly into the Phase 1 parser before dumping the resulting DOM.
pub(crate) fn cmd_fetch_parse(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(target) if target.starts_with("http://") => target,
        _ => {
            eprintln!("usage: amazing fetch-parse <http-url>");
            return ExitCode::FAILURE;
        }
    };
    let response = match amazing_net::fetch(target, &amazing_net::FetchOptions::default()) {
        Ok(response) if response.is_success() => response,
        Ok(response) => {
            eprintln!("amazing fetch-parse: HTTP status {}", response.status);
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("amazing fetch-parse: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = response.text().unwrap_or_default();
    let dom = amazing_html::parse(&source);
    print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
    ExitCode::SUCCESS
}

