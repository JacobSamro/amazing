    #[test]
    #[ignore = "requires AMAZING_WPT_ROOT at the pinned WPT revision"]
    fn pinned_wpt_sfnt_woff_and_woff2_decode_and_register() {
        let root = std::env::var("AMAZING_WPT_ROOT").expect("set AMAZING_WPT_ROOT to pinned WPT");
        for (relative, format) in [
            ("fonts/Ahem.ttf", FontFormat::OpenType),
            ("fonts/ahem-visible-zwnj.otf", FontFormat::OpenType),
            ("fonts/AD.woff", FontFormat::Woff),
            (
                "fonts/hasubi-mono/HasubiMono-Regular.woff2",
                FontFormat::Woff2,
            ),
            (
                "fonts/noto/NotoNaskhArabic-regular.woff2",
                FontFormat::Woff2,
            ),
            (
                "fonts/noto/NotoSansMongolian-regular.woff2",
                FontFormat::Woff2,
            ),
            (
                "fonts/noto/NotoSansNko-regular-webfont.woff2",
                FontFormat::Woff2,
            ),
        ] {
            let bytes = std::fs::read(std::path::Path::new(&root).join(relative)).unwrap();
            let decoded = decode_web_font_to_sfnt(format, &bytes)
                .unwrap_or_else(|error| panic!("decode and validate {relative}: {error:?}"));
            let mut provider = TextProvider::new();
            provider
                .register_web_font(WebFont {
                    family: relative.into(),
                    weight: 400,
                    stretch: 100,
                    slant: FontSlant::Normal,
                    format,
                    bytes,
                })
                .unwrap_or_else(|error| panic!("{relative}: {error:?}"));
            assert_eq!(provider.fonts[0].sfnt, decoded, "{relative}");
        }
    }

    /// TEXT-19: a real pinned WOFF2 truncated at every point up to its full
    /// length must always fail closed through `decode_web_font_to_sfnt` --
    /// never panic, never succeed on a short buffer. This is the exact shape
    /// of the live-pages corpus's `font-face "<family>": Io(Error{kind:
    /// UnexpectedEof, message: "failed to fill whole buffer"}) ` failures
    /// (`.agents/text/questions.md`): investigation traced that error to
    /// `amazing_net::NetError::Io`, raised by the network layer when a
    /// server closes the connection before delivering the bytes its own
    /// `Content-Length` promised (see `amazing-net/src/stream.rs`'s
    /// `short_content_length_body_is_an_error_not_a_truncation` test) --
    /// i.e. the decoder never receives a short buffer at all in that
    /// failure; a short-read that does reach it is exactly what this test
    /// covers, and it was already handled correctly before TEXT-19 (this
    /// test only makes that explicit and pins it against a real font).
    #[test]
    #[ignore = "requires AMAZING_WPT_ROOT at the pinned WPT revision"]
    fn pinned_wpt_woff2_fails_closed_at_every_truncation_cut() {
        let root = std::env::var("AMAZING_WPT_ROOT").expect("set AMAZING_WPT_ROOT to pinned WPT");
        let full = std::fs::read(
            std::path::Path::new(&root).join("fonts/noto/NotoNaskhArabic-regular.woff2"),
        )
        .unwrap();
        assert!(
            decode_web_font_to_sfnt(FontFormat::Woff2, &full).is_ok(),
            "the untruncated fixture must decode so the cut sweep below is meaningful"
        );
        let cuts: Vec<usize> = [0, 1, 4, 10, 50, 200, 1000]
            .into_iter()
            .chain((1..8).map(|eighth| full.len() * eighth / 8))
            .chain([full.len() - 1])
            .filter(|cut| *cut <= full.len())
            .collect();
        for cut_at in cuts {
            let truncated = &full[..cut_at];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                decode_web_font_to_sfnt(FontFormat::Woff2, truncated)
            }));
            assert!(result.is_ok(), "cut={cut_at}/{} panicked", full.len());
            assert!(
                result.unwrap().is_err(),
                "cut={cut_at}/{} unexpectedly decoded a truncated real WOFF2",
                full.len()
            );
        }
    }

    #[test]
    fn public_web_font_decoder_matches_registration_validation() {
        let sfnt = synthetic_font('A');
        assert_eq!(
            decode_web_font_to_sfnt(FontFormat::OpenType, &sfnt),
            Ok(sfnt.clone())
        );

        let mut bad_checksum = sfnt;
        bad_checksum[16] ^= 1;
        assert_eq!(
            decode_web_font_to_sfnt(FontFormat::OpenType, &bad_checksum),
            Err(TextError::MalformedFont)
        );
        for format in [FontFormat::Woff, FontFormat::Woff2] {
            assert_eq!(
                decode_web_font_to_sfnt(format, &bad_checksum),
                Err(TextError::MalformedFont)
            );
            assert_eq!(
                decode_web_font_to_sfnt(format, &[]),
                Err(TextError::ResourceLimit)
            );
            assert_eq!(
                decode_web_font_to_sfnt(format, &vec![0; MAX_COMPRESSED_FONT_BYTES + 1]),
                Err(TextError::ResourceLimit)
            );
        }
    }

