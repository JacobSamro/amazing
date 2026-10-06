//! Unit tests for the text-provider adapter, covering registration,
//! matching/fallback, shaping, bidi, line breaking, rasterization and
//! decode/validation, plus `web_font_decode_tests.rs` (spliced in via
//! `include!`, unchanged by the TEXT-27 module split).

use super::*;
use amazing_text::{
    Hyphens, LineBreak, LineRequest, OverflowWrap, ShapingBoundary, TextSpan, UnicodeBidi,
    WordBreak,
};

fn be16(value: u16) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

fn be32(value: u32) -> Vec<u8> {
    value.to_be_bytes().to_vec()
}

pub(super) fn assemble_sfnt(tables: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut tables = tables.to_vec();
    tables.sort_by_key(|(tag, _)| **tag);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&be32(0x0001_0000));
    bytes.extend_from_slice(&be16(tables.len() as u16));
    let entry_selector = (usize::BITS - tables.len().leading_zeros() - 1) as u16;
    let search_range = (1u16 << entry_selector) * 16;
    bytes.extend_from_slice(&be16(search_range));
    bytes.extend_from_slice(&be16(entry_selector));
    bytes.extend_from_slice(&be16(tables.len() as u16 * 16 - search_range));
    let mut offset = (12 + tables.len() * 16) as u32;
    for (tag, table) in &tables {
        bytes.extend_from_slice(*tag);
        bytes.extend_from_slice(&be32(0));
        bytes.extend_from_slice(&be32(offset));
        bytes.extend_from_slice(&be32(table.len() as u32));
        offset += (table.len() as u32 + 3) & !3;
    }
    for (_, table) in &tables {
        bytes.extend_from_slice(table);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
    write_sfnt_checksums(&mut bytes);
    bytes
}

fn write_sfnt_checksums(bytes: &mut [u8]) {
    let table_count = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut head_offset = None;
    for index in 0..table_count {
        let record = 12 + index * 16;
        let offset =
            u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
        let length =
            u32::from_be_bytes(bytes[record + 12..record + 16].try_into().unwrap()) as usize;
        let head = &bytes[record..record + 4] == b"head";
        let checksum = sfnt_checksum(&bytes[offset..offset + length], head.then_some(8));
        bytes[record + 4..record + 8].copy_from_slice(&checksum.to_be_bytes());
        if head {
            head_offset = Some(offset);
        }
    }
    if let Some(offset) = head_offset {
        bytes[offset + 8..offset + 12].fill(0);
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(sfnt_checksum(&bytes, None));
        bytes[offset + 8..offset + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
}

pub(super) fn synthetic_font(mapped: char) -> Vec<u8> {
    synthetic_font_with_feature(mapped, None)
}

/// Wraps `synthetic_font(mapped)` as a one-font `ttcf` collection: a TTC
/// header (tag, version, numFonts=1, one offset entry) followed by the
/// same font bytes, with every table directory offset in the embedded
/// font shifted by the header's length so they stay correct once the
/// font no longer starts at byte 0 of the file.
fn synthetic_ttc(mapped: char) -> Vec<u8> {
    let mut inner = synthetic_font(mapped);
    let header_len: u32 = 16; // tag(4) + version(4) + numFonts(4) + offsetTable[1](4)
    let table_count = u16::from_be_bytes(inner[4..6].try_into().unwrap()) as usize;
    for index in 0..table_count {
        let record = 12 + index * 16;
        let offset = u32::from_be_bytes(inner[record + 8..record + 12].try_into().unwrap());
        inner[record + 8..record + 12]
            .copy_from_slice(&(offset + header_len).to_be_bytes());
    }
    let mut ttc = Vec::new();
    ttc.extend_from_slice(b"ttcf");
    ttc.extend_from_slice(&be32(0x0001_0000)); // major=1, minor=0
    ttc.extend_from_slice(&be32(1)); // numFonts
    ttc.extend_from_slice(&be32(header_len)); // offsetTable[0]
    ttc.extend_from_slice(&inner);
    ttc
}

fn synthetic_font_with_feature(mapped: char, feature: Option<[u8; 4]>) -> Vec<u8> {
    synthetic_font_with_metrics(mapped, feature, 800)
}

pub(super) fn synthetic_font_with_metrics(mapped: char, feature: Option<[u8; 4]>, ascent: u16) -> Vec<u8> {
    synthetic_font_with_metrics_and_color(mapped, feature, ascent, false)
}

/// Same as `synthetic_font_with_metrics`, plus a minimal (all-null,
/// zero-record) COLR table -- exercises `has_color_glyph_tables`'s real
/// table-directory parse against a well-formed font rather than a
/// hand-rolled byte scan.
fn synthetic_font_with_color_table(mapped: char) -> Vec<u8> {
    synthetic_font_with_metrics_and_color(mapped, None, 800, true)
}

fn synthetic_font_with_metrics_and_color(
    mapped: char,
    feature: Option<[u8; 4]>,
    ascent: u16,
    color: bool,
) -> Vec<u8> {
    let mut glyf = vec![0u8; 10];
    glyf[0..2].copy_from_slice(&be16(1));
    glyf[6..8].copy_from_slice(&be16(500));
    glyf[8..10].copy_from_slice(&be16(700));
    glyf.extend_from_slice(&be16(3));
    glyf.extend_from_slice(&be16(0));
    glyf.extend_from_slice(&[0x31, 0x21, 0x11, 0x21]);
    glyf.extend_from_slice(&be16(500));
    glyf.extend_from_slice(&be16(0xFE0C));
    glyf.extend_from_slice(&be16(700));

    let mut cmap_format12 = Vec::new();
    cmap_format12.extend_from_slice(&be16(12));
    cmap_format12.extend_from_slice(&be16(0));
    cmap_format12.extend_from_slice(&be32(28));
    cmap_format12.extend_from_slice(&be32(0));
    cmap_format12.extend_from_slice(&be32(1));
    cmap_format12.extend_from_slice(&be32(mapped as u32));
    cmap_format12.extend_from_slice(&be32(mapped as u32));
    cmap_format12.extend_from_slice(&be32(1));
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be16(3));
    cmap.extend_from_slice(&be16(10));
    cmap.extend_from_slice(&be32(12));
    cmap.extend_from_slice(&cmap_format12);

    let mut head = vec![0u8; 54];
    head[12..16].copy_from_slice(&be32(0x5F0F_3CF5));
    head[18..20].copy_from_slice(&be16(1000));
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&be16(ascent));
    hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
    hhea[34..36].copy_from_slice(&be16(2));
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&be32(0x0001_0000));
    maxp[4..6].copy_from_slice(&be16(2));
    let loca = [be16(0), be16(0), be16(glyf.len() as u16 / 2)].concat();
    let hmtx = [be16(600), be16(0), be16(500), be16(0)].concat();
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"head", head),
        (b"hhea", hhea),
        (b"maxp", maxp),
        (b"hmtx", hmtx),
        (b"cmap", cmap),
        (b"loca", loca),
        (b"glyf", glyf),
    ];
    if let Some(tag) = feature {
        // GSUB: DFLT script, one feature and one ligature lookup mapping
        // glyphs 1+1 to glyph 1. Glyph count shows whether the tag ran.
        let mut gsub = [be32(0x0001_0000), be16(10), be16(30), be16(44)].concat();
        gsub.extend_from_slice(&be16(1));
        gsub.extend_from_slice(b"DFLT");
        gsub.extend_from_slice(&be16(8));
        gsub.extend_from_slice(
            &[be16(4), be16(0), be16(0), be16(0xFFFF), be16(1), be16(0)].concat(),
        );
        gsub.extend_from_slice(&be16(1));
        gsub.extend_from_slice(&tag);
        gsub.extend_from_slice(&[be16(8), be16(0), be16(1), be16(0)].concat());
        gsub.extend_from_slice(
            &[be16(1), be16(4), be16(4), be16(0), be16(1), be16(8)].concat(),
        );
        gsub.extend_from_slice(&[be16(1), be16(8), be16(1), be16(14)].concat());
        gsub.extend_from_slice(
            &[
                be16(1),
                be16(1),
                be16(1),
                be16(1),
                be16(4),
                be16(1),
                be16(2),
                be16(1),
            ]
            .concat(),
        );
        tables.push((b"GSUB", gsub));
    }
    if color {
        // COLR v0 header with zero base-glyph and layer records:
        // version(0) + numBaseGlyphRecords(0) + baseGlyphRecordsOffset
        // (null) + layerRecordsOffset(null) + numLayerRecords(0).
        tables.push((b"COLR", vec![0u8; 14]));
    }
    assemble_sfnt(&tables)
}

