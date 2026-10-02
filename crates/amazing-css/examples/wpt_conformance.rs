//! CSS-25: a Linux runner for the pinned WPT parsing/computed-value
//! testharness selections, driving `amazing-css` directly rather than
//! executing the JS test harness in a browser.
//!
//! WPT's `css/support/parsing-testcommon.js` and `computed-testcommon.js`
//! give every `*-valid.html`/`*-invalid.html`/`*-computed.html` file a
//! small, regular shape: top-level calls to `test_valid_value(property,
//! value[, serialized])`, `test_invalid_value(property, value)`, and
//! `test_computed_value(property, specified, computed)`, almost always with
//! plain quoted-string arguments. This extracts those call sites
//! statically (a small parenthesis/quote-aware scanner, not a JS parser) and
//! checks them against this crate's own parse/compute API:
//!
//! - valid:    `amazing_css::values::parses(property, value)` must be true.
//! - invalid:  the same call must be false.
//! - computed: cascade a lone `div { property: specified }` through
//!   `compute()` and compare `ComputedStyle::get(property)` textually
//!   against the expected computed serialization.
//!
//! Known limitations (see `docs/lanes/css_TODO.md`'s CSS-25 section for the
//! measured breakdown):
//! - A property this crate does not model a grammar for falls through to
//!   `ComputedValue::Unparsed`, which accepts any input; `values::parses`
//!   reports those as valid regardless of content, which is the honest
//!   signal for "this crate cannot yet reject bad values here" rather than a
//!   scoring bug.
//! - Computed-value string comparison is exact. This crate mostly retains
//!   cascade-winning declarations' literal (lightly-canonicalized) text
//!   rather than always re-serializing to the CSS Color 4/Values 5 computed
//!   forms WPT expects (e.g. `color(srgb 1 0 0)` for a plain named color),
//!   so computed-test failures include both real bugs and this
//!   known serialization-model gap; see the lane docs for the split.
//! - Assertions made through a per-file local JS wrapper function (rather
//!   than a direct `test_valid_value`/`test_invalid_value`/
//!   `test_computed_value` call) are not extracted, so counts are a lower
//!   bound on each file's true assertion count.
//!
//! Usage (on a Linux sandbox, after `bash scripts/ensure-wpt.sh`):
//!   WPT_ROOT=~/corpora/wpt cargo run -p amazing-css --example wpt_conformance [selection...]
//! With no arguments, runs the ten selections with matching files: css-color,
//! css-values, css-cascade, css-conditional, css-fonts, css-text,
//! css-backgrounds, css-transforms, css-flexbox, css-grid (widened in
//! CSS-27; see the lane docs for the measured breakdown). (`css/mediaqueries`
//! has no files in this exact shape -- its parsing tests use a per-file
//! `test_parsing(query, expected)` pattern against `matchMedia(...).media`
//! canonical serialization, which this crate does not implement; see the
//! lane docs.)

use amazing_css::{ComputedStyle, compute, parse_stylesheet, values::parses};
use amazing_dom::{Dom, NodeIdx};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Tally {
    pass: u32,
    fail: u32,
    skipped_non_literal: u32,
    failures: Vec<String>,
}

impl Tally {
    fn record(&mut self, ok: bool, detail: impl FnOnce() -> String) {
        if ok {
            self.pass += 1;
        } else {
            self.fail += 1;
            self.failures.push(detail());
        }
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    Invalid,
    Computed,
}

fn main() {
    let root = std::env::var("WPT_ROOT").expect("WPT_ROOT must point at a WPT checkout");
    let css_root = PathBuf::from(root).join("css");
    let requested: Vec<String> = std::env::args().skip(1).collect();
    let selections: Vec<String> = if requested.is_empty() {
        [
            "css-color", "css-values", "css-cascade", "css-conditional",
            "css-fonts", "css-text", "css-backgrounds", "css-transforms",
            "css-flexbox", "css-grid",
        ]
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        requested
    };

    let mut grand = Tally::default();
    let mut grand_files = 0usize;
    for selection in &selections {
        let dir = css_root.join(selection);
        let mut files = Vec::new();
        collect_files(&dir, &mut files);
        files.sort();
        let mut tally = Tally::default();
        for file in &files {
            run_file(file, &mut tally);
        }
        println!(
            "{selection}: {}/{} passed ({} files, {} non-literal args skipped)",
            tally.pass,
            tally.pass + tally.fail,
            files.len(),
            tally.skipped_non_literal
        );
        for failure in &tally.failures {
            println!("  FAIL {failure}");
        }
        grand_files += files.len();
        grand.pass += tally.pass;
        grand.fail += tally.fail;
        grand.skipped_non_literal += tally.skipped_non_literal;
    }
    println!(
        "TOTAL: {}/{} passed across {} files ({} non-literal args skipped)",
        grand.pass,
        grand.pass + grand.fail,
        grand_files,
        grand.skipped_non_literal
    );
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        if name.ends_with("-valid.html") || name.ends_with("-invalid.html") || name.ends_with("-computed.html") {
            out.push(path);
        }
    }
}

