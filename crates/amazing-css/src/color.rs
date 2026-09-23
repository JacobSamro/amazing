//! Compatibility copy of the existing paint color parser for CSS-1.
//! Kept in CSS so computed values never depend on a downstream crate.
//! CSS color value parsing for the paint phase: `<hex-color>`,
//! `rgb()` / `rgba()` (legacy comma and modern space syntax), and a table
//! of common named colors, all case-insensitive and panic-free.
//!
//! # Implemented
//!
//! - `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (digits case-insensitive,
//!   short forms expanded by digit-doubling per css-color-4 §4.4)
//! - `rgb()` and its exact alias `rgba()`, both syntaxes:
//!   - legacy comma form: `rgb(255, 0, 0)`, `rgba(255, 0, 0, 0.5)`
//!   - modern space form: `rgb(255 0 0)`, `rgb(255 0 0 / 0.5)`, with the
//!     solidus either space-separated or glued to a channel/alpha token
//! - channels as numbers or percentages (`50%`), out-of-range values
//!   clamped (`300` → 255, `-20` → 0), alpha clamped to 0..=1
//! - ~50 common named colors including `transparent` ([0,0,0,0])
//!
//! # Deferred / deviations
//!
//! - `currentcolor` intentionally does NOT parse here — it is a keyword
//!   that resolves against another computed property, so the caller
//!   (the paint consumer) resolves it against the element's `color`.
//! - `hsl()`/`hwb()`/`lab()`/`color()` and system colors are unsupported.
//! - Float channel precision is f32; rounding is round-half-away-from-zero
//!   after clamping, which is deterministic but not spec-normative.

/// Parses one CSS color into RGBA8 bytes. Returns `None` for anything it
/// does not understand; never panics on arbitrary (even non-UTF8-boundary-
/// friendly slicing is avoided entirely) input.
pub fn parse_css_color(input: &str) -> Option<[u8; 4]> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    // Named keywords first (case-insensitive).
    if let Some(c) = lookup_named(&s.to_ascii_lowercase()) {
        return Some(c);
    }
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let (name_raw, inner) = split_function(s)?;
    match name_raw.to_ascii_lowercase().as_str() {
        "rgb" | "rgba" => parse_rgb_args(inner),
        "hsl" | "hsla" => advanced::hsl(inner),
        "hwb" => advanced::hwb(inner),
        "color" => advanced::predefined_rgb(inner),
        "lab" => advanced::lab(inner),
        "lch" => advanced::lch(inner),
        "oklab" => advanced::oklab(inner),
        "oklch" => advanced::oklch(inner),
        _ => None,
    }
}

#[path = "color_advanced.rs"]
mod advanced;

// ---------------------------------------------------------------------------
// hex forms
// ---------------------------------------------------------------------------

