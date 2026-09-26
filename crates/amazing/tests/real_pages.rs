//! IA-REAL-PAGES corpus harness.
//!
//! Runs every page in `tests/real-pages/pages` through the lenient headless
//! `render-page` driver path in a child process, so a crash or panic is
//! attributed to one page instead of aborting the suite. Reports per-page
//! status, script errors, likely missing-feature fallbacks and render time, and
//! asserts that no page crashes or panics.
//!
//! Set `AMAZING_REAL_PAGES_EVIDENCE` to a file path to persist the TSV (used for
//! the ticket report). Otherwise the table is only printed. `AMAZING_REAL_PAGES_ONLY`
//! filters to pages whose file name contains the value.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/real-pages")
}

/// Heuristic split of a collected error into "likely missing feature" and
/// "likely page bug". The raw text is always reported for triage.
fn looks_like_missing_feature(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "not supported",
        "unsupported",
        "not implemented",
        "is not a function",
        "is not defined",
        "not a constructor",
        "cannot read propert",
        "cannot set propert",
        "non-element",
        "read-only property",
        "no such",
        "missing",
        "unknown",
        "not callable",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

fn sanitize(page: &Path) -> String {
    page.file_stem()
        .map(|stem| stem.to_string_lossy().replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_"))
        .unwrap_or_else(|| "page".into())
}

struct Row {
    page: String,
    status: String,
    errors: Vec<String>,
    render_ms: u64,
    total_ms: u64,
    exit: i32,
    timed_out: bool,
    wrote_image: bool,
    nonwhite: usize,
}

fn run_page(binary: &str, page: &Path, out_dir: &Path, timeout: Duration) -> Row {
    let out = out_dir.join(format!("{}.bmp", sanitize(page)));
    let started = Instant::now();
    let mut child = Command::new(binary)
        .arg("render-page")
        .arg(page)
        .arg(&out)
        .arg("1024")
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
            None => std::thread::sleep(Duration::from_millis(10)),
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
        }
    }
    if (exit != 0 || status == "no_report") && !stderr.trim().is_empty() {
        for line in stderr.lines().take(4) {
            if !line.trim().is_empty() {
                errors.push(format!("stderr: {line}"));
            }
        }
    }
    if timed_out {
        errors.push(format!("timeout after {}s", timeout.as_secs()));
    }
    Row {
        page: page.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        status,
        errors,
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
fn real_pages_corpus_renders_without_crashes() {
    let root = corpus_root();
    let pages_dir = root.join("pages");
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&pages_dir)
        .unwrap_or_else(|error| panic!("corpus missing at {}: {error}", pages_dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "html"))
        .collect();
    pages.sort();
    if let Ok(only) = std::env::var("AMAZING_REAL_PAGES_ONLY") {
        pages.retain(|path| path.file_name().is_some_and(|name| name.to_string_lossy().contains(&only)));
    }
    assert!(pages.len() >= 50, "expected the ~50-page corpus, found {}", pages.len());

    let out_dir = std::env::temp_dir().join(format!("amazing-real-pages-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    // Pooled sandboxes share the target dir across lanes, so another lane's
    // `cargo build -p amazing` can replace CARGO_BIN_EXE_amazing mid-run. Run a
    // private copy of the binary so the whole corpus uses the built code.
    let source_binary = env!("CARGO_BIN_EXE_amazing");
    let stable_binary = out_dir.join(if cfg!(windows) { "amazing-real.exe" } else { "amazing-real" });
    std::fs::copy(source_binary, &stable_binary)
        .unwrap_or_else(|error| panic!("copy {source_binary}: {error}"));
    let binary = stable_binary.to_str().expect("binary path is UTF-8");
    let timeout = Duration::from_secs(30);

    let mut rows = Vec::new();
    for page in &pages {
        rows.push(run_page(binary, page, &out_dir, timeout));
    }

    let mut fallback_pages = 0;
    let mut errored_pages = 0;
    let mut crashes = 0;
    let mut slowest = (0u64, String::new());
    let mut table = String::from("page\tstatus\terrors\tfallbacks\trender_ms\ttotal_ms\tnonwhite\texit\tfirst_error\n");
    for row in &rows {
        let fallbacks = row.errors.iter().filter(|error| looks_like_missing_feature(error)).count();
        if fallbacks > 0 {
            fallback_pages += 1;
        }
        if !row.errors.is_empty() {
            errored_pages += 1;
        }
        if row.exit != 0 || row.timed_out || row.status == "no_report" {
            crashes += 1;
        }
        if row.total_ms > slowest.0 {
            slowest = (row.total_ms, row.page.clone());
        }
        table.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.page,
            row.status,
            row.errors.len(),
            fallbacks,
            row.render_ms,
            row.total_ms,
            row.nonwhite,
            row.exit,
            row.errors.first().map(String::as_str).unwrap_or(""),
        ));
    }
    println!("{table}");
    println!(
        "real-pages: {} pages, {} errors, {} fallback pages, {} crashes, slowest {} ({} ms)",
        rows.len(), errored_pages, fallback_pages, crashes, slowest.1, slowest.0
    );
    let mut distinct: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for row in &rows {
        for error in &row.errors {
            *distinct.entry(error.as_str()).or_default() += 1;
        }
    }
    println!("distinct errors:");
    for (error, count) in &distinct {
        println!("  {count}x {error}");
    }

    if let Ok(path) = std::env::var("AMAZING_REAL_PAGES_EVIDENCE") {
        if let Some(parent) = Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut evidence = format!("# IA-REAL-PAGES run — {} pages\n\n", rows.len());
        evidence.push_str(&table);
        let _ = std::fs::write(path, evidence);
    }
    let _ = std::fs::remove_dir_all(&out_dir);

    let broken: Vec<&Row> = rows.iter().filter(|row| row.exit != 0 || row.timed_out).collect();
    assert!(
        broken.is_empty(),
        "pages crashed, panicked or timed out: {:?}",
        broken.iter().map(|row| (&row.page, row.exit, row.status.as_str())).collect::<Vec<_>>()
    );
    let bad_paint: Vec<&Row> = rows.iter()
        .filter(|row| row.status == "ok" && !row.wrote_image)
        .collect();
    assert!(bad_paint.is_empty(), "pages reported ok without an image: {:?}",
        bad_paint.iter().map(|row| &row.page).collect::<Vec<_>>());
    let blank: Vec<&Row> = rows.iter()
        .filter(|row| row.status == "ok" && row.nonwhite == 0)
        .collect();
    assert!(blank.is_empty(), "pages rendered blank: {:?}",
        blank.iter().map(|row| &row.page).collect::<Vec<_>>());
}