/// A font with no GSUB table (so `smcp`/`c2sc` are always unsupported)
/// mapping two distinct characters to two distinct, differently sized
/// glyphs: `lower` to glyph 1, `upper` to glyph 2. Exercises the
/// small-caps synthesis substitution path end to end through real cmap
/// bytes, the way the rest of this file's synthetic fonts exercise GSUB.
pub(super) fn synthetic_font_with_case_pair(lower: char, upper: char) -> Vec<u8> {
    let one_glyph = |x_max: u16, y_max: u16| {
        let mut glyph = vec![0u8; 10];
        glyph[0..2].copy_from_slice(&be16(1));
        glyph[6..8].copy_from_slice(&be16(x_max));
        glyph[8..10].copy_from_slice(&be16(y_max));
        glyph.extend_from_slice(&be16(3));
        glyph.extend_from_slice(&be16(0));
        glyph.extend_from_slice(&[0x31, 0x21, 0x11, 0x21]);
        glyph.extend_from_slice(&be16(x_max));
        glyph.extend_from_slice(&be16(0xFE0C));
        glyph.extend_from_slice(&be16(y_max));
        glyph
    };
    let lower_glyph = one_glyph(500, 700);
    let upper_glyph = one_glyph(600, 720);
    let mut glyf = Vec::new();
    glyf.extend_from_slice(&lower_glyph);
    glyf.extend_from_slice(&upper_glyph);

    // Format 12 groups must be sorted by codepoint for a real cmap
    // reader's binary search, so sort the two (lower, upper) mappings
    // rather than assuming `lower < upper` in code point order.
    let mut groups = [(lower as u32, 1u32), (upper as u32, 2u32)];
    groups.sort_by_key(|(codepoint, _)| *codepoint);
    let mut cmap_format12 = Vec::new();
    cmap_format12.extend_from_slice(&be16(12));
    cmap_format12.extend_from_slice(&be16(0));
    cmap_format12.extend_from_slice(&be32(16 + 12 * groups.len() as u32));
    cmap_format12.extend_from_slice(&be32(0));
    cmap_format12.extend_from_slice(&be32(groups.len() as u32));
    for (codepoint, glyph) in groups {
        cmap_format12.extend_from_slice(&be32(codepoint));
        cmap_format12.extend_from_slice(&be32(codepoint));
        cmap_format12.extend_from_slice(&be32(glyph));
    }
    let mut cmap = Vec::new();
    cmap.extend_from_slice(&be16(0));
    cmap.extend_from_slice(&be16(1));
    cmap.extend_from_slice(&be16(3));
    cmap.extend_from_slice(&be16(10));
    cmap.extend_from_slice(&be32(12));
    cmap.extend_from_slice(&cmap_format12);

    let mut head = vec![0u8; 54];
    head[12..16].copy_from_slice(&be32(0x5F0F_3CF5));
    head[18..20].copy_from_slice(&be16(1000));
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&be16(800));
    hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
    hhea[34..36].copy_from_slice(&be16(3));
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&be32(0x0001_0000));
    maxp[4..6].copy_from_slice(&be16(3));
    let loca = [
        be16(0),
        be16(0),
        be16(lower_glyph.len() as u16 / 2),
        be16(glyf.len() as u16 / 2),
    ]
    .concat();
    let hmtx = [
        be16(600),
        be16(0),
        be16(500),
        be16(0),
        be16(500),
        be16(0),
    ]
    .concat();
    let tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"head", head),
        (b"hhea", hhea),
        (b"maxp", maxp),
        (b"hmtx", hmtx),
        (b"cmap", cmap),
        (b"loca", loca),
        (b"glyf", glyf),
    ];
    assemble_sfnt(&tables)
}

#[derive(Default)]
struct FakeFontLoader {
    locals: HashMap<String, Vec<u8>>,
    urls: HashMap<String, Vec<u8>>,
    calls: Vec<String>,
    requests: Vec<(String, bool, usize)>,
}

impl FontFaceLoader for FakeFontLoader {
    fn load_local(
        &mut self,
        name: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, TextError> {
        self.calls.push(format!("local:{name}"));
        assert_eq!(max_bytes, MAX_COMPRESSED_FONT_BYTES);
        Ok(self.locals.get(name).cloned())
    }

    fn load_url(
        &mut self,
        request: FontFetchRequest<'_>,
    ) -> Result<Option<Vec<u8>>, TextError> {
        self.calls.push(format!("url:{}", request.url));
        self.requests
            .push((request.url.to_owned(), request.cors_mode, request.max_bytes));
        Ok(self.urls.get(request.url).cloned())
    }
}

#[test]
fn font_face_tries_sources_in_order_and_records_display() {
    let mut loader = FakeFontLoader::default();
    loader.urls.insert("bad.woff2".into(), synthetic_font('A'));
    loader.urls.insert("good.ttf".into(), synthetic_font('A'));
    let descriptor = FontFaceDescriptor {
        family: "Author Face".into(),
        sources: vec![
            FontFaceSource::Local("Absent".into()),
            FontFaceSource::Url {
                url: "skip.ttf".into(),
                formats: vec!["truetype".into()],
                technologies: vec!["features-graphite".into()],
            },
            FontFaceSource::Url {
                url: "bad.woff2".into(),
                formats: vec!["woff2".into()],
                technologies: vec![],
            },
            FontFaceSource::Url {
                url: "good.ttf".into(),
                formats: vec!["truetype".into()],
                technologies: vec!["features-opentype".into()],
            },
        ],
        display: FontDisplay::Swap,
        ..FontFaceDescriptor::default()
    };
    let mut provider = TextProvider::new();
    let key = provider
        .register_font_face(descriptor, &mut loader)
        .unwrap()
        .unwrap();
    assert_eq!(provider.font_display(key), Some(FontDisplay::Swap));
    assert_eq!(provider.font_face_registrations()[0].key, Some(key));
    assert_eq!(
        provider.font_face_registrations()[0].display,
        FontDisplay::Swap
    );
    assert_eq!(
        loader.calls,
        ["local:Absent", "url:bad.woff2", "url:good.ttf"]
    );
    assert!(
        loader
            .requests
            .iter()
            .all(|(_, cors, cap)| *cors && *cap == MAX_COMPRESSED_FONT_BYTES)
    );
}

#[test]
fn font_face_unicode_ranges_form_reverse_order_composite_and_shadow_installed() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Composite".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let mut loader = FakeFontLoader::default();
    loader.urls.insert("first.ttf".into(), synthetic_font('A'));
    loader.urls.insert("second.ttf".into(), synthetic_font('B'));
    for (url, ch) in [("first.ttf", 'A'), ("second.ttf", 'B')] {
        provider
            .register_font_face(
                FontFaceDescriptor {
                    family: "Composite".into(),
                    sources: vec![FontFaceSource::Url {
                        url: url.into(),
                        formats: vec![],
                        technologies: vec![],
                    }],
                    unicode_ranges: vec![UnicodeRange {
                        start: ch as u32,
                        end: ch as u32,
                    }],
                    weight: FontDescriptorRange {
                        min: 400.0,
                        max: 800.0,
                    },
                    stretch: FontDescriptorRange {
                        min: 75.0,
                        max: 125.0,
                    },
                    ..FontFaceDescriptor::default()
                },
                &mut loader,
            )
            .unwrap();
    }
    let style = TextStyle {
        families: vec!["Composite".into()],
        weight: 700,
        stretch: 120,
        ..TextStyle::default()
    };
    assert_eq!(provider.matching_font(&style, Some("A")), Some(1));
    assert_eq!(provider.matching_font(&style, Some("B")), Some(2));
    assert_eq!(provider.matching_font(&style, Some("C")), None);
    assert_eq!(provider.fonts[1].attributes_for(&style).weight, 700);
    assert_eq!(provider.fonts[1].attributes_for(&style).stretch, 120);
}

