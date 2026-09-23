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

