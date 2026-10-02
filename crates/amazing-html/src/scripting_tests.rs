use super::*;

#[test]
fn configured_scripting_mode_is_durable_across_all_parser_entry_points() {
    let source = "<body><noscript><b>x</b></noscript>";
    for mode in [ScriptingMode::Enabled, ScriptingMode::Disabled] {
        let expected = mode == ScriptingMode::Enabled;
        let full = parse_with_scripting_mode(source, mode);
        assert_eq!(full.document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
        assert_eq!(parse_bytes_with_scripting_mode(source.as_bytes(), None, mode)
            .document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
        for split in 0..=source.len() {
            let mut text = IncrementalParser::new();
            text.set_scripting_mode(mode);
            text.push_chunk(&source[..split]); text.push_chunk(&source[split..]);
            assert_eq!(text.finish().document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
            let mut bytes = IncrementalByteParser::new(None);
            bytes.set_scripting_mode(mode);
            bytes.push_chunk(&source.as_bytes()[..split]); bytes.push_chunk(&source.as_bytes()[split..]);
            assert_eq!(bytes.finish().document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
        }
        let mut parser = ResumableParser::new(Dom::new());
        parser.set_scripting_mode(mode);
        assert_eq!(parser.dom().document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
        parser.push_source(source);
        assert_eq!(parser.run(), ParseSignal::Eof);
        assert_eq!(parser.dom().document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
        for fragment in [
            parse_fragment_with_scripting_mode("noscript", Namespace::Html, "<b>x</b>", mode).unwrap(),
            parse_fragment_with_shadow_roots_and_scripting_mode("noscript", "<b>x</b>", mode).unwrap(),
        ] {
            assert_eq!(fragment.document_scripting_enabled(NodeIdx(0)).unwrap(), expected);
            assert_eq!(fragment.name(fragment.children(NodeIdx(0)).next().unwrap()),
                if expected { None } else { Some("b") });
        }
    }
    assert_eq!(parse(source).document_scripting_enabled(NodeIdx(0)).unwrap(), true);
    assert_eq!(parse_xml("<root/>").unwrap().document_scripting_enabled(NodeIdx(0)).unwrap(), false);
}

#[test]
fn disabled_noscript_handles_head_metadata_comments_and_body_markup() {
    let source = "<head><noscript><meta name=x><!--c--><style>a<b</style></noscript><body><noscript><b>x</b></noscript>";
    let dom = parse_with_scripting_mode(source, ScriptingMode::Disabled);
    assert_eq!(dom.dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n      <noscript>\n        <meta name=\"x\">\n        <!-- c -->\n        <style>\n          \"a<b\"\n    <body>\n      <noscript>\n        <b>\n          \"x\"\n");
    assert_eq!(parse(source).dump(NodeIdx(0)),
        parse_with_scripting_mode(source, ScriptingMode::Enabled).dump(NodeIdx(0)));
}

#[test]
fn disabled_noscript_reprocesses_body_tokens_after_preserving_leading_whitespace() {
    let source = "<head><noscript> \t<!--c--><div>x";
    assert_eq!(parse_with_scripting_mode(source, ScriptingMode::Disabled).dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n      <noscript>\n        \" \t\"\n        <!-- c -->\n    <body>\n      <div>\n        \"x\"\n");
}

#[test]
fn noscript_fragment_context_obeys_its_document_scripting_mode() {
    let source = "<b>x</b><!--c-->";
    assert_eq!(parse_fragment_with_scripting_mode("noscript", Namespace::Html, source, ScriptingMode::Disabled)
        .unwrap().dump(NodeIdx(0)), "#document\n  <b>\n    \"x\"\n  <!-- c -->\n");
    assert_eq!(parse_fragment_with_scripting_mode("noscript", Namespace::Html, source, ScriptingMode::Enabled)
        .unwrap().dump(NodeIdx(0)), "#document\n  \"<b>x</b><!--c-->\"\n");
}

#[test]
fn disabled_mode_survives_every_text_and_utf8_byte_split() {
    let source = "<head><noscript><!--é--><meta name=x></noscript><body><noscript><b>é</b></noscript>";
    let expected = parse_with_scripting_mode(source, ScriptingMode::Disabled).dump(NodeIdx(0));
    for split in source.char_indices().map(|(i, _)| i).chain(std::iter::once(source.len())) {
        let mut parser = IncrementalParser::new();
        parser.set_scripting_mode(ScriptingMode::Disabled);
        parser.push_chunk(&source[..split]);
        parser.push_chunk(&source[split..]);
        assert_eq!(parser.finish().dump(NodeIdx(0)), expected, "text split {split}");
    }
    let bytes = format!("\u{feff}{source}").into_bytes();
    assert_eq!(parse_bytes_with_scripting_mode(&bytes, None, ScriptingMode::Disabled).dump(NodeIdx(0)), expected);
    for split in 0..=bytes.len() {
        let mut parser = IncrementalByteParser::new(None);
        parser.set_scripting_mode(ScriptingMode::Disabled);
        parser.push_chunk(&bytes[..split]);
        parser.push_chunk(&bytes[split..]);
        assert_eq!(parser.finish().dump(NodeIdx(0)), expected, "byte split {split}");
    }
}

#[test]
fn disabled_resumable_parser_never_prepares_html_or_svg_scripts() {
    let source = "<script>1</script><script async src=x></script><script defer src=x></script><script type=module>2</script><svg><script/><script>3</script></svg>";
    let mut parser = ResumableParser::new(Dom::new());
    parser.set_scripting_mode(ScriptingMode::Disabled);
    parser.push_source(source);
    assert_eq!(parser.run(), ParseSignal::Eof);
    assert!(parser.take_deferred_or_async().is_empty());
    assert!(parser.dom().dump(NodeIdx(0)).contains("<svg script>"));
    let mut enabled = ResumableParser::new(Dom::new());
    enabled.push_source(source);
    assert!(matches!(enabled.run(), ParseSignal::Script(_)));
}