#[test]
fn font_face_rejects_invalid_ranges_and_keeps_failed_author_family_missing() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Declared".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let mut loader = FakeFontLoader::default();
    let invalid = FontFaceDescriptor {
        family: "Declared".into(),
        sources: vec![FontFaceSource::Local("missing".into())],
        weight: FontDescriptorRange {
            min: 800.0,
            max: 400.0,
        },
        ..FontFaceDescriptor::default()
    };
    assert_eq!(
        provider.register_font_face(invalid, &mut loader),
        Err(TextError::InvalidStyle)
    );
    let missing = FontFaceDescriptor {
        family: "Declared".into(),
        sources: vec![FontFaceSource::Local("missing".into())],
        ..FontFaceDescriptor::default()
    };
    assert_eq!(provider.register_font_face(missing, &mut loader), Ok(None));
    assert_eq!(provider.font_face_registrations()[0].key, None);
    assert_eq!(
        provider.font_face_registrations()[0].display,
        FontDisplay::Auto
    );
    let style = TextStyle {
        families: vec!["Declared".into()],
        ..TextStyle::default()
    };
    assert_eq!(provider.matching_font(&style, Some("A")), None);
}

include!("web_font_decode_tests.rs");

#[test]
fn malformed_woff2_fails_closed_without_changing_generation() {
    let mut provider = TextProvider::new();
    let result = provider.register_web_font(WebFont {
        family: "Broken".into(),
        weight: 400,
        stretch: 100,
        slant: FontSlant::Normal,
        format: FontFormat::Woff2,
        bytes: vec![0; 64],
    });

    assert_eq!(result, Err(TextError::MalformedFont));
    assert_eq!(provider.generation(), 0);
}

#[test]
fn sfnt_directory_bounds_and_checksums_reject_corrupt_input() {
    let original = synthetic_font('A');
    let mut truncated_directory = original.clone();
    truncated_directory[4..6].copy_from_slice(&200u16.to_be_bytes());
    let mut bad_offset = original.clone();
    bad_offset[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut bad_table_checksum = original.clone();
    bad_table_checksum[16] ^= 1;
    for bytes in [truncated_directory, bad_offset, bad_table_checksum] {
        let result = TextProvider::new().register_web_font(WebFont {
            family: "Malformed".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes,
        });
        assert!(matches!(
            result,
            Err(TextError::MalformedFont | TextError::ResourceLimit)
        ));
    }

    // A stale checkSumAdjustment does not invalidate otherwise sound
    // tables; the pinned CSSTest fonts rely on this compatibility.
    let mut stale_global_checksum = original;
    let head_record = (0..u16::from_be_bytes(stale_global_checksum[4..6].try_into().unwrap())
        as usize)
        .map(|index| 12 + index * 16)
        .find(|record| &stale_global_checksum[*record..*record + 4] == b"head")
        .unwrap();
    let head_offset = u32::from_be_bytes(
        stale_global_checksum[head_record + 8..head_record + 12]
            .try_into()
            .unwrap(),
    ) as usize;
    stale_global_checksum[head_offset + 8] ^= 1;
    assert!(
        TextProvider::new()
            .register_web_font(WebFont {
                family: "Stale checksum".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: stale_global_checksum,
            })
            .is_ok()
    );
}

/// TEXT-19: a truncated web font must always fail closed (a clean
/// `TextError`, never a panic) regardless of where the cut lands, since
/// this is exactly the shape of a network short-read (the live-pages
/// corpus's `Io(Error{kind: UnexpectedEof, ...})` failures, see
/// `.agents/text/questions.md` and `docs/lanes/text_TODO.md`'s TEXT-19
/// section: that error is `amazing-net`'s, raised before the decoder
/// ever sees the bytes, not a decoder bug -- this test locks in that a
/// synthetic WOFF2-shaped buffer's decode was never the gap). The real
/// pinned-WOFF2 counterpart is
/// `pinned_wpt_woff2_fails_closed_at_every_truncation_cut` in
/// `web_font_decode_tests.rs`.
#[test]
fn truncated_woff2_fails_closed_at_every_cut_point_never_panics() {
    let source = synthetic_font('T');
    for cut_at in [0, 1, 10, 50, 200, source.len() / 2, source.len() - 1] {
        let truncated = &source[..cut_at.min(source.len())];
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            decode_web_font_to_sfnt(FontFormat::Woff2, truncated)
        }));
        assert!(result.is_ok(), "cut={cut_at} panicked");
        assert!(
            result.unwrap().is_err(),
            "cut={cut_at} unexpectedly decoded a non-WOFF2 buffer as WOFF2"
        );
    }
}

/// TEXT-19: the Linux system font scan can pick a `.ttc` collection
/// (common for CJK families such as Noto Sans CJK); registering one must
/// shape and rasterize a real glyph from its first font, not fail or
/// pick up garbage from the TTC header/offset table.
#[test]
fn ttc_collection_registers_shapes_and_rasterizes_its_first_font() {
    let ttc = synthetic_ttc('T');
    assert_eq!(sniff_font_format(&ttc), Some(FontFormat::Collection));

    let mut provider = TextProvider::new();
    let key = provider
        .register_web_font(WebFont {
            family: "Collection".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::Collection,
            bytes: ttc,
        })
        .expect("a well-formed one-font ttc registers");

    let style = TextStyle {
        families: vec!["Collection".into()],
        size: 16.0,
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..1,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];
    let shaped = provider
        .shape(Paragraph {
            text: "T",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .expect("shapes through the ttc");
    assert_eq!(shaped.runs.len(), 1);
    assert_eq!(shaped.runs[0].font, key);
    assert_ne!(
        shaped.runs[0].glyphs[0].id,
        amazing_text::GlyphId(0),
        "shaped the .notdef glyph, not 'T'"
    );

    let raster_key = GlyphCacheKey::new(
        provider.generation(),
        key,
        shaped.runs[0].glyphs[0].id,
        16.0,
        0,
        0,
    )
    .unwrap();
    let image = provider
        .rasterize(GlyphRasterRequest {
            key: raster_key,
            variations: &[],
            synthesis: shaped.runs[0].synthesis,
        })
        .expect("rasterizes without erroring")
        .expect("a real glyph produces an image");
    assert!(
        image.width > 0 && image.height > 0,
        "empty raster for a real glyph"
    );
}

/// TEXT-19: the same short-read robustness as
/// `truncated_woff2_fails_closed_at_every_cut_point_never_panics`, but
/// for a `ttcf` buffer specifically -- a truncated TTC header/offset
/// table or a truncated embedded font must not panic or select a
/// garbage offset.
#[test]
fn truncated_ttc_fails_closed_never_panics() {
    let source = synthetic_ttc('T');
    for cut_at in [0, 4, 8, 12, 15, 16, 20, 50, source.len() / 2, source.len() - 1] {
        let truncated = &source[..cut_at.min(source.len())];
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            decode_web_font_to_sfnt(FontFormat::Collection, truncated)
        }));
        assert!(result.is_ok(), "cut={cut_at} panicked");
        assert!(
            result.unwrap().is_err(),
            "cut={cut_at} unexpectedly decoded a truncated ttc"
        );
    }
}

#[test]
fn oversized_web_font_is_rejected_before_decode() {
    let mut provider = TextProvider::new();
    let result = provider.register_web_font(WebFont {
        family: "TooLarge".into(),
        weight: 400,
        stretch: 100,
        slant: FontSlant::Normal,
        format: FontFormat::Woff2,
        bytes: vec![0; MAX_COMPRESSED_FONT_BYTES + 1],
    });

    assert_eq!(result, Err(TextError::ResourceLimit));
    assert_eq!(provider.generation(), 0);
}

#[test]
fn aggregate_font_set_bytes_are_bounded() {
    let bytes = synthetic_font('A');
    let mut provider = TextProvider::new();
    provider.font_set_bytes = MAX_FONT_SET_BYTES - bytes.len() + 1;

    assert_eq!(
        provider.register_web_font(WebFont {
            family: "AggregateLimit".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes,
        }),
        Err(TextError::ResourceLimit)
    );
}

