//! IA-LIVE-PAGES live corpus harness.
//!
//! Fetches every URL in `tests/live-pages/urls.txt` through the lenient
//! headless `render-page` path in a child process, so a crash or timeout is
//! attributed to one page. Each page also runs `tests/live-pages/probe.js` in
//! its own interpreter and reports which common web APIs are missing.
//!
//! This test needs the public network, so it is `#[ignore]` and the normal
//! `--workspace` gate never runs it. Run it explicitly on the Boat sandbox:
//!
//!   AMAZING_LIVE_PAGES_EVIDENCE=$HOME/live-evidence \
//!     cargo nextest run -p amazing --locked --run-ignored \
//!       -E 'test(live_pages)' --no-capture
//!
//! It writes `run.tsv`, `top-missing-apis.md` and `errors.md` to
//! `$AMAZING_LIVE_PAGES_EVIDENCE` (or `crates/amazing/tests/live_pages/evidence`).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/live-pages")
}

fn read_urls(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("urls missing at {}: {error}", path.display()));
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn looks_like_missing_feature(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "not supported", "unsupported", "not implemented", "is not a function",
        "is not defined", "not a constructor", "cannot read propert",
        "cannot set propert", "non-element", "read-only property", "no such",
        "missing", "unknown", "not callable",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Collapse a renderer warning like `image "URL": UnsupportedSvg` to its error
/// kind so per-URL messages cluster.
fn warning_kind(warning: &str) -> String {
    match warning.rsplit_once("\": ") {
        Some((_, kind)) => kind.to_owned(),
        None => warning.to_owned(),
    }
}

fn sanitize(url: &str) -> String {
    let mut out = String::new();
    for c in url.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c == '/' || c == '.' || c == ':' {
            out.push('_');
        }
    }
    if out.len() > 80 {
        out.truncate(80);
    }
    out
}

struct Row {
    url: String,
    status: String,
    errors: Vec<String>,
    warnings: Vec<String>,
    probe: Vec<(String, String)>,
    render_ms: u64,
    total_ms: u64,
    exit: i32,
    timed_out: bool,
    wrote_image: bool,
    nonwhite: usize,
}

