use amazing_dom::{Dom, Namespace, NodeIdx, QuirksMode};
use crate::{parse, parse_bytes, IncrementalParser, IncrementalByteParser, ResumableParser, ParseSignal};

const CASES: &[(&str, QuirksMode)] = &[
    ("<!doctype html><p>x", QuirksMode::NoQuirks),
    ("<!doctype HTML SYSTEM 'about:legacy-compat'><p>x", QuirksMode::NoQuirks),
    ("<!doctype html PUBLIC '-//W3C//DTD XHTML 1.0 Transitional//EN' 'x'><p>x", QuirksMode::LimitedQuirks),
    ("<!doctype html PUBLIC '-//W3C//DTD HTML 4.01 Transitional//EN' 'x'><p>x", QuirksMode::LimitedQuirks),
    ("<!doctype html PUBLIC '-//W3C//DTD HTML 4.01 Transitional//EN'><p>x", QuirksMode::Quirks),
    ("<!doctype other><p>x", QuirksMode::Quirks),
    ("<!-- lead --> \n<p>x", QuirksMode::Quirks),
    ("", QuirksMode::Quirks),
];

#[test]
fn complete_documents_preserve_all_modes_and_clone_them() {
    for &(source, mode) in CASES {
        let mut dom = parse(source);
        assert_eq!(dom.document_quirks_mode(NodeIdx(0)).unwrap(), mode, "{source}");
        assert_eq!(parse_bytes(source.as_bytes(), None).document_quirks_mode(NodeIdx(0)).unwrap(), mode);
        let document = dom.clone_node(NodeIdx(0), true).unwrap();
        assert_eq!(dom.document_quirks_mode(document).unwrap(), mode);
    }
}

#[test]
fn every_text_and_byte_split_preserves_document_mode() {
    for &(source, mode) in CASES {
        for split in 0..=source.len() {
            let mut text = IncrementalParser::new();
            text.push_chunk(&source[..split]);
            text.push_chunk(&source[split..]);
            assert_eq!(text.finish().document_quirks_mode(NodeIdx(0)).unwrap(), mode);
            let mut bytes = IncrementalByteParser::new(None);
            bytes.push_chunk(&source.as_bytes()[..split]);
            bytes.push_chunk(&source.as_bytes()[split..]);
            assert_eq!(bytes.finish().document_quirks_mode(NodeIdx(0)).unwrap(), mode);
        }
    }
}

#[test]
fn scripts_observe_selected_mode_before_the_parser_pause() {
    for &(prefix, mode) in CASES {
        let mut parser = ResumableParser::new(Dom::new());
        parser.push_source(&format!("{prefix}<script>mode</script><p>after"));
        assert!(matches!(parser.run(), ParseSignal::Script(_)));
        assert_eq!(parser.dom().document_quirks_mode(NodeIdx(0)).unwrap(), mode);
        assert_eq!(parser.run(), ParseSignal::Eof);
        assert_eq!(parser.dom().document_quirks_mode(NodeIdx(0)).unwrap(), mode);
    }
}

#[test]
fn srcdoc_and_fragment_documents_keep_no_quirks() {
    for &(source, _) in CASES {
        let mut parser = ResumableParser::new(Dom::new());
        parser.set_srcdoc();
        parser.push_source(source);
        assert_eq!(parser.run(), ParseSignal::Eof);
        assert_eq!(parser.dom().document_quirks_mode(NodeIdx(0)).unwrap(), QuirksMode::NoQuirks);
    }
    let fragment = crate::parse_fragment_with_namespace("div", Namespace::Html, "<p>x").unwrap();
    assert_eq!(fragment.document_quirks_mode(NodeIdx(0)).unwrap(), QuirksMode::NoQuirks);
}
