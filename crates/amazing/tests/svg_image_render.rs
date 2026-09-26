//! RC-28 regression: `<img src="*.svg">` must decode, size and paint.
//!
//! `tests/real-pages/pages/docs-index.html` has a header logo and six card
//! thumbnails, all `<img src="*.svg">` referencing SVGs with a solid-color
//! `<rect>` background plus a `<text>` label. Before RC-28, `amazing-svg`'s
//! byte decoder rejected the whole document on the first unrecognized
//! element (`text`), so every one of those images painted nothing. This
//! renders the real fixture page and asserts each image's known `<rect>`
//! fill color actually appears on the raster in a solid run, proving the
//! image decoded, was sized, and was painted — not just that the page
//! rendered "some" non-white pixels.
use std::path::Path;
use std::process::Command;

fn corpus_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/real-pages"))
}

/// Longest run of pixels matching `target` (exact RGB, ignoring alpha) in any
/// single row. A real filled rect produces a long run; anti-aliased text or
/// thin strokes of a similar hue do not.
fn max_horizontal_run(image: &amazing_media::Image, target: [u8; 3]) -> usize {
    let mut best = 0usize;
    for y in 0..image.height as usize {
        let mut run = 0usize;
        for x in 0..image.width as usize {
            let at = (y * image.width as usize + x) * 4;
            let px = &image.pixels[at..at + 4];
            if [px[0], px[1], px[2]] == target {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
    }
    best
}

#[test]
fn docs_index_svg_images_paint_their_rect_backgrounds() {
    let root = corpus_root();
    let page = root.join("pages/docs-index.html");
    assert!(page.is_file(), "fixture missing: {}", page.display());

    let out_dir = std::env::temp_dir().join(format!("amazing-svg-image-render-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let out_bmp = out_dir.join("docs-index.bmp");

    let source_binary = env!("CARGO_BIN_EXE_amazing");
    let stable_binary = out_dir.join(if cfg!(windows) { "amazing-real.exe" } else { "amazing-real" });
    std::fs::copy(source_binary, &stable_binary)
        .unwrap_or_else(|error| panic!("copy {source_binary}: {error}"));

    let output = Command::new(&stable_binary)
        .arg("render-page")
        .arg(&page)
        .arg(&out_bmp)
        .arg("1024")
        .arg("900")
        .output()
        .expect("spawn render-page");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "render-page exited with {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status.code()
    );
    assert!(
        stdout.lines().any(|line| line.starts_with("SUMMARY\tok\t")),
        "render-page did not report ok\nstdout: {stdout}\nstderr: {stderr}"
    );
    // The bug this guards against surfaced as `amazing render: image "...": UnsupportedSvg`.
    assert!(
        !stderr.contains("UnsupportedSvg"),
        "an SVG image failed to decode\nstderr: {stderr}"
    );

    let bytes = std::fs::read(&out_bmp).unwrap_or_else(|error| panic!("read {}: {error}", out_bmp.display()));
    let image = amazing_media::decode_image(&bytes).expect("decode rendered BMP");
    let _ = std::fs::remove_dir_all(&out_dir);

    // logo.svg: <rect width="120" height="32" fill="#0a58ca"/>. `header.site
    // img` is scaled to height:28px, so allow a small tolerance for the
    // resample instead of requiring an exact channel match.
    let logo_run = (0..image.height as usize)
        .flat_map(|y| (0..image.width as usize).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let at = (y * image.width as usize + x) * 4;
            let px = &image.pixels[at..at + 4];
            (px[0] as i32 - 10).abs() <= 12 && (px[1] as i32 - 88).abs() <= 12 && (px[2] as i32 - 202).abs() <= 12
        })
        .count();
    assert!(
        logo_run >= 200,
        "expected the logo's #0a58ca rect to paint at least 200 matching pixels, found {logo_run}"
    );

    // thumb-N.svg: <rect width="160" height="120" fill="#2f6f4e"/>, painted at
    // intrinsic size (no CSS height override), so exact fill color and a long
    // solid run are expected.
    let thumb_run = max_horizontal_run(&image, [47, 111, 78]);
    assert!(
        thumb_run >= 60,
        "expected a thumbnail's #2f6f4e rect to paint a run of at least 60 pixels, longest run found {thumb_run}"
    );
}
