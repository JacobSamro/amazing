//! Transitional contract for CSS-2 (remove paint's duplicate color parser).
//! Compile paint's actual production parser as a test-only module, avoiding a
//! reversed runtime dependency or a workspace lockfile change. Any edit to that
//! source is exercised here; this is not a second copied test implementation.
#[path = "../../amazing-paint/src/color.rs"]
mod transitional_paint;

fn parity(input: &str) {
    assert_eq!(
        amazing_css::color::parse_css_color(input),
        transitional_paint::parse_css_color(input),
        "{input:?}"
    );
}

#[test]
fn canonical_css_parser_matches_transitional_paint_for_edges_and_generated_inputs() {
    for input in [
        "",
        " ",
        "currentcolor",
        "CURRENTCOLOR",
        "transparent",
        "red",
        "ReBeCcApUrPlE",
        "aliceblue",
        "crimson",
        "notacolor",
        "red extra",
        "#f00",
        "#0f8c",
        "#123456",
        "#12345678",
        "#12",
        "#12345",
        "#1234567",
        "#ggg",
        "#é",
        "🦀",
        "rgb(🦀)",
        "rgb(1 2 3/0.25)",
        "rgb(100%,0%,0%)",
        "rgb(50%,200,0%)",
        "rgb(1e40,0,0)",
        "rgb(-1e40 0% 50%)",
        "rgb(nan,0,0)",
        "rgb(inf,0,0)",
        "rgb(NaN%,0,0)",
        "rgb()",
        "rgb(1,2)",
        "rgb(1,2,3,4,5)",
        "rgb(1 2 3 4)",
        "rgb(1,2,3/)",
        "rgb(1,2,)3)",
        "rgb(1,2,3))extra",
        "rgba(1,2,3,)",
        "rgba(1,2,3,banana)",
        "rgb(1 2 3 / nan)",
        "rgb(1 2 3 / 1e40)",
        "hsl(1,2,3)",
        "rgb(\0 1,2,3)",
        "rgb(1,,3)",
        "(((,",
        ")))",
    ] {
        parity(input);
    }
    for n in 0..=255 {
        parity(&format!("#{n:02x}80ff"));
        parity(&format!("#{n:02x}80ff{n:02x}"));
        parity(&format!("rgba({n}, {}, {}, 0.5)", n - 128, n + 128));
        parity(&format!("rgb({n}% 0% 50% / {n}%)"));
    }
    // Deterministic malformed inputs exercise Unicode boundaries and recovery.
    let alphabet = [
        'r', 'g', 'b', '(', ')', ',', '/', '%', '#', '0', '9', '-', '.', ' ', 'é', '🦀',
    ];
    let mut state = 0x1234_5678u32;
    for _ in 0..2048 {
        let mut text = String::new();
        for _ in 0..16 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            text.push(alphabet[(state >> 28) as usize]);
        }
        parity(&text);
    }
}
