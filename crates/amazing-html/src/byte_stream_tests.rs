use amazing_dom::NodeIdx;

use crate::{parse_bytes, IncrementalByteParser};

fn assert_splits(bytes: &[u8], label: Option<&str>) {
    let expected = parse_bytes(bytes, label).dump(NodeIdx(0));
    for split in 0..=bytes.len() {
        let mut parser = IncrementalByteParser::new(label);
        parser.push_chunk(&bytes[..split]);
        parser.push_chunk(&bytes[split..]);
        assert_eq!(parser.finish().dump(NodeIdx(0)), expected,
            "two chunks at {split}, label {label:?}, bytes {bytes:?}");
    }
    let mut parser = IncrementalByteParser::new(label);
    let mut start = 0;
    let mut step = 1;
    while start < bytes.len() {
        let end = (start + step).min(bytes.len());
        parser.push_chunk(&bytes[start..end]);
        start = end;
        step = step % 7 + 1;
    }
    assert_eq!(parser.finish().dump(NodeIdx(0)), expected,
        "many chunks, label {label:?}, bytes {bytes:?}");
}

#[test]
fn byte_boundaries_match_complete_decoding_and_html_recovery() {
    for (bytes, label) in [
        (&b"\xef\xbb\xbf<meta charset=windows-1252><p>\xc3\xa9\r\nx"[..], Some("utf-16be")),
        (&b"<meta charset=utf-8><p>\xf0\x9f\x98\x80&amp;\r\nx"[..], None),
        (&b"<meta content='text/html; charset=UTF-8' http-equiv='content-type'><textarea>&amp;\xc3\xa9</textarea>"[..], None),
        (&b"<!-- <meta charset=utf-8> --><meta charset=windows-1252><script>x < y</script><p>\x80"[..], None),
        (&b"<meta charset=utf-16><p>\xc3\xa9"[..], Some("unsupported-charset")),
        (&b"<p title='&ampx='>\x80&notin; &#65;<style>a{content:'</stylex>'}</style>"[..], None),
        (&b"<p>\xe2(\xa1 \xf0\x9f\x98"[..], Some("utf-8")),
        (&b"<p>\xc0\xaf \xe2\x82( \xf4\x90\x80\x80 \xed\xa0\x80"[..], Some("utf-8")),
        (&b"<div a='unfinished &amp"[..], Some("utf8")),
        (&b"\xff\xfe<\0p\0>\0=\xd8\0\xdc<\0/\0p\0>\0"[..], Some("utf-8")),
        (&b"\xfe\xff\0<\0p\0>\xd8=\xdc\0\0x\0<\0/\0p\0>"[..], None),
        (&b"\xff\xfe<\0p\0>\0=\xd8\0\xdc<\0/\0p\0>\0"[..], None),
        (&b"\xfe\xff\0<\0p\0>\xd8\0\xdc=\0<\0/\0p\0>"[..], Some("windows-1252")),
        (&b"<\0?\0x\0m\0l\0<\0p\0>\0"[..], None),
        (&b"<p>\x80\x81\x8d\x9f"[..], Some("latin1")),
    ] {
        assert_splits(bytes, label);
    }
}

#[test]
fn prescan_limit_and_incomplete_prefix_match_complete_input() {
    for prefix_len in [1010, 1023, 1024] {
        let mut bytes = vec![b' '; prefix_len];
        bytes.extend_from_slice(b"<meta charset=utf-8><p>\xc3\xa9");
        assert_splits(&bytes, None);
    }
    assert_splits(b"\xef\xbb", None);
    assert_splits(b"\xff", Some("utf-16le"));
    assert_splits(b"<!-- never closed <meta charset=utf-8>", None);
}

#[test]
fn consumed_byte_and_scalar_input_remains_bounded() {
    let mut parser = IncrementalByteParser::new(Some("utf-8"));
    for _ in 0..2000 {
        parser.push_chunk(b"<p>1234567890</p>");
        assert!(parser.buffered_input_bytes() <= 1024);
        assert!(parser.buffered_input_scalars() <= 12);
    }
    assert_eq!(parser.finish().dump(NodeIdx(0)).matches("<p>").count(), 2000);

    let mut parser = IncrementalByteParser::new(Some("utf-8"));
    let huge = "<p>1234567890</p>".repeat(2000);
    parser.push_chunk(huge.as_bytes());
    assert!(parser.buffered_input_bytes() <= 3);
    assert!(parser.buffered_input_scalars() <= 12);
    assert_eq!(parser.finish().dump(NodeIdx(0)).matches("<p>").count(), 2000);
}