#[test]
fn font_table_and_glyph_counts_have_explicit_limits() {
    let mut too_many_tables = synthetic_font('A');
    too_many_tables[4..6].copy_from_slice(&((MAX_FONT_TABLES + 1) as u16).to_be_bytes());
    assert_eq!(
        TextProvider::new().register_web_font(WebFont {
            family: "Tables".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: too_many_tables,
        }),
        Err(TextError::ResourceLimit)
    );

    let mut too_many_glyphs = synthetic_font('A');
    let table_count = u16::from_be_bytes([too_many_glyphs[4], too_many_glyphs[5]]) as usize;
    let maxp_offset = (0..table_count)
        .map(|index| 12 + index * 16)
        .find(|offset| &too_many_glyphs[*offset..*offset + 4] == b"maxp")
        .map(|offset| {
            u32::from_be_bytes(too_many_glyphs[offset + 8..offset + 12].try_into().unwrap())
                as usize
        })
        .unwrap();
    too_many_glyphs[maxp_offset + 4..maxp_offset + 6]
        .copy_from_slice(&((MAX_FONT_GLYPHS + 1) as u16).to_be_bytes());
    write_sfnt_checksums(&mut too_many_glyphs);
    assert_eq!(
        TextProvider::new().register_web_font(WebFont {
            family: "Glyphs".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: too_many_glyphs,
        }),
        Err(TextError::ResourceLimit)
    );
}

#[test]
fn blank_raster_cache_entries_have_a_hard_count_limit() {
    let provider = TextProvider::new();
    for glyph in 0..(MAX_RASTER_CACHE_ENTRIES as u32 + 100) {
        provider.cache_raster(
            GlyphCacheKey {
                generation: 0,
                font: FontKey(0),
                glyph: GlyphId(glyph),
                size_64: 16 * 64,
                subpixel_x_64: 0,
                subpixel_y_64: 0,
                variation_hash: 0,
                synthesis: 0,
            },
            None,
        );
    }

    assert_eq!(
        provider.raster_cache.borrow().len(),
        MAX_RASTER_CACHE_ENTRIES
    );
    assert!(provider.raster_cache_bytes.get() > 0);
}

