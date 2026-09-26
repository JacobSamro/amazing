use std::process::Command;

fn assert_oracle_result(name: &str, html_source: &str, expect_pixel_parity: bool) {
    let dir = std::env::temp_dir().join(format!("amazing-invalidation-oracle-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let html = dir.join("page.html");
    let direct_image = dir.join("direct.ppm");
    let oracle_image = dir.join("oracle.ppm");
    std::fs::write(&html, html_source).unwrap();
    let run = |oracle: bool, output: &std::path::Path| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_amazing"));
        command.args(["render-wpt", html.to_str().unwrap(), output.to_str().unwrap(), "512", "512"]);
        command.env("AMAZING_STORAGE_DIR", dir.join("storage"));
        if oracle { command.env("AMAZING_INVALIDATION_ORACLE", "1"); }
        command.output().unwrap()
    };
    let direct = run(false, &direct_image);
    let oracle = run(true, &oracle_image);
    assert!(direct.status.success(), "{}", String::from_utf8_lossy(&direct.stderr));
    assert!(oracle.status.success(), "{}", String::from_utf8_lossy(&oracle.stderr));
    let direct_pixels = std::fs::read(&direct_image).unwrap();
    let oracle_pixels = std::fs::read(&oracle_image).unwrap();
    let diagnostic = String::from_utf8_lossy(&oracle.stderr);
    if expect_pixel_parity {
        assert_eq!(direct_pixels, oracle_pixels);
        assert!(diagnostic.contains("compositor/full pixel parity"), "{diagnostic}");
    } else {
        let header_end = |image: &[u8]| image.iter().enumerate()
            .filter(|(_, byte)| **byte == b'\n').nth(2).unwrap().0 + 1;
        let direct_start = header_end(&direct_pixels);
        let oracle_start = header_end(&oracle_pixels);
        assert_eq!(&direct_pixels[..direct_start], &oracle_pixels[..oracle_start]);
        let direct_rgb = &direct_pixels[direct_start..];
        let oracle_rgb = &oracle_pixels[oracle_start..];
        assert_eq!(direct_rgb.len(), oracle_rgb.len());
        let mut rounding = 0;
        for (direct, candidate) in direct_rgb.chunks_exact(3).zip(oracle_rgb.chunks_exact(3)) {
            if direct != candidate {
                assert!(direct.iter().zip(candidate).all(|(a, b)| a.abs_diff(*b) <= 1));
                rounding += 1;
            }
        }
        assert!(rounding > 0 && diagnostic.contains(&format!("layer-rounding pixels={rounding}")),
            "rounding={rounding} {diagnostic}");
        assert!(!diagnostic.contains("parity loss:"), "{diagnostic}");
    }
    assert!(String::from_utf8_lossy(&oracle.stderr).contains("full_fallback=false"),
        "{}", String::from_utf8_lossy(&oracle.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn diagnostic_frame_matches_full_render_after_testrendered_mutation() {
    assert_oracle_result("reflow", "<!doctype html><style>body{margin:0}.changed{background:#008000;width:48px;height:32px}</style><body><div id=box></div><script>document.addEventListener('TestRendered',()=>document.querySelector('#box').setAttribute('class','changed'))</script>", true);
}

#[test]
fn diagnostic_frame_matches_full_render_for_effects_scroller() {
    assert_oracle_result("effects", "<!doctype html><style>body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:300px;height:180px;overflow:scroll;background:#ddd}.scroller.changed{width:280px}.moving{will-change:transform;background:blue;width:120px;height:80px}.fade{opacity:.5;background:green;width:120px;height:80px}</style><body><div class=fixed>Fixed</div><section class=scroller><div class=moving>Move</div><div class=fade>Fade</div><p>Overflow content</p><p>More overflow content</p></section><script>document.addEventListener('TestRendered',()=>document.querySelector('section').setAttribute('class','scroller changed'))</script>", !cfg!(windows));
}
