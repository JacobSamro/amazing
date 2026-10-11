use super::*;
use amazing_text::{FontKey, FontSlant, TextStyle, TextSystem};
fn face(name: &str, weight: u16, japanese: bool) -> Vec<u8> {
    let bytes = crate::installed_font_tests::metadata_font(
        weight,
        5,
        0,
        0.0,
        &[(16, name), (4, name), (6, name)],
    );
    let n = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let tables = (0..n)
        .map(|i| {
            let row = &bytes[12 + i * 16..28 + i * 16];
            let tag: [u8; 4] = row[..4].try_into().unwrap();
            let start = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
            let mut value = bytes[start..start + len].to_vec();
            if &tag == b"OS/2" {
                value[42..58].fill(0);
                value[78..86].fill(0);
                if japanese {
                    value[78..82].copy_from_slice(&(1u32 << 17).to_be_bytes());
                }
            }
            (tag, value)
        })
        .collect::<Vec<_>>();
    let refs = tables
        .iter()
        .map(|(tag, value)| (tag, value.clone()))
        .collect::<Vec<_>>();
    crate::tests::assemble_sfnt(&refs)
}
fn selected(p: &TextProvider, style: &TextStyle, text: &str) -> Option<FontKey> {
    p.matching_font(style, Some(text)).map(|i| p.fonts[i].key)
}
#[test]
fn registered_key_view_distinguishes_actual_native_family_from_configured_alias() {
    let mut p = TextProvider::new();
    let key = p
        .register_installed_font_aliases(
            &["serif"],
            FontFormat::OpenType,
            face("Native Face", 700, true),
        )
        .unwrap();
    let view = p.registered_font_face(key).unwrap();
    assert_eq!(view.key, key);
    assert_eq!(view.family, "serif");
    assert!(!view.author_descriptors);
    let native = view.native.unwrap();
    assert_eq!(native.families, ["Native Face"]);
    assert_eq!(native.weight, 700);
    assert_eq!(native.code_page_ranges, Some([1 << 17, 0]));
    assert!(native.declares_japanese_support());
    assert!(p.registered_font_face(FontKey(0)).is_none());
    assert!(p.registered_font_face(FontKey(u64::MAX)).is_none());
}
#[test]
fn borrowed_registered_views_remain_provider_local_with_independent_keys() {
    let mut a = TextProvider::new();
    let mut b = TextProvider::new();
    let ak = a
        .register_installed_font("A", FontFormat::OpenType, face("Native A", 400, false))
        .unwrap();
    let bk = b
        .register_installed_font("B", FontFormat::OpenType, face("Native B", 700, true))
        .unwrap();
    assert_eq!(ak, bk);
    assert_eq!(
        a.registered_font_face(ak).unwrap().native.unwrap().families,
        ["Native A"]
    );
    assert_eq!(
        b.registered_font_face(bk).unwrap().native.unwrap().families,
        ["Native B"]
    );
}
#[test]
fn unnamed_system_fallback_uses_only_genuine_installed_records_and_actual_coverage() {
    let mut p = TextProvider::new();
    p.register_web_font(WebFont {
        family: "Web Only".into(),
        weight: 400,
        stretch: 100,
        slant: FontSlant::Normal,
        format: FontFormat::OpenType,
        bytes: face("Web Bytes", 400, false),
    })
    .unwrap();
    let style = TextStyle {
        families: vec!["Absent".into()],
        ..Default::default()
    };
    assert_eq!(selected(&p, &style, "A"), None);
    let native = p
        .register_installed_font_aliases(
            &[],
            FontFormat::OpenType,
            face("Real Installed", 400, false),
        )
        .unwrap();
    assert_eq!(selected(&p, &style, "A"), Some(native));
    assert_eq!(selected(&p, &style, "中"), None);
    assert_eq!(
        p.registered_font_face(native).unwrap().family,
        "real installed"
    );
}
#[test]
fn japanese_composite_generic_prefers_native_language_metadata_without_reassigning_named_fonts() {
    let mut p = TextProvider::new();
    let latin = p
        .register_installed_font_aliases(
            &["serif", "Named"],
            FontFormat::OpenType,
            face("Latin Face", 400, false),
        )
        .unwrap();
    let jp = p
        .register_installed_font_aliases(
            &[],
            FontFormat::OpenType,
            face("Japanese Declaration", 400, true),
        )
        .unwrap();
    let mut style = TextStyle {
        families: vec!["serif".into()],
        language: Some("ja-JP".into()),
        ..Default::default()
    };
    assert_eq!(selected(&p, &style, "A"), Some(jp));
    assert_eq!(selected(&p, &style, "中"), None);
    style.families = vec!["Named".into(), "serif".into()];
    assert_eq!(selected(&p, &style, "A"), Some(latin));
    style.families = vec!["serif".into()];
    style.language = Some("en".into());
    assert_eq!(selected(&p, &style, "A"), Some(latin));
    style.language = Some("JA".into());
    assert_eq!(selected(&p, &style, "A"), Some(jp));
    for language in ["Japanese", "ja-", "ja--JP", "ja-x", "ja-JP!"] {
        style.language = Some(language.into());
        assert_eq!(selected(&p, &style, "A"), Some(latin));
    }
    style.language = Some("ja".into());
    style.families = vec!["\"serif\"".into()];
    assert_eq!(selected(&p, &style, "A"), Some(latin));
}
#[test]
fn native_fallback_keeps_real_style_ranking_and_private_use_no_fallback_rule() {
    let mut p = TextProvider::new();
    let regular = p
        .register_installed_font_aliases(&[], FontFormat::OpenType, face("Regular", 400, true))
        .unwrap();
    let bold = p
        .register_installed_font_aliases(&[], FontFormat::OpenType, face("Bold", 700, true))
        .unwrap();
    let mut style = TextStyle {
        families: vec!["Absent".into()],
        language: Some("ja".into()),
        ..Default::default()
    };
    assert_eq!(selected(&p, &style, "A"), Some(regular));
    style.weight = 700;
    assert_eq!(selected(&p, &style, "A"), Some(bold));
    assert_eq!(selected(&p, &style, "\u{e000}"), None);
    let explicit = p
        .register_installed_font(
            "Private",
            FontFormat::OpenType,
            crate::tests::synthetic_font('\u{e000}'),
        )
        .unwrap();
    style.families = vec!["Private".into()];
    assert_eq!(selected(&p, &style, "\u{e000}"), Some(explicit));
}
#[test]
fn os2_declarations_do_not_invent_glyphs_or_replace_shaping_language() {
    let mut p = TextProvider::new();
    let key = p
        .register_installed_font_aliases(&[], FontFormat::OpenType, face("Native", 400, true))
        .unwrap();
    let style = TextStyle {
        families: vec!["serif".into()],
        language: Some("ja".into()),
        ..Default::default()
    };
    let span = amazing_text::TextSpan {
        range: 0..1,
        source: 0,
        style: style.clone(),
        boundary_before: amazing_text::ShapingBoundary::Continue,
    };
    let shaped = p
        .shape(amazing_text::Paragraph {
            text: "A",
            spans: &[span],
            sources: &[],
            bidi_scopes: &[],
            base_direction: amazing_text::Direction::LeftToRight,
        })
        .unwrap();
    assert!(shaped.runs.iter().all(|run| run.font == key));
    assert_eq!(selected(&p, &style, "あ"), None);
    assert!(
        p.registered_font_face(key)
            .unwrap()
            .native
            .unwrap()
            .declares_cjk_support()
    );
}
#[test]
fn author_rule_with_generic_spelling_still_shadows_native_generic_composition() {
    let mut p = TextProvider::new();
    let latin = p
        .register_installed_font("serif", FontFormat::OpenType, face("Latin", 400, false))
        .unwrap();
    p.register_installed_font_aliases(&[], FontFormat::OpenType, face("Japanese", 400, true))
        .unwrap();
    struct Bytes(Vec<u8>);
    impl FontFaceLoader for Bytes {
        fn load_local(&mut self, _: &str, _: usize) -> Result<Option<Vec<u8>>, TextError> {
            Ok(Some(self.0.clone()))
        }
        fn load_url(&mut self, _: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> {
            Ok(None)
        }
    }
    let mut loader = Bytes(face("Authored Bytes", 400, false));
    let author = p
        .register_font_face(
            FontFaceDescriptor {
                family: "serif".into(),
                sources: vec![FontFaceSource::Local("Authored".into())],
                ..Default::default()
            },
            &mut loader,
        )
        .unwrap()
        .unwrap();
    let style = TextStyle {
        families: vec!["serif".into()],
        language: Some("ja".into()),
        ..Default::default()
    };
    assert_eq!(selected(&p, &style, "A"), Some(author));
    assert_ne!(author, latin);
    let view = p.registered_font_face(author).unwrap();
    assert!(view.author_descriptors);
    assert!(view.native.is_none());
}
#[test]
fn exhausted_native_registration_keeps_existing_record_identity_and_metadata() {
    let mut p = TextProvider::new();
    let first = p
        .register_installed_font("Existing", FontFormat::OpenType, face("Native", 400, true))
        .unwrap();
    let resource =
        InstalledFontFace::new(FontFormat::OpenType, face("Additional", 400, false)).unwrap();
    while p.fonts.len() < MAX_FONT_FACES {
        p.register_installed_face_aliases(&[], &resource).unwrap();
    }
    let count = p.fonts.len();
    let generation = p.generation();
    assert!(matches!(
        p.register_installed_face_aliases(&[], &resource),
        Err(amazing_text::TextError::ResourceLimit)
    ));
    assert_eq!(p.fonts.len(), count);
    assert_eq!(p.generation(), generation);
    assert_eq!(
        p.registered_font_face(first)
            .unwrap()
            .native
            .unwrap()
            .families,
        ["Native"]
    );
}