fn run_file(path: &Path, tally: &mut Tally) {
    let Ok(source) = std::fs::read_to_string(path) else { return };
    let name = path.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
    let (mode, function) = if name.ends_with("-invalid.html") {
        (Mode::Invalid, "test_invalid_value")
    } else if name.ends_with("-computed.html") {
        (Mode::Computed, "test_computed_value")
    } else {
        (Mode::Valid, "test_valid_value")
    };
    let display = path.to_string_lossy().into_owned();
    for call in find_calls(&source, function) {
        let Some(property) = call.first().and_then(Option::as_ref) else {
            tally.skipped_non_literal += 1;
            continue;
        };
        let Some(value) = call.get(1).and_then(Option::as_ref) else {
            tally.skipped_non_literal += 1;
            continue;
        };
        match mode {
            Mode::Valid => {
                let ok = parses(property, value);
                tally.record(ok, || format!("{display}: valid {property}: {value:?}"));
            }
            Mode::Invalid => {
                let ok = !parses(property, value);
                tally.record(ok, || format!("{display}: invalid {property}: {value:?} unexpectedly parsed"));
            }
            Mode::Computed => {
                // Omitted expected means specified; an explicitly present
                // dynamic expression cannot be evaluated by this static runner.
                // Never manufacture a pass by replacing it with specified.
                if call.len() > 2 && call[2].is_none() {
                    tally.skipped_non_literal += 1;
                    continue;
                }
                let Some(expected) = call.get(2).and_then(Option::as_ref).or(Some(value)) else {
                    tally.skipped_non_literal += 1;
                    continue;
                };
                let actual = computed_text(property, value);
                let ok = actual.as_deref() == Some(expected.as_str());
                tally.record(ok, || {
                    format!(
                        "{display}: computed {property}: {value:?} -> {actual:?}, expected {expected:?}"
                    )
                });
            }
        }
    }
}

fn computed_text(property: &str, value: &str) -> Option<String> {
    let mut dom = Dom::new();
    let html = dom.create_element(NodeIdx(0), "html");
    let body = dom.create_element(html, "body");
    let root = dom.create_element(body, "div");
    let css = format!("div {{ {property}: {value} }}");
    let styles = compute(&dom, &[parse_stylesheet(&css)]);
    let style: &ComputedStyle = styles.get(root)?;
    style.get(property).map(str::to_owned)
}

/// Find every top-level call to `name(...)` in `source` and return each
/// call's arguments, one `Option<String>` per top-level comma-separated
/// argument: `Some(text)` for a plain single/double-quoted JS string literal
/// (unescaped), `None` for anything else (an identifier, a template
/// literal, a nested call, a number...).
fn find_calls(source: &str, name: &str) -> Vec<Vec<Option<String>>> {
    let bytes = source.as_bytes();
    let pattern = format!("{name}(");
    let mut calls = Vec::new();
    let mut search_from = 0usize;
    while let Some(offset) = source[search_from..].find(&pattern) {
        let start = search_from + offset;
        let before_ok = start == 0 || {
            let previous = bytes[start - 1];
            !(previous.is_ascii_alphanumeric() || previous == b'_')
        };
        let args_start = start + pattern.len();
        if before_ok {
            if let Some((args_text, end)) = balanced_args(source, args_start) {
                calls.push(split_and_unquote(&args_text));
                search_from = end;
                continue;
            }
        }
        search_from = args_start;
    }
    calls
}

/// From just after a call's opening `(`, return the raw argument-list text
/// up to (not including) the matching `)`, plus the index just past it.
/// Tracks nested parens and both quote kinds so a `)` or `,` inside a string
/// literal or a nested call does not end the scan early.
fn balanced_args(source: &str, start: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    let mut quote: Option<u8> = None;
    let mut escape = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if ch == b'\\' {
                escape = true;
            } else if ch == q {
                quote = None;
            }
        } else {
            match ch {
                b'\'' | b'"' => quote = Some(ch),
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((source[start..i].to_owned(), i + 1));
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// Split a balanced argument-list string on top-level commas (outside
/// quotes/parens) and unquote each plain string-literal argument.
fn split_and_unquote(args: &str) -> Vec<Option<String>> {
    let bytes = args.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut escape = false;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let ch = bytes[i];
        if let Some(q) = quote {
            if escape {
                escape = false;
            } else if ch == b'\\' {
                escape = true;
            } else if ch == q {
                quote = None;
            }
        } else {
            match ch {
                b'\'' | b'"' => quote = Some(ch),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(unquote(args[start..i].trim()));
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    parts.push(unquote(args[start..].trim()));
    parts
}

fn unquote(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let quote = bytes[0];
    if (quote != b'\'' && quote != b'"') || bytes[bytes.len() - 1] != quote {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(escaped) => out.push(escaped),
                None => {}
            }
        } else {
            out.push(ch);
        }
    }
    Some(out)
}