fn parse_hex(h: &str) -> Option<[u8; 4]> {
    if !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    fn v(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            _ => b.to_ascii_lowercase() - b'a' + 10,
        }
    }
    match h.len() {
        // Short forms expand each digit d to d*16 + d (== d * 17).
        3 => {
            let d: Vec<u8> = h.bytes().map(v).collect();
            Some([d[0] * 17, d[1] * 17, d[2] * 17, 255])
        }
        4 => {
            let d: Vec<u8> = h.bytes().map(v).collect();
            Some([d[0] * 17, d[1] * 17, d[2] * 17, d[3] * 17])
        }
        6 | 8 => {
            let b: Vec<u8> = h
                .as_bytes()
                .chunks(2)
                .map(|p| v(p[0]) * 16 + v(p[1]))
                .collect();
            if b.len() == 3 {
                Some([b[0], b[1], b[2], 255])
            } else {
                Some([b[0], b[1], b[2], b[3]])
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// functional forms
// ---------------------------------------------------------------------------

/// Splits `name(inner)` into `(name, inner)`; requires the name to be
/// ASCII alphanumeric/hyphen and nothing but whitespace after `)`.
/// All byte offsets come from scanning ASCII punctuation of a `&str`, so
/// every slice below lands on a char boundary.
fn split_function(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    if s.as_bytes()[open] != b'(' {
        return None;
    }
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    let name = s[..open].trim();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    if !s[close + 1..].trim().is_empty() {
        return None;
    }
    Some((name, &s[open + 1..close]))
}

/// Parses the argument list shared by both syntaxes.
fn parse_rgb_args(inner: &str) -> Option<[u8; 4]> {
    let inner = inner.trim();
    if inner.contains(',') {
        // Legacy comma syntax; the solidus never appears in this form.
        if inner.contains('/') {
            return None;
        }
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let (r, g, b) = (
            parse_channel(parts.first()?),
            parse_channel(parts.get(1)?),
            parse_channel(parts.get(2)?),
        );
        let a = match parts.len() {
            3 => 255,
            4 => parse_alpha(parts.get(3)?)?,
            _ => return None,
        };
        Some([clamp_byte(r?), clamp_byte(g?), clamp_byte(b?), a])
    } else {
        // Modern space syntax. Re-tokenize so `255 0 0/0.5`, `255 0 0 /0.5`
        // and `255 0 0 / 0.5` all normalize to [c, c, c, "/", a].
        let joined = inner.split_ascii_whitespace().collect::<Vec<_>>().join(" ");
        let joined = joined.replace('/', " / ");
        let t: Vec<&str> = joined.split_ascii_whitespace().collect();
        match t.as_slice() {
            [r, g, b] => Some([
                clamp_byte(parse_channel(r)?),
                clamp_byte(parse_channel(g)?),
                clamp_byte(parse_channel(b)?),
                255,
            ]),
            [r, g, b, slash, a] if *slash == "/" => Some([
                clamp_byte(parse_channel(r)?),
                clamp_byte(parse_channel(g)?),
                clamp_byte(parse_channel(b)?),
                parse_alpha(a)?,
            ]),
            _ => None,
        }
    }
}

/// One RGB channel as a raw 0..=255-scale f32 (percentages rescaled).
/// Alphabetic tokens (`nan`, `inf`) are rejected; decimal values that
/// overflow f32 to ±infinity survive and clamp in [`clamp_byte`].
fn parse_channel(tok: &str) -> Option<f32> {
    let val = if let Some(p) = tok.strip_suffix('%') {
        p.trim().parse::<f32>().ok()? * 255.0 / 100.0
    } else {
        // Only digits, sign/decimal point, and exponent markers are legal
        // here; this rejects the literals "nan"/"inf"/"infinity" while
        // letting values like 1e40 overflow-parse and clamp later.
        if !tok
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E'))
        {
            return None;
        }
        tok.parse::<f32>().ok()?
    };
    if val.is_finite() || val.is_infinite() {
        Some(val)
    } else {
        None
    }
}

/// Alpha as number (0..=1) or percentage; `None` for unparseable or
/// non-finite input (the whole color is then rejected, matching how
/// channels behave).
fn parse_alpha(tok: &str) -> Option<u8> {
    let frac = match tok.strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok().map(|v| v / 100.0),
        None => tok.parse::<f32>().ok(),
    };
    match frac {
        Some(a) if a.is_finite() => Some(((a.clamp(0.0, 1.0)) * 255.0).round() as u8),
        _ => None,
    }
}

fn clamp_byte(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    v.round().clamp(0.0, 255.0) as u8
}

// ---------------------------------------------------------------------------
// named colors
// ---------------------------------------------------------------------------

/// Common CSS named colors (subset, aliases included). `transparent` maps
/// to fully transparent black per css-color-4.
const NAMED: &[(&str, [u8; 4])] = &[
    ("aliceblue", [240, 248, 255, 255]),
    ("antiquewhite", [250, 235, 215, 255]),
    ("aqua", [0, 255, 255, 255]),
    ("aquamarine", [127, 255, 212, 255]),
    ("azure", [240, 255, 255, 255]),
    ("beige", [245, 245, 220, 255]),
    ("black", [0, 0, 0, 255]),
    ("blue", [0, 0, 255, 255]),
    ("brown", [165, 42, 42, 255]),
    ("chocolate", [210, 105, 30, 255]),
    ("coral", [255, 127, 80, 255]),
    ("crimson", [220, 20, 60, 255]),
    ("cyan", [0, 255, 255, 255]),
    ("darkgray", [169, 169, 169, 255]),
    ("darkgrey", [169, 169, 169, 255]),
    ("dimgray", [105, 105, 105, 255]),
    ("dimgrey", [105, 105, 105, 255]),
    ("fuchsia", [255, 0, 255, 255]),
    ("gold", [255, 215, 0, 255]),
    ("goldenrod", [218, 165, 32, 255]),
    ("gray", [128, 128, 128, 255]),
    ("green", [0, 128, 0, 255]),
    ("greenyellow", [173, 255, 47, 255]),
    ("grey", [128, 128, 128, 255]),
    ("honeydew", [240, 255, 240, 255]),
    ("hotpink", [255, 105, 180, 255]),
    ("indigo", [75, 0, 130, 255]),
    ("ivory", [255, 255, 240, 255]),
    ("khaki", [240, 230, 140, 255]),
    ("lavender", [230, 230, 250, 255]),
    ("lime", [0, 255, 0, 255]),
    ("magenta", [255, 0, 255, 255]),
    ("maroon", [128, 0, 0, 255]),
    ("navy", [0, 0, 128, 255]),
    ("olive", [128, 128, 0, 255]),
    ("orange", [255, 165, 0, 255]),
    ("orchid", [218, 112, 214, 255]),
    ("pink", [255, 192, 203, 255]),
    ("plum", [221, 160, 221, 255]),
    ("purple", [128, 0, 128, 255]),
    ("rebeccapurple", [102, 51, 153, 255]),
    ("red", [255, 0, 0, 255]),
    ("salmon", [250, 128, 114, 255]),
    ("seagreen", [46, 139, 87, 255]),
    ("silver", [192, 192, 192, 255]),
    ("skyblue", [135, 206, 235, 255]),
    ("slategray", [112, 128, 144, 255]),
    ("snow", [255, 250, 250, 255]),
    ("steelblue", [70, 130, 180, 255]),
    ("tan", [210, 180, 140, 255]),
    ("teal", [0, 128, 128, 255]),
    ("thistle", [216, 191, 216, 255]),
    ("tomato", [255, 99, 71, 255]),
    ("transparent", [0, 0, 0, 0]),
    ("turquoise", [64, 224, 208, 255]),
    ("violet", [238, 130, 238, 255]),
    ("wheat", [245, 222, 179, 255]),
    ("white", [255, 255, 255, 255]),
    ("whitesmoke", [245, 245, 245, 255]),
    ("yellow", [255, 255, 0, 255]),
];

fn lookup_named(lower: &str) -> Option<[u8; 4]> {
    // Deterministic platform palette for the currently supported system
    // colors. Deprecated aliases share the corresponding modern role.
    match lower {
        "buttonface" | "threedface" => return Some([240, 240, 240, 255]),
        "buttonborder" | "threedlightshadow" => return Some([128, 128, 128, 255]),
        _ => {},
    }
    NAMED.iter().find(|(n, _)| *n == lower).map(|(_, c)| *c)
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_three_digit_expands_by_doubling() {
        assert_eq!(parse_css_color("#f00"), Some([255, 0, 0, 255]));
        assert_eq!(parse_css_color("#0f8"), Some([0, 255, 136, 255]));
    }

    #[test]
    fn hex_six_digit_exact() {
        assert_eq!(parse_css_color("#000000"), Some([0, 0, 0, 255]));
        assert_eq!(parse_css_color("#ffffff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_css_color("#1a2B3c"), Some([26, 43, 60, 255]));
    }

    #[test]
    fn hex_eight_digit_carries_alpha() {
        assert_eq!(parse_css_color("#ff000080"), Some([255, 0, 0, 128]));
        assert_eq!(parse_css_color("#12345678"), Some([18, 52, 86, 120]));
    }

    #[test]
    fn hex_four_digit_short_alpha() {
        assert_eq!(parse_css_color("#f00c"), Some([255, 0, 0, 204]));
    }

    #[test]
    fn hex_rejects_bad_lengths_and_digits() {
        for bad in ["#", "#12", "#12345", "#1234567", "#ggg", "#ff00zz"] {
            assert_eq!(parse_css_color(bad), None, "expected None for {bad}");
        }
    }

    #[test]
    fn named_colors_case_insensitive() {
        assert_eq!(parse_css_color("WHITE"), Some([255, 255, 255, 255]));
        assert_eq!(parse_css_color("Black"), Some([0, 0, 0, 255]));
        assert_eq!(parse_css_color("ReD"), Some([255, 0, 0, 255]));
        assert_eq!(parse_css_color("  gReEn "), Some([0, 128, 0, 255]));
        assert_eq!(parse_css_color("rebeccapurple"), Some([102, 51, 153, 255]));
    }

    #[test]
    fn transparent_is_zero_alpha_black() {
        assert_eq!(parse_css_color("transparent"), Some([0, 0, 0, 0]));
    }

    #[test]
    fn currentcolor_is_caller_resolved_not_parsed_here() {
        assert_eq!(parse_css_color("currentcolor"), None);
        assert_eq!(parse_css_color("CURRENTCOLOR"), None);
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(parse_css_color("notacolor"), None);
        assert_eq!(parse_css_color("red extra"), None);
        assert_eq!(parse_css_color(""), None);
        assert_eq!(parse_css_color("   "), None);
    }

    #[test]
    fn rgb_legacy_comma_syntax() {
        assert_eq!(parse_css_color("rgb(255,0,0)"), Some([255, 0, 0, 255]));
        assert_eq!(
            parse_css_color("rgb( 10 , 20 , 30 )"),
            Some([10, 20, 30, 255])
        );
        assert_eq!(parse_css_color("rgba(1,2,3,0.5)"), Some([1, 2, 3, 128]));
        assert_eq!(
            parse_css_color("RGB(0, 128, 255)"),
            Some([0, 128, 255, 255])
        );
    }

    #[test]
    fn rgb_channels_clamp_into_range() {
        assert_eq!(parse_css_color("rgb(300,-20,0)"), Some([255, 0, 0, 255]));
        assert_eq!(parse_css_color("rgba(0,0,9999,2)"), Some([0, 0, 255, 255]));
        assert_eq!(parse_css_color("rgba(0,0,0,-3)"), Some([0, 0, 0, 0]));
    }

    #[test]
    fn rgb_percent_channels_mixed_with_numbers() {
        assert_eq!(parse_css_color("rgb(100%,0%,0%)"), Some([255, 0, 0, 255]));
        // Mixed % and number units are invalid CSS but accepted
        // permissively here: each channel parses on its own scale.
        assert_eq!(
            parse_css_color("rgb(50%, 200, 0%)"),
            Some([128, 200, 0, 255])
        );
        assert_eq!(
            parse_css_color("rgba(100%,100%,100%,50%)"),
            Some([255, 255, 255, 128])
        );
    }

    #[test]
    fn rgb_modern_space_syntax() {
        assert_eq!(parse_css_color("rgb(255 0 0)"), Some([255, 0, 0, 255]));
        assert_eq!(
            parse_css_color("rgb(255 0 0 / 0.5)"),
            Some([255, 0, 0, 128])
        );
        assert_eq!(parse_css_color("rgb(1 2 3/0.25)"), Some([1, 2, 3, 64]));
        assert_eq!(parse_css_color("rgb(4 5 6 /100%)"), Some([4, 5, 6, 255]));
        assert_eq!(parse_css_color("rgba(0 0 0 / 1)"), Some([0, 0, 0, 255]));
    }

    #[test]
    fn rgb_rejects_malformed_arguments() {
        for bad in [
            "rgb()",
            "rgb(1,2)",
            "rgb(1,2,3,4,5)",
            "rgb(1;2;3)",
            "rgb(1 2)",
            "rgb(1 2 3 4)",
            "rgb(1,2,3/",
            "rgb(nan,0,0)",
            "rgb(inf,0,0)",
            "rgb(",
            "rgb(1,2,3))extra",
            "hsl(1,2,3)",
            "",
            // Unparseable alpha rejects the whole color, like channels do.
            "rgba(1,2,3,banana)",
            "rgba(1,2,3,)",
            "rgb(1 2 3 / nan)",
        ] {
            assert_eq!(parse_css_color(bad), None, "expected None for {bad}");
        }
        // A stray closing paren inside the args fails channel parsing too.
        assert_eq!(parse_css_color("rgb(1,2,)3)"), None);
    }

    #[test]
    fn huge_finite_channels_clamp_instead_of_overflowing() {
        assert_eq!(parse_css_color("rgb(1e40,0,0)"), Some([255, 0, 0, 255]));
        assert_eq!(parse_css_color("rgb(-1e40 0% 50%)"), Some([0, 0, 128, 255]));
    }

    #[test]
    fn parser_never_panics_on_hostile_input() {
        // Multibyte characters must not break byte-offset slicing; odd
        // punctuation exercises every early-return path.
        let hostile = [
            "é",
            "🦀",
            "rgb(🦀)",
            "(())",
            "(((,",
            ")))",
            "#é",
            "%",
            "/",
            "rgb(1,2,3",
            "\u{0}",
            "rgb(\u{0}1,2,3)",
            "rgba(1,2,3,🦀)",
            "-rgb(1,2,3)",
            "rgb--(1,2,3)",
            "rgb(1,,3)",
            "rgb((1,2,3)",
        ];
        for h in hostile {
            let _ = parse_css_color(h); // must only return None, never panic
        }
    }
}