fn run_page(binary: &str, url: &str, probe: &Path, out_dir: &Path, timeout: Duration) -> Row {
    let out = out_dir.join(format!("{}.bmp", sanitize(url)));
    let started = Instant::now();
    let mut child = Command::new(binary)
        .arg("render-page")
        .arg(url)
        .arg(&out)
        .arg("1024")
        .env("AMAZING_PAGE_PROBE", probe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn render-page");
    let stdout = child.stdout.take().expect("stdout pipe");
    let stderr = child.stderr.take().expect("stderr pipe");
    let out_reader = std::thread::spawn(move || read_to_string(stdout));
    let err_reader = std::thread::spawn(move || read_to_string(stderr));

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let exit = loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => break status.code().unwrap_or(-1),
            None if Instant::now() > deadline => {
                let _ = child.kill();
                timed_out = true;
                break -1;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    let total_ms = started.elapsed().as_millis() as u64;
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();

    let mut status = "no_report".to_owned();
    let mut render_ms = 0;
    let mut wrote_image = false;
    let mut nonwhite = 0;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut probe_values = Vec::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("SUMMARY\t") {
            let fields: Vec<&str> = rest.split('\t').collect();
            if fields.len() >= 7 {
                status = fields[0].to_owned();
                render_ms = fields[2].parse().unwrap_or(0);
                wrote_image = fields[5] == "1";
                nonwhite = fields[6].parse().unwrap_or(0);
            }
        } else if let Some(error) = line.strip_prefix("ERROR\t") {
            errors.push(error.to_owned());
        } else if let Some(record) = line.strip_prefix("PROBE\t")
            && let Some(rest) = record.strip_prefix("API|")
        {
            let mut parts = rest.splitn(2, '|');
            if let (Some(name), Some(kind)) = (parts.next(), parts.next()) {
                probe_values.push((name.to_owned(), kind.to_owned()));
            }
        }
    }
    if (exit != 0 || status == "no_report") && !stderr.trim().is_empty() {
        for line in stderr.lines().take(4) {
            if !line.trim().is_empty() {
                errors.push(format!("stderr: {line}"));
            }
        }
    }
    // Media/resource warnings are printed by the renderer even on a successful
    // page; keep them separate from errors so the live report can cluster them
    // (for example UnsupportedSvg or an unsupported image format).
    for line in stderr.lines() {
        if let Some(warning) = line.strip_prefix("amazing render: ") {
            warnings.push(warning.to_owned());
        }
    }
    if timed_out {
        errors.push(format!("timeout after {}s", timeout.as_secs()));
    }
    Row {
        url: url.to_owned(),
        status,
        errors,
        warnings,
        probe: probe_values,
        render_ms,
        total_ms,
        exit,
        timed_out,
        wrote_image,
        nonwhite,
    }
}

fn read_to_string(mut reader: impl Read) -> String {
    let mut buf = String::new();
    let _ = reader.read_to_string(&mut buf);
    buf
}

#[test]
#[ignore = "live network corpus; run explicitly on the Boat sandbox"]
fn live_pages_corpus() {
    let root = corpus_root();
    let urls = read_urls(&root.join("urls.txt"));
    assert!(urls.len() >= 30, "expected the ~40-URL live corpus, found {}", urls.len());
    let probe = root.join("probe.js");
    assert!(probe.is_file(), "probe missing at {}", probe.display());

    let out_dir = std::env::temp_dir().join(format!("amazing-live-pages-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    // Pooled sandboxes share ~/amazing/target across lanes, so another lane's
    // `cargo build -p amazing` can replace CARGO_BIN_EXE_amazing mid-run (seen:
    // the first pages used this branch's URL-aware binary, later pages a
    // sibling's file-only one). Prefer a binary pinned by the run command
    // (`AMAZING_LIVE_PAGES_BIN`) and run a private copy either way, so the whole
    // corpus uses the code this test was built with.
    let source_binary = std::env::var_os("AMAZING_LIVE_PAGES_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_amazing")));
    let stable_binary = out_dir.join(if cfg!(windows) { "amazing-live.exe" } else { "amazing-live" });
    std::fs::copy(&source_binary, &stable_binary)
        .unwrap_or_else(|error| panic!("copy {}: {error}", source_binary.display()));
    let binary = stable_binary.to_str().expect("binary path is UTF-8");
    let timeout = Duration::from_secs(150);

    let mut rows = Vec::new();
    for url in &urls {
        let row = run_page(binary, url, &probe, &out_dir, timeout);
        println!(
            "{}\t{}\terrors={}\t{}ms",
            row.url, row.status, row.errors.len(), row.total_ms
        );
        rows.push(row);
    }
    let _ = std::fs::remove_dir_all(&out_dir);

    // Aggregates.
    let mut table = String::from("url\tstatus\terrors\twarnings\tfallbacks\trender_ms\ttotal_ms\tnonwhite\texit\tfirst_error\n");
    let mut errored = 0;
    let mut crashes = 0;
    let mut missing: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut probed: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut error_freq: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut warn_freq: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for row in &rows {
        let fallbacks = row.errors.iter().filter(|e| looks_like_missing_feature(e)).count();
        if !row.errors.is_empty() {
            errored += 1;
        }
        if row.exit != 0 || row.timed_out || row.status == "no_report" {
            crashes += 1;
        }
        for error in &row.errors {
            *error_freq.entry(error.clone()).or_default() += 1;
        }
        for warning in &row.warnings {
            *warn_freq.entry(warning_kind(warning)).or_default() += 1;
        }
        for (name, kind) in &row.probe {
            if kind != "error" {
                *probed.entry(name.clone()).or_default() += 1;
            }
            if kind == "undefined" {
                *missing.entry(name.clone()).or_default() += 1;
            }
        }
        table.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.url, row.status, row.errors.len(), row.warnings.len(), fallbacks,
            row.render_ms, row.total_ms, row.nonwhite, row.exit,
            row.errors.first().map(String::as_str).unwrap_or(""),
        ));
    }
    println!("{table}");
    println!(
        "live-pages: {} urls, {} with errors, {} fetch/no-report failures",
        rows.len(), errored, crashes
    );

    // Top missing APIs by page count (top 20).
    let mut ranked: Vec<(&String, &usize)> = missing.iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut api_report = String::from(
        "# IA-LIVE-PAGES — top missing web APIs by page count\n\n\
         `probe.js` inspects each page's own realm after its scripts. An API is \
         \"missing\" when `typeof` is `undefined`. Counts are pages, not calls.\n\n\
         | # | API | pages missing | pages probed |\n|---:|---|---:|---:|\n",
    );
    for (i, (name, count)) in ranked.iter().take(20).enumerate() {
        let total = probed.get(*name).copied().unwrap_or(0);
        api_report.push_str(&format!("| {} | `{}` | {} | {} |\n", i + 1, name, count, total));
    }
    println!("{api_report}");

    let mut err_report = String::from("# IA-LIVE-PAGES — script errors by frequency\n\n| count | error |\n|---:|---|\n");
    let mut err_ranked: Vec<(&String, &usize)> = error_freq.iter().collect();
    err_ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (error, count) in err_ranked.iter().take(30) {
        err_report.push_str(&format!("| {} | {} |\n", count, error.replace('|', "\\|")));
    }

    let mut warn_report = String::from("# IA-LIVE-PAGES — renderer/resource warnings by frequency\n\n| count | warning kind |\n|---:|---|\n");
    let mut warn_ranked: Vec<(&String, &usize)> = warn_freq.iter().collect();
    warn_ranked.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (warning, count) in warn_ranked.iter().take(30) {
        warn_report.push_str(&format!("| {} | {} |\n", count, warning.replace('|', "\\|")));
    }
    println!("{warn_report}");

    let evidence_dir = std::env::var_os("AMAZING_LIVE_PAGES_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/live_pages/evidence"));
    let _ = std::fs::create_dir_all(&evidence_dir);
    let header = format!(
        "# IA-LIVE-PAGES run — {} urls\n\n# Fetched live through amazing-net (TLS). Two consecutive\n# runs are not byte-compared: the public web changes.\n\n",
        rows.len()
    );
    let _ = std::fs::write(evidence_dir.join("run.tsv"), format!("{header}{table}"));
    let _ = std::fs::write(evidence_dir.join("top-missing-apis.md"), api_report);
    let _ = std::fs::write(evidence_dir.join("errors.md"), err_report);
    let _ = std::fs::write(evidence_dir.join("warnings.md"), warn_report);
}
