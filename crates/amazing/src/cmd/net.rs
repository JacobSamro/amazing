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

/// Phase 5 integration artifact: fetch an HTTP response and pass its bytes
/// and transport charset to document-mode selection before parsing.
pub(crate) fn cmd_fetch_parse(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(target) if target.starts_with("http://") => target,
        _ => {
            eprintln!("usage: amazing fetch-parse <http-url>");
            return ExitCode::FAILURE;
        }
    };
    let response = match amazing_net::fetch_stream(
        target,
        &amazing_net::FetchOptions::default(),
        amazing_net::StreamOptions::default(),
    ) {
        Ok(response) if (200..300).contains(&response.status) => response,
        Ok(response) => {
            eprintln!("amazing fetch-parse: HTTP status {}", response.status);
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("amazing fetch-parse: {error}");
            return ExitCode::FAILURE;
        }
    };
    match crate::document::parse_streaming_document(response) {
        Ok((document, _metrics)) => {
            print!("{}", document.dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing fetch-parse: {error}");
            error.exit_code()
        }
    }
}
