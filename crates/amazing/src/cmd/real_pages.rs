//! Headless real-page runner for the IA-REAL-PAGES corpus.
//!
//! Unlike `render`, this path is lenient: a page script, inline handler or
//! event-loop failure is collected and reported instead of aborting the run, so
//! a saved real-world page still produces an image and a classification. Each
//! run prints a one-line `SUMMARY` record for the corpus harness and a
//! machine-readable JSON line with the collected errors.
use std::{cell::RefCell, process::ExitCode, rc::Rc, time::Instant};

use crate::dom_host::BrowserDomHost;

struct PageReport<'a> {
    page: &'a str,
    status: &'a str,
    errors: &'a [String],
    width: u32,
    height: u32,
    render_ms: u64,
    wrote_image: bool,
    nonwhite: usize,
}

fn json_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn emit(report: &PageReport<'_>) {
    let errors = report.errors.iter()
        .map(|error| json_string(error))
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"schema\":1,\"page\":{},\"status\":{},\"script_errors\":[{}],\"width\":{},\"height\":{},\"render_ms\":{},\"wrote_image\":{},\"nonwhite\":{}}}",
        json_string(report.page),
        json_string(report.status),
        errors,
        report.width,
        report.height,
        report.render_ms,
        report.wrote_image,
        report.nonwhite,
    );
    for error in report.errors {
        println!("ERROR\t{error}");
    }
    println!(
        "SUMMARY\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        report.status,
        report.errors.len(),
        report.render_ms,
        report.width,
        report.height,
        usize::from(report.wrote_image),
        report.nonwhite,
    );
}

/// `amazing render-page <page.html> <out.bmp> [viewport_width] [fixed_height]`.
/// Always exits 0 when it produced a report; the `status` field carries the
/// classification. Only unreadable input is a hard failure.
pub(crate) fn cmd_render_page(args: &[String]) -> ExitCode {
    let page = match args.first() {
        Some(page) => page.as_str(),
        None => {
            eprintln!("usage: amazing render-page <page.html> <out.bmp> [viewport_width] [fixed_height]");
            return ExitCode::FAILURE;
        }
    };
    let out_path = args.get(1).map(String::as_str).unwrap_or("render-page.bmp");
    let width: f32 = args.get(2).and_then(|w| w.parse().ok()).unwrap_or(1024.0);
    let fixed_height: Option<u32> = args.get(3).and_then(|h| h.parse().ok());

    let started = Instant::now();
    let html = match crate::cmd::parse::read_bytes(Some(page), "render-page") {
        Ok(bytes) => bytes,
        Err(code) => return code,
    };
    let document = match crate::document::parse_document_bytes(&html, page, None) {
        Ok(document) => document,
        Err(error) => {
            let errors = vec![error.to_string()];
            emit(&PageReport {
                page, status: "parse_error", errors: &errors, width: 0, height: 0,
                render_ms: started.elapsed().as_millis() as u64, wrote_image: false, nonwhite: 0,
            });
            return ExitCode::SUCCESS;
        }
    };
    let scripting = document.kind == crate::document::DocumentKind::Html;
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: document.dom,
        native_forwarded: false,
    }));
    let mut errors = if scripting {
        crate::cmd::js::execute_page_scripts_lenient(&host, page)
    } else {
        Vec::new()
    };
    let surface = match crate::cmd::render::paint_host_surface(&host, page, width, fixed_height, "") {
        Ok(surface) => surface,
        Err(error) => {
            errors.push(format!("paint: {error}"));
            emit(&PageReport {
                page, status: "render_error", errors: &errors, width: width as u32, height: 0,
                render_ms: started.elapsed().as_millis() as u64, wrote_image: false, nonwhite: 0,
            });
            return ExitCode::SUCCESS;
        }
    };
    let render_ms = started.elapsed().as_millis() as u64;
    let wrote_image = surface.save_bmp(out_path).is_ok();
    let nonwhite = surface.pixels.chunks_exact(4)
        .filter(|px| px[0] != 255 || px[1] != 255 || px[2] != 255)
        .count();
    emit(&PageReport {
        page, status: "ok", errors: &errors, width: surface.width, height: surface.height,
        render_ms, wrote_image, nonwhite,
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::json_string;

    #[test]
    fn json_string_escapes_control_and_quote() {
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("a\"b\\c\n\t"), "\"a\\\"b\\\\c\\n\\t\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }
}
