//! Catalogue snapshots wrap the existing native outline/metrics fixture with
//! name records only; no production font aliases, outlines or metrics change.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) fn directory() -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "amazing-face-snapshot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    p
}
pub(super) fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut part = [0; 4];
        part[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(part))
    })
}
pub(super) fn font() -> Vec<u8> {
    let original = include_bytes!("../tests/fixtures/font-metrics-space.ttf");
    let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|i| {
            let row = &original[12 + i * 16..28 + i * 16];
            let offset = u32::from_be_bytes(row[8..12].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(row[12..16].try_into().unwrap()) as usize;
            (
                row[..4].try_into().unwrap(),
                original[offset..offset + length].to_vec(),
            )
        })
        .filter(|(tag, _)| tag != b"name")
        .collect();
    let names = [
        (1u16, "Snapshot Face"),
        (4, "Snapshot Face Regular"),
        (6, "SnapshotFace-Regular"),
    ];
    let mut name = [0u16.to_be_bytes(), 3u16.to_be_bytes(), 42u16.to_be_bytes()].concat();
    let mut strings = Vec::new();
    for (id, value) in names {
        let encoded = value
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        for value in [
            3u16,
            1,
            0x409,
            id,
            encoded.len() as u16,
            strings.len() as u16,
        ] {
            name.extend(value.to_be_bytes());
        }
        strings.extend(encoded);
    }
    name.extend(strings);
    tables.push((*b"name", name));
    tables.sort_by_key(|(tag, _)| *tag);
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    bytes[..4].copy_from_slice(&[0, 1, 0, 0]);
    bytes[4..6].copy_from_slice(&(tables.len() as u16).to_be_bytes());
    let selector = (usize::BITS - tables.len().leading_zeros() - 1) as u16;
    let search = (1u16 << selector) * 16;
    bytes[6..8].copy_from_slice(&search.to_be_bytes());
    bytes[8..10].copy_from_slice(&selector.to_be_bytes());
    bytes[10..12].copy_from_slice(&(tables.len() as u16 * 16 - search).to_be_bytes());
    let mut head = 0;
    for (i, (tag, mut table)) in tables.into_iter().enumerate() {
        if &tag == b"head" {
            table[8..12].fill(0);
            head = bytes.len();
        }
        let record = 12 + 16 * i;
        let offset = bytes.len();
        bytes[record..record + 4].copy_from_slice(&tag);
        bytes[record + 4..record + 8].copy_from_slice(&checksum(&table).to_be_bytes());
        bytes[record + 8..record + 12].copy_from_slice(&(offset as u32).to_be_bytes());
        bytes[record + 12..record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
        bytes.extend(table);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
    }
    assert_ne!(head, 0);
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&bytes));
    bytes[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    bytes
}
#[test]
fn duplicate_files_share_one_validated_snapshot_and_preserve_discovery_order() {
    let root = directory();
    let a = root.join("a.ttf");
    let b = root.join("b.ttf");
    let bytes = font();
    std::fs::write(&a, &bytes).unwrap();
    std::fs::write(&b, &bytes).unwrap();
    let catalogue = catalogue_from_paths(vec![b.clone(), a.clone(), a.clone()]);
    assert_eq!(catalogue.faces.len(), 2);
    assert_eq!(catalogue.faces[0].path, a);
    assert_eq!(catalogue.faces[1].path, b);
    let a = catalogue.faces[0].resource.as_ref().unwrap();
    let b = catalogue.faces[1].resource.as_ref().unwrap();
    assert_eq!(a.original_bytes().as_ptr(), b.original_bytes().as_ptr());
    assert!(std::ptr::eq(a.profile(), b.profile()));
    assert_eq!(a.profile().full_names, ["Snapshot Face Regular"]);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn immutable_catalogue_keeps_captured_face_after_removal_and_next_snapshot_observes_change() {
    let root = directory();
    let path = root.join("real.ttf");
    let bytes = font();
    std::fs::write(&path, &bytes).unwrap();
    let catalogue = catalogue_from_paths(vec![path.clone()]);
    assert_eq!(catalogue.faces.len(), 1);
    std::fs::write(&path, b"invalid replacement").unwrap();
    assert!(catalogue_from_paths(vec![path.clone()]).faces.is_empty());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        catalogue
            .local_face("snapshotface-regular")
            .unwrap()
            .original_bytes(),
        bytes
    );
    let mut provider = amazing_text_provider::TextProvider::new();
    register_catalogue_faces(&mut provider, &catalogue);
    assert!(
        amazing_text::TextSystem::metrics(
            &provider,
            &amazing_text::TextStyle {
                families: vec!["sans-serif".into()],
                ..Default::default()
            }
        )
        .is_ok()
    );
    let span = amazing_text::TextSpan {
        range: 0..1,
        source: 0,
        style: amazing_text::TextStyle {
            families: vec!["sans-serif".into()],
            ..Default::default()
        },
        boundary_before: amazing_text::ShapingBoundary::Continue,
    };
    let shaped = amazing_text::TextSystem::shape(
        &provider,
        amazing_text::Paragraph {
            text: " ",
            spans: &[span],
            sources: &[],
            bidi_scopes: &[],
            base_direction: amazing_text::Direction::LeftToRight,
        },
    )
    .unwrap();
    assert!(!shaped.runs.is_empty());
    assert!(
        shaped
            .runs
            .iter()
            .all(|run| run.font != amazing_text::FontKey(0))
    );
    assert!(catalogue_from_paths(vec![path]).faces.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn local_snapshot_reuses_real_names_and_respects_byte_caps_without_disk_or_url_fallback() {
    let root = directory();
    let path = root.join("real.ttf");
    let bytes = font();
    std::fs::write(&path, &bytes).unwrap();
    let catalogue = catalogue_from_paths(vec![path.clone()]);
    std::fs::remove_file(path).unwrap();
    let mut cache = std::collections::HashMap::new();
    let mut loader = PageFontLoader {
        installed_catalogue: Some(&catalogue),
        stylesheet_url: "about:blank",
        document_url: "about:blank",
        warnings: vec![],
        cache: &mut cache,
    };
    assert!(
        loader
            .load_local_face("SnapshotFace-Regular", bytes.len() - 1)
            .unwrap()
            .is_none()
    );
    let face = loader
        .load_local_face("SnapshotFace-Regular", bytes.len())
        .unwrap()
        .unwrap();
    assert_eq!(face.original_bytes(), bytes);
    assert_eq!(
        loader
            .load_local("Snapshot Face Regular", bytes.len())
            .unwrap()
            .unwrap(),
        bytes
    );
    assert!(
        loader
            .load_local("filename-alias", bytes.len())
            .unwrap()
            .is_none()
    );
    assert!(loader.cache.is_empty());
    assert!(loader.warnings.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
