//! Actual HTML/CSS -> resource decode -> replaced layout -> PagePaint controls.
use std::{fs, path::PathBuf, process::Command, time::{SystemTime, UNIX_EPOCH}};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("amazing-image-presentation-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn render(&self, css: &str, natural: (usize, usize)) -> amazing_media::Image {
        self.render_source(css, natural, "src='red.ppm'")
    }
    fn render_source(&self, css: &str, natural: (usize, usize), source: &str) -> amazing_media::Image {
        let mut ppm = format!("P6\n{} {}\n255\n", natural.0, natural.1).into_bytes();
        for _ in 0..natural.0 * natural.1 { ppm.extend_from_slice(&[255, 0, 0]); }
        fs::write(self.0.join("red.ppm"), ppm).unwrap();
        // Content is exactly x=6..14,y=6..14. Padding, border and margin
        // independently expose accidentally fitting to the outer replaced box.
        fs::write(self.0.join("page.html"), format!("<style>html,body{{margin:0;background:white;min-height:24px}}img{{display:block;width:8px;height:8px;margin:3px;border:1px solid black;padding:2px;background:lime;{css}}}</style><img {source}>")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_amazing"))
            .args(["render", self.0.join("page.html").to_str().unwrap(), self.0.join("result.bmp").to_str().unwrap(), "24"])
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        amazing_media::decode_image(&fs::read(self.0.join("result.bmp")).unwrap()).unwrap()
    }
}
impl Drop for Fixture { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
fn assert_red_bounds(image: &amazing_media::Image, bounds: (usize, usize, usize, usize), context: &str) {
    assert!(image.width >= 18 && image.height >= 18, "{context}");
    for y in 0..18 { for x in 0..18 {
        let offset = (y * image.width as usize + x) * 4;
        let red = image.pixels[offset..offset+4] == [255, 0, 0, 255];
        let expected = x >= bounds.0 && x < bounds.2 && y >= bounds.1 && y < bounds.3;
        assert_eq!(red, expected, "{context}: pixel ({x},{y})");
    }}
}
#[test]
fn installed_every_fit_and_both_scale_down_orders_use_the_content_box() {
    let fixture = Fixture::new();
    for (fit, natural, bounds) in [
        ("fill", (4, 2), (6, 6, 14, 14)),
        ("contain", (4, 2), (6, 8, 14, 12)),
        ("cover", (4, 2), (6, 6, 14, 14)),
        ("none", (4, 2), (8, 9, 12, 11)),
        ("scale-down", (4, 2), (8, 9, 12, 11)),
        ("contain scale-down", (4, 2), (8, 9, 12, 11)),
        ("scale-down contain", (4, 2), (8, 9, 12, 11)),
        ("cover scale-down", (4, 2), (8, 9, 12, 11)),
        ("scale-down cover", (4, 2), (8, 9, 12, 11)),
        ("cover scale-down", (8, 16), (6, 6, 14, 14)),
        ("contain scale-down", (8, 16), (8, 6, 12, 14)),
        ("SCALE-DOWN /**/ co\\76 er", (8, 16), (6, 6, 14, 14)),
    ] {
        assert_red_bounds(&fixture.render(&format!("object-fit:{fit}"), natural), bounds, fit);
    }
}
#[test]
fn installed_positions_substitution_and_cascade_reach_actual_resource_paint() {
    let fixture = Fixture::new();
    for (position, bounds) in [
        ("left top", (6, 6, 10, 8)),
        ("right bottom", (10, 12, 14, 14)),
        ("right 1px bottom 2px", (9, 10, 13, 12)),
        ("25% 50%", (7, 9, 11, 11)),
        ("calc(50% + 1px) calc(50% - 1px)", (9, 8, 13, 10)),
    ] {
        let css = format!("--fit:scale-down cover;--pos:{position};object-fit:var(--fit);object-position:var(--pos)");
        assert_red_bounds(&fixture.render(&css, (4, 2)), bounds, position);
    }
    for css in [
        "object-fit:none!important;object-fit:fill",
        "object-fit:none;object-fit:cover contain",
        "object-fit:none;object-fit:cover scale-down scale-down",
    ] {
        assert_red_bounds(&fixture.render(css, (4, 2)), (8, 9, 12, 11), css);
    }
    assert_red_bounds(&fixture.render("object-fit:cover;object-fit:var(--missing)", (4, 2)), (6, 6, 14, 14), "invalid substitution resets to fill");
}

#[test]
fn installed_intrinsic_ratio_and_border_box_constraints_precede_fitting() {
    let fixture = Fixture::new();
    for (css, bounds) in [
        ("width:auto;height:auto;object-fit:cover", (6, 6, 10, 8)),
        ("width:8px;height:auto;object-fit:contain", (6, 6, 14, 10)),
        ("width:auto;height:4px;object-fit:cover", (6, 6, 14, 10)),
        ("box-sizing:border-box;object-fit:cover", (6, 6, 8, 8)),
        ("width:auto;height:8px;max-height:4px;object-fit:cover", (6, 6, 14, 10)),
        ("width:auto;height:2px;min-height:4px;object-fit:cover", (6, 6, 14, 10)),
        ("width:auto;height:2px;min-height:4px;max-height:2px;object-fit:cover", (6, 6, 14, 10)),
        ("width:auto;height:4px;max-width:4px;object-fit:cover", (6, 6, 10, 10)),
        ("width:auto;height:4px;min-width:12px;object-fit:cover", (6, 6, 18, 10)),
        ("width:auto;height:10px;box-sizing:border-box;object-fit:cover", (6, 6, 14, 10)),
        ("width:auto;height:.25em;font-size:16px;object-fit:cover", (6, 6, 14, 10)),
        ("width:auto;height:50%;object-fit:cover", (6, 6, 10, 8)),
        ("width:auto;height:0;object-fit:cover", (6, 6, 6, 6)),
    ] {
        assert_red_bounds(&fixture.render(css, (4, 2)), bounds, css);
    }
}

#[test]
fn installed_source_density_consistently_corrects_layout_and_natural_fit() {
    let fixture = Fixture::new();
    // A single selected descriptor makes source choice deterministic at DPR1.
    for (source, natural, bounds) in [
        ("srcset='red.ppm 1x'", (4, 2), (8, 9, 12, 11)),
        ("srcset='red.ppm 1.5x'", (6, 3), (8, 9, 12, 11)),
        ("srcset='red.ppm 2x'", (8, 4), (8, 9, 12, 11)),
        ("srcset='red.ppm 8w' sizes='4px'", (8, 4), (8, 9, 12, 11)),
    ] {
        for fit in ["none", "scale-down", "contain scale-down", "scale-down contain", "cover scale-down", "scale-down cover"] {
            let css = format!("object-fit:{fit}");
            assert_red_bounds(&fixture.render_source(&css, natural, source), bounds, &format!("{source} {fit}"));
        }
        assert_red_bounds(&fixture.render_source("width:auto;height:auto;object-fit:none", natural, source), (6, 6, 10, 8), source);
        for css in [
            "width:auto;height:4px;object-fit:cover",
            "width:auto;height:8px;max-height:4px;object-fit:cover",
            "width:auto;height:10px;box-sizing:border-box;object-fit:cover",
        ] {
            assert_red_bounds(&fixture.render_source(css, natural, source), (6, 6, 14, 10), &format!("{source} {css}"));
        }
        for display in ["inline", "inline-block"] {
            for fit in ["none", "scale-down", "cover scale-down"] {
                let css = format!("display:{display};object-fit:{fit}");
                let image = fixture.render_source(&css, natural, source);
                let red = image.pixels.chunks_exact(4).filter(|p| *p == [255, 0, 0, 255]).count();
                assert_eq!(red, 8, "{source} {display} {fit}: natural CSS4x2 object");
            }
        }
    }
}
