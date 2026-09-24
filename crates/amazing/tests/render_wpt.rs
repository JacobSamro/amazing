use std::fs;
use std::process::Command;

fn fixture_path(name: &str) -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("amazing-render-wpt-{}-{nonce}-{name}", std::process::id()))
}

#[test]
fn fixed_render_writes_the_exact_requested_viewport() {
    let html = fixture_path("fixed.html");
    let ppm = fixture_path("fixed.ppm");
    let storage = fixture_path("fixed-storage");
    fs::write(
        &html,
        "<!doctype html><style>body{margin:0;background:green}</style>",
    )
    .expect("write render fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args([
            "render-wpt",
            html.to_str().expect("UTF-8 fixture path"),
            ppm.to_str().expect("UTF-8 output path"),
            "37",
            "23",
        ])
        .env("AMAZING_STORAGE_DIR", &storage)
        .output()
        .expect("run fixed renderer");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&ppm).expect("read PPM");
    let header = b"P6\n37 23\n255\n";
    assert!(bytes.starts_with(header));
    assert_eq!(bytes.len(), header.len() + 37 * 23 * 3);
    let _ = fs::remove_file(html);
    let _ = fs::remove_file(ppm);
    let _ = fs::remove_dir_all(storage);
}

#[test]
fn fixed_render_rejects_unbounded_dimensions() {
    let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
        .args(["render-wpt", "missing.html", "missing.ppm", "0", "600"])
        .output()
        .expect("run fixed renderer");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("width must be"));
}

#[test]
fn wait_timeout_and_page_script_error_do_not_write_images() {
    for (name, source, exit, detail) in [
        (
            "wait-timeout",
            "<!doctype html><html class='reftest-wait'><body>pending</body></html>",
            3,
            "reftest-wait remained",
        ),
        (
            "script-error",
            "<!doctype html><script>throw new Error('page sentinel')</script>",
            1,
            "page sentinel",
        ),
    ] {
        let html = fixture_path(&format!("{name}.html"));
        let ppm = fixture_path(&format!("{name}.ppm"));
        let storage = fixture_path(&format!("{name}-storage"));
        let _ = fs::remove_file(&ppm);
        fs::write(&html, source).expect("write render fixture");
        let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
            .args([
                "render-wpt",
                html.to_str().expect("UTF-8 fixture path"),
                ppm.to_str().expect("UTF-8 output path"),
                "64",
                "48",
            ])
            .env("AMAZING_STORAGE_DIR", &storage)
            .output()
            .expect("run fixed renderer");
        assert_eq!(output.status.code(), Some(exit));
        assert!(String::from_utf8_lossy(&output.stderr).contains(detail));
        assert!(!ppm.exists());
        let _ = fs::remove_file(html);
        let _ = fs::remove_dir_all(storage);
    }
}
