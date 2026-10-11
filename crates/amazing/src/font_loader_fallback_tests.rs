//! Metadata-only wrapping of the original space font. Outlines/metrics stay genuine.
use super::installed_catalogue_tests::{checksum, directory, font};
use super::*;
fn native_font(family: &str, japanese: bool) -> Vec<u8> {
    let bytes = font();
    let n = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut tables = (0..n)
        .map(|i| {
            let row = &bytes[12 + i * 16..28 + i * 16];
            let tag: [u8; 4] = row[..4].try_into().unwrap();
            let start = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
            (tag, bytes[start..start + len].to_vec())
        })
        .collect::<Vec<_>>();
    let mut name = [0u16.to_be_bytes(), 3u16.to_be_bytes(), 42u16.to_be_bytes()].concat();
    let mut strings = Vec::new();
    for id in [1u16, 4, 6] {
        let value = family
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        for scalar in [3u16, 1, 0x409, id, value.len() as u16, strings.len() as u16] {
            name.extend(scalar.to_be_bytes());
        }
        strings.extend(value);
    }
    name.extend(strings);
    if japanese && !tables.iter().any(|(tag, _)| tag == b"OS/2") {
        // The base fixture has no OS/2 table; declare a minimal v4 one with a real weight.
        let mut os2 = vec![0u8; 96];
        os2[4..6].copy_from_slice(&400u16.to_be_bytes());
        tables.push((*b"OS/2", os2));
        tables.sort_by_key(|(tag, _)| *tag);
    }
    for (tag, value) in &mut tables {
        if tag == b"name" {
            *value = name.clone();
        }
        if tag == b"OS/2" && japanese {
            if value.len() < 96 {
                value.resize(96, 0);
            }
            value[..2].copy_from_slice(&4u16.to_be_bytes());
            let pages = u32::from_be_bytes(value[78..82].try_into().unwrap()) | (1 << 17);
            value[78..82].copy_from_slice(&pages.to_be_bytes());
        }
    }
    let mut output = vec![0; 12 + 16 * tables.len()];
    output[..12].copy_from_slice(&bytes[..12]);
    let count = tables.len() as u16;
    let selector = u16::BITS as u16 - 1 - count.leading_zeros() as u16;
    output[4..6].copy_from_slice(&count.to_be_bytes());
    output[6..8].copy_from_slice(&((1u16 << selector) * 16).to_be_bytes());
    output[8..10].copy_from_slice(&selector.to_be_bytes());
    output[10..12].copy_from_slice(&(count * 16 - (1u16 << selector) * 16).to_be_bytes());
    let mut head = None;
    for (i, (tag, mut value)) in tables.into_iter().enumerate() {
        if &tag == b"head" {
            value[8..12].fill(0);
            head = Some(output.len());
        }
        let row = 12 + i * 16;
        let offset = output.len();
        output[row..row + 4].copy_from_slice(&tag);
        output[row + 4..row + 8].copy_from_slice(&checksum(&value).to_be_bytes());
        output[row + 8..row + 12].copy_from_slice(&(offset as u32).to_be_bytes());
        output[row + 12..row + 16].copy_from_slice(&(value.len() as u32).to_be_bytes());
        output.extend(value);
        while output.len() % 4 != 0 {
            output.push(0);
        }
    }
    let head = head.unwrap();
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&output));
    output[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    output
}
#[test]
fn unused_native_faces_register_with_only_real_names_and_duplicates_share_one_key() {
    let root = directory();
    let a = root.join("a.ttf");
    let b = root.join("b.ttf");
    let c = root.join("c.ttf");
    std::fs::write(&a, native_font("Primary Face", false)).unwrap();
    let unused = native_font("Supplement Face", true);
    std::fs::write(&b, &unused).unwrap();
    std::fs::write(&c, &unused).unwrap();
    let catalogue = catalogue_from_paths(vec![c, b, a]);
    let mut provider = amazing_text_provider::TextProvider::new();
    register_catalogue_faces(&mut provider, &catalogue);
    let records = (1..16)
        .filter_map(|key| provider.registered_font_face(amazing_text::FontKey(key)))
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].native.unwrap().families, ["Primary Face"]);
    assert_eq!(records[1].native.unwrap().families, ["Supplement Face"]);
    assert_eq!(records[1].family, "supplement face");
    assert!(records[1].native.unwrap().declares_japanese_support());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn added_fallback_preserves_original_selected_record_and_local_physical_identity() {
    let root = directory();
    let a = root.join("a.ttf");
    let b = root.join("b.ttf");
    std::fs::write(&a, native_font("Primary Face", false)).unwrap();
    std::fs::write(&b, native_font("Supplement Face", true)).unwrap();
    let catalogue = catalogue_from_paths(vec![b.clone(), a.clone()]);
    let primary = catalogue_from_paths(vec![a]);
    let mut before = amazing_text_provider::TextProvider::new();
    register_catalogue_faces(&mut before, &primary);
    let mut after = amazing_text_provider::TextProvider::new();
    register_catalogue_faces(&mut after, &catalogue);
    let key = amazing_text::FontKey(1);
    assert_eq!(
        before.registered_font_face(key).unwrap().family,
        after.registered_font_face(key).unwrap().family
    );
    assert_eq!(
        before.registered_font_face(key).unwrap().native,
        after.registered_font_face(key).unwrap().native
    );
    std::fs::remove_file(b).unwrap();
    assert_eq!(
        catalogue
            .local_face("Supplement Face")
            .unwrap()
            .profile()
            .families,
        ["Supplement Face"]
    );
    assert!(
        after
            .registered_font_face(amazing_text::FontKey(0))
            .is_none()
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn genuine_decoder_limits_do_not_admit_fictional_large_fallback_faces() {
    let root = directory();
    let path = root.join("large.ttf");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(super::super::MAX_PAGE_FONT_BYTES as u64 + 1)
        .unwrap();
    drop(file);
    let catalogue = catalogue_from_paths(vec![path]);
    assert!(catalogue.faces.is_empty());
    let mut provider = amazing_text_provider::TextProvider::new();
    register_catalogue_faces(&mut provider, &catalogue);
    assert!(
        provider
            .registered_font_face(amazing_text::FontKey(1))
            .is_none()
    );
    std::fs::remove_dir_all(root).unwrap();
}