#[test]
fn registered_face_shapes_clusters_and_rasterizes_the_same_glyph_id() {
    let mut provider = TextProvider::new();
    let key = provider
        .register_web_font(WebFont {
            family: "Synthetic".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Synthetic".into()],
        size: 10.0,
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..1,
        source: 7,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text: "A",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(shaped.runs.len(), 1);
    assert_eq!(shaped.runs[0].font, key);
    assert_eq!(shaped.runs[0].glyphs[0].id, amazing_text::GlyphId(1));
    assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..1);
    assert!((shaped.runs[0].glyphs[0].advance_x - 5.0).abs() < 0.02);

    let raster_key = GlyphCacheKey::new(
        provider.generation(),
        key,
        shaped.runs[0].glyphs[0].id,
        10.0,
        0,
        0,
    )
    .unwrap();
    let image = provider
        .rasterize(GlyphRasterRequest {
            key: raster_key,
            variations: &[],
            synthesis: shaped.runs[0].synthesis,
        })
        .unwrap()
        .unwrap();
    assert_eq!((image.width, image.height), (5, 7));
    assert_eq!(image.content, amazing_text::GlyphContent::Mask);
}

#[test]
fn has_color_glyph_tables_detects_colr_and_not_plain_fonts() {
    assert!(!has_color_glyph_tables(&synthetic_font('A')));
    assert!(has_color_glyph_tables(&synthetic_font_with_color_table('A')));
}

#[test]
fn emoji_presentation_intent_reads_the_trailing_variation_selector() {
    assert_eq!(
        emoji_presentation_intent("\u{263A}\u{FE0E}"),
        Some(EmojiPresentation::Text)
    );
    assert_eq!(
        emoji_presentation_intent("\u{263A}\u{FE0F}"),
        Some(EmojiPresentation::Emoji)
    );
    assert_eq!(emoji_presentation_intent("\u{263A}"), None);
    assert_eq!(emoji_presentation_intent(""), None);
}

/// CSS Fonts' font-variant-emoji / UTR #51: a trailing U+FE0F forces
/// emoji presentation even when the declared family ("serif" here, as in
/// WPT's font-variant-emoji-1.html) already maps the bare code point --
/// real engines still switch to a colour-capable face. U+FE0E (or no
/// selector) keeps the declared face, since this font's own default
/// presentation for the character is already the plain glyph.
#[test]
fn emoji_variation_selectors_pick_presentation_appropriate_fonts() {
    let mut provider = TextProvider::new();
    let text_font = provider
        .register_web_font(WebFont {
            family: "serif".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{263A}'),
        })
        .unwrap();
    let emoji_font = provider
        .register_web_font(WebFont {
            family: "emoji".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_color_table('\u{263A}'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["serif".into()],
        size: 10.0,
        ..TextStyle::default()
    };
    let font_key = |index: Option<usize>| index.map(|index| provider.fonts[index].key);

    assert_eq!(
        font_key(provider.matching_font(&style, Some("\u{263A}"))),
        Some(text_font)
    );
    assert_eq!(
        font_key(provider.matching_font(&style, Some("\u{263A}\u{FE0E}"))),
        Some(text_font)
    );
    assert_eq!(
        font_key(provider.matching_font(&style, Some("\u{263A}\u{FE0F}"))),
        Some(emoji_font)
    );
}

#[test]
fn synthetic_oblique_changes_the_registered_face_outline() {
    let mut provider = TextProvider::new();
    let font = provider
        .register_web_font(WebFont {
            family: "Regular only".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 30.0, 0, 0).unwrap();
    let normal = provider
        .rasterize(GlyphRasterRequest {
            key,
            variations: &[],
            synthesis: amazing_text::FontSynthesis {
                weight: false,
                style: false,
                small_caps: false,
                position: false,
            },
        })
        .unwrap()
        .unwrap();
    let oblique = provider
        .rasterize(GlyphRasterRequest {
            key: key.with_style(
                &[],
                amazing_text::FontSynthesis {
                    weight: false,
                    style: true,
                    small_caps: false,
                    position: false,
                },
            ),
            variations: &[],
            synthesis: amazing_text::FontSynthesis {
                weight: false,
                style: true,
                small_caps: false,
                position: false,
            },
        })
        .unwrap()
        .unwrap();
    assert_ne!(
        (normal.width, normal.bearing_x, normal.data),
        (oblique.width, oblique.bearing_x, oblique.data)
    );
}

#[test]
fn synthetic_bold_changes_the_registered_face_outline() {
    let mut provider = TextProvider::new();
    let font = provider
        .register_web_font(WebFont {
            family: "Regular only".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 30.0, 0, 0).unwrap();
    let normal = provider
        .rasterize(GlyphRasterRequest {
            key,
            variations: &[],
            synthesis: amazing_text::FontSynthesis::default(),
        })
        .unwrap()
        .unwrap();
    let synthesis = amazing_text::FontSynthesis {
        weight: true,
        style: false,
        small_caps: false,
        position: false,
    };
    let bold = provider
        .rasterize(GlyphRasterRequest {
            key: key.with_style(&[], synthesis),
            variations: &[],
            synthesis,
        })
        .unwrap()
        .unwrap();
    assert_ne!((normal.width, normal.data), (bold.width, bold.data));
}

#[test]
fn synthetic_small_caps_changes_the_registered_face_outline() {
    let mut provider = TextProvider::new();
    let font = provider
        .register_web_font(WebFont {
            family: "Regular only".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 30.0, 0, 0).unwrap();
    let normal = provider
        .rasterize(GlyphRasterRequest {
            key,
            variations: &[],
            synthesis: amazing_text::FontSynthesis {
                weight: false,
                style: false,
                small_caps: false,
                position: false,
            },
        })
        .unwrap()
        .unwrap();
    let synthesis = amazing_text::FontSynthesis {
        weight: false,
        style: false,
        small_caps: true,
        position: false,
    };
    let small_caps = provider
        .rasterize(GlyphRasterRequest {
            key: key.with_style(&[], synthesis),
            variations: &[],
            synthesis,
        })
        .unwrap()
        .unwrap();
    assert_ne!(
        (normal.width, normal.data),
        (small_caps.width, small_caps.data)
    );
}

#[test]
fn synthetic_position_changes_the_registered_face_outline() {
    let mut provider = TextProvider::new();
    let font = provider
        .register_web_font(WebFont {
            family: "Regular only".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let key = GlyphCacheKey::new(provider.generation(), font, GlyphId(1), 30.0, 0, 0).unwrap();
    let normal = provider
        .rasterize(GlyphRasterRequest {
            key,
            variations: &[],
            synthesis: amazing_text::FontSynthesis {
                weight: false,
                style: false,
                small_caps: false,
                position: false,
            },
        })
        .unwrap()
        .unwrap();
    let synthesis = amazing_text::FontSynthesis {
        weight: false,
        style: false,
        small_caps: false,
        position: true,
    };
    let position = provider
        .rasterize(GlyphRasterRequest {
            key: key.with_style(&[], synthesis),
            variations: &[],
            synthesis,
        })
        .unwrap()
        .unwrap();
    assert_ne!((normal.width, normal.data), (position.width, position.data));
}

#[test]
fn feature_on_and_off_reach_opentype_shaping() {
    for tag in [*b"liga", *b"clig", *b"dlig", *b"hlig", *b"calt"] {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Feature fixture".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font_with_feature('A', Some(tag)),
            })
            .unwrap();
        let shape = |value| {
            let style = TextStyle {
                families: vec!["Feature fixture".into()],
                features: vec![amazing_text::FeatureSetting { tag, value }],
                ..TextStyle::default()
            };
            let spans = [TextSpan {
                range: 0..2,
                source: 0,
                style,
                boundary_before: ShapingBoundary::Continue,
            }];
            provider
                .shape(Paragraph {
                    text: "AA",
                    spans: &spans,
                    sources: &[],
                    bidi_scopes: &[],
                    base_direction: Direction::LeftToRight,
                })
                .unwrap()
                .runs
                .iter()
                .map(|run| run.glyphs.len())
                .sum::<usize>()
        };
        assert_eq!(shape(0), 2, "{} disabled", String::from_utf8_lossy(&tag));
        assert_eq!(shape(1), 1, "{} enabled", String::from_utf8_lossy(&tag));
    }
}

#[test]
fn explicit_smcp_substitutes_the_face_mapped_uppercase_glyph_when_unsupported() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Case pair".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_case_pair('a', 'A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Case pair".into()],
        features: vec![amazing_text::FeatureSetting {
            tag: *b"smcp",
            value: 1,
        }],
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..1,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];
    let shaped = provider
        .shape(Paragraph {
            text: "a",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(shaped.runs[0].glyphs[0].id, amazing_text::GlyphId(2));
    assert!(shaped.runs[0].synthesis.small_caps);
}

#[test]
fn smcp_synthesis_is_skipped_when_font_synthesis_disallows_small_caps() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Case pair".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_case_pair('a', 'A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Case pair".into()],
        features: vec![amazing_text::FeatureSetting {
            tag: *b"smcp",
            value: 1,
        }],
        synthesis: amazing_text::FontSynthesis {
            small_caps: false,
            ..amazing_text::FontSynthesis::default()
        },
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..1,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];
    let shaped = provider
        .shape(Paragraph {
            text: "a",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(shaped.runs[0].glyphs[0].id, amazing_text::GlyphId(1));
    assert!(!shaped.runs[0].synthesis.small_caps);
}

#[test]
fn c2sc_scales_an_already_uppercase_letter_without_substitution() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Case pair".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_case_pair('a', 'A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Case pair".into()],
        features: vec![amazing_text::FeatureSetting {
            tag: *b"c2sc",
            value: 1,
        }],
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..1,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];
    let shaped = provider
        .shape(Paragraph {
            text: "A",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    // Already the uppercase (id 2) glyph: c2sc scales it in place, no
    // substitution needed.
    assert_eq!(shaped.runs[0].glyphs[0].id, amazing_text::GlyphId(2));
    assert!(shaped.runs[0].synthesis.small_caps);
}

#[test]
fn explicit_sups_flags_position_synthesis_and_raises_the_baseline() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Position fixture".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('a'),
        })
        .unwrap();
    let shape = |value| {
        let style = TextStyle {
            families: vec!["Position fixture".into()],
            size: 20.0,
            features: vec![amazing_text::FeatureSetting {
                tag: *b"sups",
                value,
            }],
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..1,
            source: 0,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];
        provider
            .shape(Paragraph {
                text: "a",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap()
    };
    let unsynthesized = shape(0);
    let raised = shape(1);
    assert!(!unsynthesized.runs[0].synthesis.position);
    assert!(raised.runs[0].synthesis.position);
    assert!(raised.runs[0].glyphs[0].offset_y > unsynthesized.runs[0].glyphs[0].offset_y);
}

#[test]
fn kerning_features_are_limited_to_their_shaping_axis() {
    // layout expresses `font-kerning: normal` as kern=1 plus vkrn=1. Each
    // tag must only run on its own axis (font-kerning-02 horizontal,
    // font-kerning-03 vertical), even though HarfBuzz would apply an
    // explicitly requested tag in either buffer direction.
    let shape = |tag: [u8; 4], vertical: bool| -> usize {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Kerning fixture".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font_with_feature('A', Some(tag)),
            })
            .unwrap();
        let style = TextStyle {
            families: vec!["Kerning fixture".into()],
            writing_mode: if vertical {
                WritingMode::VerticalRl
            } else {
                WritingMode::HorizontalTb
            },
            text_orientation: if vertical {
                TextOrientation::Upright
            } else {
                TextOrientation::Mixed
            },
            features: vec![
                amazing_text::FeatureSetting {
                    tag: *b"kern",
                    value: 1,
                },
                amazing_text::FeatureSetting {
                    tag: *b"vkrn",
                    value: 1,
                },
            ],
            ..TextStyle::default()
        };
        let spans = [TextSpan {
            range: 0..2,
            source: 0,
            style,
            boundary_before: ShapingBoundary::Continue,
        }];
        provider
            .shape(Paragraph {
                text: "AA",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap()
            .runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum()
    };
    assert_eq!(shape(*b"kern", false), 1, "kern runs horizontally");
    assert_eq!(shape(*b"vkrn", true), 1, "vkrn runs vertically");
    assert_eq!(shape(*b"kern", true), 2, "kern is dropped vertically");
    assert_eq!(shape(*b"vkrn", false), 2, "vkrn is dropped horizontally");
}

#[test]
fn explicit_kerning_off_still_disables_the_axis_feature() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Kerning fixture".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_feature('A', Some(*b"kern")),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Kerning fixture".into()],
        features: vec![
            amazing_text::FeatureSetting {
                tag: *b"kern",
                value: 0,
            },
            amazing_text::FeatureSetting {
                tag: *b"vkrn",
                value: 1,
            },
        ],
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..2,
        source: 0,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];
    let glyphs = provider
        .shape(Paragraph {
            text: "AA",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap()
        .runs
        .iter()
        .map(|run| run.glyphs.len())
        .sum::<usize>();
    assert_eq!(glyphs, 2, "kern=off suppresses the horizontal lookup");
}

#[test]
fn vertical_upright_uses_vert_and_never_defaults_to_vrt2() {
    let glyph_count = |tag, writing_mode, text_orientation| {
        let mut provider = TextProvider::new();
        provider
            .register_web_font(WebFont {
                family: "Vertical feature".into(),
                weight: 400,
                stretch: 100,
                slant: FontSlant::Normal,
                format: FontFormat::OpenType,
                bytes: synthetic_font_with_feature('A', Some(tag)),
            })
            .unwrap();
        let spans = [TextSpan {
            range: 0..2,
            source: 0,
            style: TextStyle {
                families: vec!["Vertical feature".into()],
                writing_mode,
                text_orientation,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];
        provider
            .shape(Paragraph {
                text: "AA",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap()
            .runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>()
    };
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        assert_eq!(glyph_count(*b"vert", mode, TextOrientation::Upright), 1);
        assert_eq!(glyph_count(*b"vert", mode, TextOrientation::Sideways), 2);
        assert_eq!(glyph_count(*b"vrt2", mode, TextOrientation::Upright), 2);
    }
    assert_eq!(
        glyph_count(
            *b"vert",
            WritingMode::HorizontalTb,
            TextOrientation::Upright
        ),
        2
    );
}

#[test]
fn mixed_vertical_orientation_follows_unicode_vo_for_each_base_character() {
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let style = TextStyle {
            writing_mode: mode,
            text_orientation: TextOrientation::Mixed,
            ..TextStyle::default()
        };
        for (text, expected) in [
            ("A", GlyphOrientation::SidewaysRight),
            ("漢", GlyphOrientation::Upright),
            ("Ａ", GlyphOrientation::Upright),
            ("（", GlyphOrientation::Upright),
            ("\u{200d}漢", GlyphOrientation::Upright),
        ] {
            assert_eq!(
                cluster_orientation(&style, text, 0..text.len()),
                expected,
                "{text}"
            );
        }
    }
    let style = TextStyle {
        writing_mode: WritingMode::SidewaysLr,
        ..TextStyle::default()
    };
    assert_eq!(
        cluster_orientation(&style, "漢", 0.."漢".len()),
        GlyphOrientation::SidewaysLeft
    );
}

#[test]
fn upright_vertical_text_forces_left_to_right_bidi_direction() {
    let text = "אב";
    for (mode, expected) in [
        (WritingMode::VerticalRl, Direction::LeftToRight),
        (WritingMode::VerticalLr, Direction::LeftToRight),
        (WritingMode::SidewaysRl, Direction::RightToLeft),
        (WritingMode::SidewaysLr, Direction::RightToLeft),
    ] {
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 0,
            style: TextStyle {
                writing_mode: mode,
                text_orientation: TextOrientation::Upright,
                direction: Direction::RightToLeft,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        }];
        assert_eq!(
            bidi_directions(text, &spans, &spans, Direction::RightToLeft, &[]).unwrap(),
            vec![expected]
        );
    }
}

#[test]
fn synchronous_registration_has_no_pending_font_loads() {
    let mut provider = TextProvider::new();
    assert_eq!(provider.pending_font_loads(), 0);
    provider
        .register_web_font(WebFont {
            family: "Registered".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    assert_eq!(provider.pending_font_loads(), 0);
}

#[test]
fn author_metric_and_feature_descriptors_reach_shaping() {
    let mut loader = FakeFontLoader::default();
    loader.urls.insert(
        "face.otf".into(),
        synthetic_font_with_feature('A', Some(*b"liga")),
    );
    let mut provider = TextProvider::new();
    provider.begin_font_load(FontDisplay::Fallback).unwrap();
    assert_eq!(provider.pending_font_loads(), 1);
    assert_eq!(
        provider.pending_font_display_periods(),
        [FontDisplayPeriods {
            block_ms: 100,
            swap_ms: Some(3000)
        }]
    );
    provider
        .register_font_face(
            FontFaceDescriptor {
                family: "Adjusted".into(),
                sources: vec![FontFaceSource::Url {
                    url: "face.otf".into(),
                    formats: vec![],
                    technologies: vec![],
                }],
                size_adjust: 1.25,
                ascent_override: Some(0.8),
                descent_override: Some(0.2),
                line_gap_override: Some(0.1),
                features: vec![amazing_text::FeatureSetting {
                    tag: *b"liga",
                    value: 1,
                }],
                ..FontFaceDescriptor::default()
            },
            &mut loader,
        )
        .unwrap()
        .unwrap();
    provider.end_font_load();
    assert_eq!(provider.pending_font_loads(), 0);
    let mut style = TextStyle {
        families: vec!["Adjusted".into()],
        size: 20.0,
        ..TextStyle::default()
    };
    let metrics = font_metrics(&provider.fonts[0], style.size).unwrap();
    assert_eq!(
        (metrics.ascent, metrics.descent, metrics.leading),
        (20.0, 5.0, 2.5)
    );
    let count = |style: &TextStyle| {
        let spans = [TextSpan {
            range: 0..2,
            source: 0,
            style: style.clone(),
            boundary_before: ShapingBoundary::Continue,
        }];
        provider
            .shape(Paragraph {
                text: "AA",
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            })
            .unwrap()
            .runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>()
    };
    assert_eq!(count(&style), 1);
    style.features.push(amazing_text::FeatureSetting {
        tag: *b"liga",
        value: 0,
    });
    assert_eq!(count(&style), 2);
}

#[test]
fn fallback_selects_a_covering_face_per_grapheme_cluster() {
    let mut provider = TextProvider::new();
    let primary = provider
        .register_web_font(WebFont {
            family: "Primary".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let secondary = provider
        .register_web_font(WebFont {
            family: "Secondary".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('B'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Primary".into(), "Secondary".into()],
        size: 10.0,
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..2,
        source: 9,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text: "AB",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(shaped.runs.len(), 2);
    assert_eq!(shaped.runs[0].font, primary);
    assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..1);
    assert_eq!(shaped.runs[1].font, secondary);
    assert_eq!(shaped.runs[1].glyphs[0].cluster, 1..2);
    assert!(shaped.runs.iter().all(|run| run.glyphs[0].id == GlyphId(1)));
}

#[test]
fn earlier_named_family_beats_later_face_regardless_of_attribute_distance() {
    let mut provider = TextProvider::new();
    let first = provider
        .register_web_font(WebFont {
            family: "CSSTest Multi Word".into(),
            weight: 1000,
            stretch: 1000,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    provider
        .register_web_font(WebFont {
            family: "second".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["'csstest multi word'".into(), "second".into()],
        ..TextStyle::default()
    };
    assert_eq!(
        provider.fonts[provider.matching_font(&style, Some("A")).unwrap()].key,
        first
    );
}

#[test]
fn missing_glyph_in_selected_face_advances_to_next_family() {
    let mut provider = TextProvider::new();
    for (family, slant, glyph) in [
        ("primary", FontSlant::Normal, 'A'),
        ("primary", FontSlant::Italic, 'B'),
        ("secondary", FontSlant::Normal, 'B'),
    ] {
        provider
            .register_web_font(WebFont {
                family: family.into(),
                weight: 400,
                stretch: 100,
                slant,
                format: FontFormat::OpenType,
                bytes: synthetic_font(glyph),
            })
            .unwrap();
    }
    let style = TextStyle {
        families: vec!["primary".into(), "secondary".into()],
        ..TextStyle::default()
    };
    let selected = provider.matching_font(&style, Some("B")).unwrap();
    assert_eq!(provider.fonts[selected].family, "secondary");
}

#[test]
fn missing_author_family_uses_covering_installed_fallback() {
    let mut provider = TextProvider::new();
    let serif = provider
        .register_web_font(WebFont {
            family: "serif".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["'Uninstalled Family'".into()],
        ..TextStyle::default()
    };
    assert_eq!(
        provider.fonts[provider.matching_font(&style, Some("A")).unwrap()].key,
        serif
    );
    assert_eq!(provider.matching_font(&style, Some("B")), None);
}

#[test]
fn installed_fallback_chooses_the_matching_face_within_its_family() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "serif".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let bold = provider
        .register_web_font(WebFont {
            family: "serif".into(),
            weight: 700,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Unknown".into()],
        weight: 700,
        ..TextStyle::default()
    };
    assert_eq!(
        provider.fonts[provider.matching_font(&style, Some("A")).unwrap()].key,
        bold
    );
}

#[test]
fn first_available_font_for_metrics_must_cover_space() {
    let mut provider = TextProvider::new();
    let no_space = provider
        .register_web_font(WebFont {
            family: "A no space".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_metrics('A', None, 900),
        })
        .unwrap();
    let with_space = provider
        .register_web_font(WebFont {
            family: "B with space".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_metrics(' ', None, 600),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["'A no space'".into(), "B with space".into()],
        size: 16.0,
        ..TextStyle::default()
    };
    assert_eq!(
        provider.fonts[provider.matching_font(&style, None).unwrap()].key,
        no_space
    );
    let metrics = provider.metrics(&style).unwrap();
    let expected = font_metrics(&provider.fonts[1], style.size).unwrap();
    assert_eq!(
        provider.fonts[provider
            .matching_font_with_space_policy(&style, Some(" "), true)
            .unwrap()]
        .key,
        with_space
    );
    assert_eq!(metrics, expected);
    assert_eq!(provider.font_relative_metrics(&style).line_height_normal, expected.line_height());
    assert_ne!(
        metrics.ascent,
        font_metrics(&provider.fonts[0], style.size).unwrap().ascent
    );
}
#[test]
fn unicode_range_can_exclude_space_from_the_first_available_font() {
    let mut loader = FakeFontLoader::default();
    loader.locals.insert(
        "Space glyph".into(),
        synthetic_font_with_metrics(' ', None, 900),
    );
    let mut provider = TextProvider::new();
    provider
        .register_font_face(
            FontFaceDescriptor {
                family: "Excluded space".into(),
                sources: vec![FontFaceSource::Local("Space glyph".into())],
                unicode_ranges: vec![UnicodeRange {
                    start: 0x61,
                    end: 0x61,
                }],
                ..FontFaceDescriptor::default()
            },
            &mut loader,
        )
        .unwrap()
        .unwrap();
    provider
        .register_web_font(WebFont {
            family: "Included space".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font_with_metrics(' ', None, 600),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Excluded space".into(), "Included space".into()],
        size: 16.0,
        ..TextStyle::default()
    };
    assert!(!font_covers_space(&provider.fonts[0]));
    assert_eq!(
        provider.metrics(&style).unwrap().ascent,
        font_metrics(&provider.fonts[1], style.size).unwrap().ascent
    );
}

#[test]
fn private_use_ignores_generic_faces_but_uses_named_faces() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "serif".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{E000}'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["serif".into()],
        ..TextStyle::default()
    };
    assert_eq!(provider.matching_font(&style, Some("\u{E000}")), None);
    assert!(provider.matching_font(&style, Some("A")).is_none());
    let quoted = TextStyle {
        families: vec!["'serif'".into()],
        ..TextStyle::default()
    };
    assert!(provider.matching_font(&quoted, Some("\u{E000}")).is_some());
    provider
        .register_web_font(WebFont {
            family: "PUA fixture".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{E000}'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["serif".into(), "PUA fixture".into()],
        ..TextStyle::default()
    };
    assert!(provider.matching_font(&style, Some("\u{E000}")).is_some());
}

#[test]
fn default_ignorable_controls_stay_with_the_surrounding_face() {
    let mut provider = TextProvider::new();
    let primary = provider
        .register_web_font(WebFont {
            family: "Primary".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Primary".into()],
        size: 10.0,
        ..TextStyle::default()
    };
    let spans = [TextSpan {
        range: 0..4,
        source: 11,
        style,
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text: "A\u{200d}",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert!(!shaped.runs.is_empty());
    assert!(shaped.runs.iter().all(|run| run.font == primary));
}

#[test]
fn fallback_grapheme_is_not_split_at_a_dom_span_boundary() {
    let mut provider = TextProvider::new();
    provider
        .register_web_font(WebFont {
            family: "Primary".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('A'),
        })
        .unwrap();
    provider
        .register_web_font(WebFont {
            family: "Secondary".into(),
            weight: 400,
            stretch: 100,
            slant: FontSlant::Normal,
            format: FontFormat::OpenType,
            bytes: synthetic_font('\u{0301}'),
        })
        .unwrap();
    let style = TextStyle {
        families: vec!["Primary".into(), "Secondary".into()],
        size: 10.0,
        ..TextStyle::default()
    };
    let spans = [
        TextSpan {
            range: 0..1,
            source: 20,
            style: style.clone(),
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 1..3,
            source: 21,
            style,
            boundary_before: ShapingBoundary::Continue,
        },
    ];

    let shaped = provider
        .shape(Paragraph {
            text: "A\u{0301}",
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(shaped.runs.len(), 1);
    assert_eq!(shaped.runs[0].glyphs.len(), 1);
    assert_eq!(shaped.runs[0].glyphs[0].cluster, 0..3);
    assert!(shaped.runs.iter().all(|run| run.font == FontKey(0)));
}

#[test]
fn mixed_bidi_line_reorders_whole_grapheme_clusters() {
    let provider = TextProvider::new();
    let text = "abc אב\u{05b0}ג";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 30,
        style: TextStyle::default(),
        boundary_before: ShapingBoundary::Continue,
    }];

    let line = provider
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            },
            range: 0..text.len(),
        })
        .unwrap();

    assert_eq!(
        line.visual_clusters,
        vec![0..1, 1..2, 2..3, 3..4, 10..12, 6..10, 4..6]
    );
    let run_clusters = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|glyph| glyph.cluster.clone()))
        .collect::<Vec<_>>();
    assert_eq!(run_clusters, line.visual_clusters);
}

#[test]
fn break_all_adds_only_grapheme_cluster_opportunities() {
    let provider = TextProvider::new();
    let text = "A\u{0301}B";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 31,
        style: TextStyle {
            word_break: WordBreak::BreakAll,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert_eq!(
        shaped.breaks,
        vec![
            BreakOpportunity {
                offset: 3,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 4,
                kind: BreakKind::Mandatory,
            },
        ]
    );
}

#[test]
fn bidi_override_applies_css_direction_to_every_cluster() {
    let provider = TextProvider::new();
    let text = "abאב";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 32,
        style: TextStyle {
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::BidiOverride,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let line = provider
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            },
            range: 0..text.len(),
        })
        .unwrap();

    assert_eq!(line.visual_clusters, vec![4..6, 2..4, 1..2, 0..1]);
}

#[test]
fn manual_soft_hyphen_exposes_a_discretionary_break() {
    let provider = TextProvider::new();
    let text = "ab\u{00ad}cd";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 33,
        style: TextStyle {
            hyphens: Hyphens::Manual,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert!(shaped.breaks.contains(&BreakOpportunity {
        offset: 4,
        kind: BreakKind::Hyphen,
    }));
}

#[test]
fn line_break_anywhere_adds_typographic_character_boundaries() {
    let provider = TextProvider::new();
    let text = "A\u{0301}B";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 34,
        style: TextStyle {
            line_break: LineBreak::Anywhere,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert_eq!(
        shaped.breaks,
        vec![
            BreakOpportunity {
                offset: 3,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 4,
                kind: BreakKind::Mandatory,
            },
        ]
    );
}

#[test]
fn overflow_wrap_exposes_separate_emergency_cluster_breaks() {
    let provider = TextProvider::new();
    let text = "A\u{0301}B";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 35,
        style: TextStyle {
            overflow_wrap: OverflowWrap::BreakWord,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert_eq!(
        shaped.breaks,
        vec![
            BreakOpportunity {
                offset: 3,
                kind: BreakKind::Emergency,
            },
            BreakOpportunity {
                offset: 4,
                kind: BreakKind::Mandatory,
            },
        ]
    );
}

#[test]
fn keep_all_preserves_the_break_after_ideographic_space() {
    let provider = TextProvider::new();
    let text = "字字　字字";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 36,
        style: TextStyle {
            word_break: WordBreak::KeepAll,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];

    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert_eq!(
        shaped.breaks,
        vec![
            BreakOpportunity {
                offset: 9,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 15,
                kind: BreakKind::Mandatory,
            },
        ]
    );
}

#[test]
fn word_break_manual_suppresses_dictionary_segmented_breaks_like_keep_all() {
    // "กรุงเทพคือสวยงาม" (Bangkok is beautiful) has no spaces; ICU's
    // default (Normal) word segmentation still finds dictionary-based
    // word-internal break opportunities for Thai. CSS Text 4's
    // `word-break: manual` generalizes `keep-all` to exactly this case
    // and must suppress every one of them, leaving only the mandatory
    // break at the end of the text.
    let provider = TextProvider::new();
    let text = "กรุงเทพคือสวยงาม";

    let normal_spans = [TextSpan {
        range: 0..text.len(),
        source: 1,
        style: TextStyle::default(),
        boundary_before: ShapingBoundary::Continue,
    }];
    let normal = provider
        .shape(Paragraph {
            text,
            spans: &normal_spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert!(
        normal.breaks.len() > 1,
        "expected dictionary-segmented breaks under word-break: normal, got {:?}",
        normal.breaks
    );

    let manual_spans = [TextSpan {
        range: 0..text.len(),
        source: 1,
        style: TextStyle {
            word_break: WordBreak::Manual,
            ..TextStyle::default()
        },
        boundary_before: ShapingBoundary::Continue,
    }];
    let manual = provider
        .shape(Paragraph {
            text,
            spans: &manual_spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();
    assert_eq!(
        manual.breaks,
        vec![BreakOpportunity {
            offset: text.len(),
            kind: BreakKind::Mandatory,
        }]
    );
}

#[test]
fn css_override_remains_resolved_when_a_line_ends_inside_the_span() {
    let provider = TextProvider::new();
    let text = "> abcd > אבגד >";
    let outer = TextStyle::default();
    let inner = TextStyle {
        direction: Direction::RightToLeft,
        unicode_bidi: UnicodeBidi::BidiOverride,
        ..TextStyle::default()
    };
    let spans = [
        TextSpan {
            range: 0..2,
            source: 40,
            style: outer.clone(),
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 2..17,
            source: 41,
            style: inner,
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 17..19,
            source: 42,
            style: outer,
            boundary_before: ShapingBoundary::Continue,
        },
    ];

    let line = provider
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            },
            range: 0..8,
        })
        .unwrap();

    assert_eq!(
        line.visual_clusters,
        vec![0..1, 1..2, 7..8, 6..7, 5..6, 4..5, 3..4, 2..3]
    );
}

#[test]
fn ltr_paragraph_keeps_an_rtl_isolate_before_a_following_number() {
    let provider = TextProvider::new();
    let text = "א 3";
    let spans = [
        TextSpan {
            range: 0..2,
            source: 43,
            style: TextStyle {
                direction: Direction::RightToLeft,
                unicode_bidi: UnicodeBidi::Isolate,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 2..4,
            source: 44,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        },
    ];

    let line = provider
        .shape_line(LineRequest {
            paragraph: Paragraph {
                text,
                spans: &spans,
                sources: &[],
                bidi_scopes: &[],
                base_direction: Direction::LeftToRight,
            },
            range: 0..text.len(),
        })
        .unwrap();

    assert_eq!(line.visual_clusters, vec![0..2, 2..3, 3..4]);
}

#[test]
fn line_break_tailoring_follows_each_style_span() {
    let provider = TextProvider::new();
    let text = "AB CD";
    let spans = [
        TextSpan {
            range: 0..2,
            source: 45,
            style: TextStyle {
                word_break: WordBreak::BreakAll,
                ..TextStyle::default()
            },
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 2..5,
            source: 46,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::StyleChange,
        },
    ];
    let shaped = provider
        .shape(Paragraph {
            text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        })
        .unwrap();

    assert_eq!(
        shaped.breaks,
        vec![
            BreakOpportunity {
                offset: 1,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 3,
                kind: BreakKind::Allowed,
            },
            BreakOpportunity {
                offset: 5,
                kind: BreakKind::Mandatory,
            },
        ]
    );
}

#[test]
fn nested_bidi_scope_is_stable_across_descendant_style_spans() {
    let provider = TextProvider::new();
    let text = "א 3";
    let style = TextStyle::default();
    let single = [TextSpan {
        range: 0..text.len(),
        source: 47,
        style: style.clone(),
        boundary_before: ShapingBoundary::Continue,
    }];
    let split = [
        TextSpan {
            range: 0..2,
            source: 47,
            style: style.clone(),
            boundary_before: ShapingBoundary::Continue,
        },
        TextSpan {
            range: 2..text.len(),
            source: 48,
            style,
            boundary_before: ShapingBoundary::StyleChange,
        },
    ];
    let scopes = [amazing_text::BidiScope {
        range: 0..text.len(),
        direction: Direction::RightToLeft,
        unicode_bidi: UnicodeBidi::Isolate,
    }];
    let shape = |spans: &[TextSpan]| {
        provider
            .shape_line(LineRequest {
                paragraph: Paragraph {
                    text,
                    spans,
                    sources: &[],
                    bidi_scopes: &scopes,
                    base_direction: Direction::LeftToRight,
                },
                range: 0..text.len(),
            })
            .unwrap()
            .visual_clusters
    };

    assert_eq!(shape(&single), shape(&split));
}

#[test]
fn bidi_scope_closes_before_and_reopens_after_paragraph_break() {
    for (text, expected) in [
        ("א\nא", "\u{2067}א\u{2069}\n\u{2067}א\u{2069}"),
        ("א\r\nא", "\u{2067}א\u{2069}\r\n\u{2067}א\u{2069}"),
        ("א\u{001c}א", "\u{2067}א\u{2069}\u{001c}\u{2067}א\u{2069}"),
    ] {
        let spans = [TextSpan {
            range: 0..text.len(),
            source: 47,
            style: TextStyle::default(),
            boundary_before: ShapingBoundary::Continue,
        }];
        let scopes = [amazing_text::BidiScope {
            range: 0..text.len(),
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::Isolate,
        }];

        let analysis = bidi_analysis(text, &spans, &scopes).unwrap();

        assert_eq!(analysis.text, expected);
    }
}

#[test]
fn bidi_scope_does_not_reset_at_a_non_b_line_separator() {
    let text = "א\u{2028}א";
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 47,
        style: TextStyle::default(),
        boundary_before: ShapingBoundary::Continue,
    }];
    let scopes = [amazing_text::BidiScope {
        range: 0..text.len(),
        direction: Direction::RightToLeft,
        unicode_bidi: UnicodeBidi::Isolate,
    }];

    let analysis = bidi_analysis(text, &spans, &scopes).unwrap();

    assert_eq!(analysis.text, "\u{2067}א\u{2028}א\u{2069}");
}

#[test]
fn isolate_override_uses_first_strong_isolate_around_override() {
    assert_eq!(
        bidi_controls(Direction::LeftToRight, UnicodeBidi::IsolateOverride),
        ("\u{2068}\u{202d}", "\u{202c}\u{2069}")
    );
    assert_eq!(
        bidi_controls(Direction::RightToLeft, UnicodeBidi::IsolateOverride),
        ("\u{2068}\u{202e}", "\u{202c}\u{2069}")
    );
}

#[test]
fn bidi_scope_paragraph_break_cross_product_has_an_operation_cap() {
    let text = "\n".repeat(4097);
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 47,
        style: TextStyle::default(),
        boundary_before: ShapingBoundary::Continue,
    }];
    let scopes = (0..4097)
        .map(|_| amazing_text::BidiScope {
            range: 0..text.len(),
            direction: Direction::RightToLeft,
            unicode_bidi: UnicodeBidi::Isolate,
        })
        .collect::<Vec<_>>();

    assert!(matches!(
        bidi_analysis(&text, &spans, &scopes),
        Err(TextError::ResourceLimit)
    ));
}

#[test]
fn unsafe_breaks_include_internal_and_flagged_harf_edges() {
    let run = ShapedRun {
        text_range: 0..3,
        source: 1,
        font: FontKey(0),
        font_size: 16.0,
        direction: Direction::LeftToRight,
        writing_mode: WritingMode::HorizontalTb,
        metrics: FontMetrics {
            ascent: 10.0,
            descent: 3.0,
            leading: 0.0,
            x_height: 5.0,
            cap_height: 7.0,
            vertical_ascent: 8.0,
            vertical_descent: 8.0,
            vertical_leading: 0.0,
            has_canonical_vertical_metrics: false,
        },
        variations: Vec::new(),
        synthesis: amazing_text::FontSynthesis::default(),
        glyphs: vec![
            ShapedGlyph {
                id: GlyphId(1),
                cluster: 0..2,
                advance_x: 1.0,
                advance_y: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                orientation: amazing_text::GlyphOrientation::Upright,
                flags: GlyphFlags::default(),
            },
            ShapedGlyph {
                id: GlyphId(2),
                cluster: 2..3,
                advance_x: 1.0,
                advance_y: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                orientation: amazing_text::GlyphOrientation::Upright,
                flags: GlyphFlags {
                    unsafe_to_break: true,
                    unsafe_to_concat: false,
                },
            },
        ],
    };
    let breaks = [
        BreakOpportunity {
            offset: 1,
            kind: BreakKind::Allowed,
        },
        BreakOpportunity {
            offset: 2,
            kind: BreakKind::Allowed,
        },
    ];

    assert_eq!(unsafe_break_offsets(&[run], &breaks), vec![1, 2]);
}

#[test]
fn paragraph_cluster_limit_is_checked_before_itemized_output_allocation() {
    let provider = TextProvider::new();
    let text = "a".repeat(MAX_TEXT_CLUSTERS + 1);
    let spans = [TextSpan {
        range: 0..text.len(),
        source: 49,
        style: TextStyle::default(),
        boundary_before: ShapingBoundary::Continue,
    }];

    assert_eq!(
        provider.shape(Paragraph {
            text: &text,
            spans: &spans,
            sources: &[],
            bidi_scopes: &[],
            base_direction: Direction::LeftToRight,
        }),
        Err(TextError::ResourceLimit)
    );
}
