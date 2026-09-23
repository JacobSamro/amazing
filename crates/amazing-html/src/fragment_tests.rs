use amazing_dom::NodeIdx;

use crate::{parse, parse_fragment};
use crate::tokenizer::{Token, Tokenizer};

fn dump(context: &str, src: &str) -> String {
    parse_fragment(context, src).unwrap().dump(NodeIdx(0))
}

#[test]
fn virtual_table_family_contexts_keep_only_fragment_children() {
    assert_eq!(dump("table", "<table><tr>"),
        "#document\n  <tbody>\n    <tr>\n");
    assert_eq!(dump("tr", "</tr><td>"), "#document\n  <td>\n");
    assert_eq!(dump("tbody", "<tbody><a>"), "#document\n  <a>\n");
    assert_eq!(dump("colgroup", "<a><col>"), "#document\n  <col>\n");
    assert_eq!(dump("caption", "<table>"), "#document\n  <table>\n");
}

#[test]
fn html_fragment_keeps_comments_after_explicit_html_end() {
    assert_eq!(dump("html", "</html><!--abc-->"),
        "#document\n  <head>\n  <body>\n  <!-- abc -->\n");
    assert_eq!(dump("body", "<span><body>"), "#document\n  <span>\n");
    assert_eq!(dump("frameset", "<frame>"), "#document\n  <frame>\n");
}

#[test]
fn text_fragment_contexts_use_their_initial_tokenizer_states() {
    // Literal tree expectations were recorded before implementation.
    assert_eq!(dump("textarea", "a<b>&amp;\r\nz</textarea><i>"),
        "#document\n  \"a<b>&\nz</textarea><i>\"\n");
    assert_eq!(dump("title", "x&amp;<b>y</title>"),
        "#document\n  \"x&<b>y</title>\"\n");
    assert_eq!(dump("style", "a<b>&amp;\r\n</style><i>"),
        "#document\n  \"a<b>&amp;\n</style><i>\"\n");
}

#[test]
fn text_context_end_tags_and_malformed_eof_stay_in_fragment_text() {
    assert_eq!(dump("textarea", "\n&amp;<x"),
        "#document\n  \"\n&<x\"\n");
    assert_eq!(dump("style", "a &amp; <sty"),
        "#document\n  \"a &amp; <sty\"\n");
    assert_eq!(dump("title", "x</TiTlE><p>z"),
        "#document\n  \"x</TiTlE><p>z\"\n");
}

#[test]
fn select_and_option_contexts_have_independent_tree_oracles() {
    assert_eq!(dump("select", "<option>A<option>B</option><optgroup><option>C"),
        "#document\n  <option>\n    \"A\"\n  <option>\n    \"B\"\n  <optgroup>\n    <option>\n      \"C\"\n");
    assert_eq!(dump("option", "one</option><b>two</b>"),
        "#document\n  \"one\"\n  <b>\n    \"two\"\n");
}

#[test]
fn select_fragment_ignores_stray_context_end_and_disallowed_tags() {
    assert_eq!(dump("select", "</select><option>a</option><div>b</div>&amp;\r\nc"),
        "#document\n  <option>\n    \"a\"\n  \"b&\nc\"\n");
    assert_eq!(dump("option", "<option>x<option>y"),
        "#document\n  <option>\n    \"x\"\n  <option>\n    \"y\"\n");
}

#[test]
fn contexts_without_required_rules_remain_explicitly_unsupported() {
    assert_eq!(dump("template", "x"), "#document\n  \"x\"\n");
}

#[test]
fn plaintext_document_turns_all_remaining_markup_into_text() {
    assert_eq!(parse("<plaintext>a<b>&amp;</plaintext><p>z").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <plaintext>\n        \"a<b>&amp;</plaintext><p>z\"\n");
    assert_eq!(parse("<p>lead</p><plaintext>one</plaintext><b>two").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <p>\n        \"lead\"\n      <plaintext>\n        \"one</plaintext><b>two\"\n");
}

#[test]
fn plaintext_fragment_is_virtual_and_literal_through_eof() {
    assert_eq!(dump("plaintext", "a<b>&amp;\0\r\nc</plaintext><i>"),
        "#document\n  \"a<b>&amp;�\nc</plaintext><i>\"\n");
    assert_eq!(dump("plaintext", "x<"), "#document\n  \"x<\"\n");
    assert_eq!(dump("plaintext", ""), "#document\n");
}

#[test]
fn foreign_cdata_is_text_and_html_cdata_is_a_bogus_comment() {
    assert_eq!(parse("<svg><![CDATA[a<b>&amp;]]><circle/></svg>").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <svg svg>\n        \"a<b>&amp;\"\n        <svg circle>\n");
    assert_eq!(parse("<math><mtext><![CDATA[x<y &amp;]]></mtext></math>").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <math math>\n        <math mtext>\n          \"x<y &amp;\"\n");
    assert_eq!(parse("<![CDATA[html]]><p>z").dump(NodeIdx(0)),
        "#document\n  <!-- [CDATA[html]] -->\n  <html>\n    <head>\n    <body>\n      <p>\n        \"z\"\n");
}

#[test]
fn cdata_uses_adjusted_current_node_across_integration_points() {
    assert_eq!(parse("<svg><foreignObject><![CDATA[foreign]]><div><![CDATA[html]]></div></foreignObject></svg>").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <svg svg>\n        <svg foreignObject>\n          \"foreign\"\n          <div>\n            <!-- [CDATA[html]] -->\n");
    assert_eq!(parse("<math><mtext><![CDATA[foreign]]><b><![CDATA[html]]></b></mtext></math>").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <math math>\n        <math mtext>\n          \"foreign\"\n          <b>\n            <!-- [CDATA[html]] -->\n");
}

#[test]
fn foreign_cdata_null_and_unterminated_brackets_recover_as_text() {
    assert_eq!(parse("<svg><![CDATA[a\0b]").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <svg svg>\n        \"a�b]\"\n");
    assert_eq!(parse("<svg><![CDATA[a]]").dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      <svg svg>\n        \"a]]\"\n");
}

#[test]
fn script_fragment_uses_script_data_and_keeps_virtual_context_end_tag() {
    assert_eq!(dump("script", "a<b>&amp;"), "#document\n  \"a<b>&amp;\"\n");
    assert_eq!(dump("script", "x</script><i>y"), "#document\n  \"x</script><i>y\"\n");
    assert_eq!(dump("script", "a</ScRiPt >b</SCRIPTX>c"),
        "#document\n  \"a</ScRiPt >b</SCRIPTX>c\"\n");
    assert_eq!(dump("script", "a\0b\r\nc"), "#document\n  \"a�b\nc\"\n");
    assert_eq!(dump("script", "a</Sc\0ript>"), "#document\n  \"a</Sc�ript>\"\n");
}

#[test]
fn script_fragment_escape_and_double_escape_sequences() {
    assert_eq!(dump("script", "<!-- x --> y <b>"),
        "#document\n  \"<!-- x --> y <b>\"\n");
    assert_eq!(dump("script", "<!-- <script>inside</script> --> after"),
        "#document\n  \"<!-- <script>inside</script> --> after\"\n");
    for input in ["a<", "a</", "a</ScR", "<!-- <scr"] {
        assert_eq!(dump("script", input), format!("#document\n  {input:?}\n"));
    }
}

#[test]
fn noscript_fragment_uses_virtual_rawtext_context() {
    assert_eq!(dump("noscript", "a<b>&amp;"), "#document\n  \"a<b>&amp;\"\n");
    assert_eq!(dump("NoScRiPt", "x</NoScRiPt><i>y"),
        "#document\n  \"x</NoScRiPt><i>y\"\n");
    assert_eq!(dump("noscript", "x</noscripts>y</noscript >z"),
        "#document\n  \"x</noscripts>y</noscript >z\"\n");
    assert_eq!(dump("noscript", "a\0b\r\nc\rd"),
        "#document\n  \"a�b\nc\nd\"\n");
    assert_eq!(dump("noscript", "a</NoSc"), "#document\n  \"a</NoSc\"\n");
    assert_eq!(dump("noscript", ""), "#document\n");
    assert_eq!(dump("template", "x"), "#document\n  \"x\"\n");
}

fn streamed_noscript(chunks: &[&str]) -> String {
    let mut tokenizer = Tokenizer::streaming();
    tokenizer.enter_fragment_text_context("noscript");
    let mut text = String::new();
    for chunk in chunks {
        tokenizer.push_chars(chunk.chars());
        while let Some(token) = tokenizer.next_available() {
            match token {
                Token::Characters(chars) => text.push_str(&chars),
                unexpected => panic!("unexpected noscript fragment token: {unexpected:?}"),
            }
        }
    }
    tokenizer.finish_input();
    while let Some(token) = tokenizer.next_available() {
        match token {
            Token::Characters(chars) => text.push_str(&chars),
            Token::Eof => break,
            unexpected => panic!("unexpected noscript fragment token: {unexpected:?}"),
        }
    }
    text
}

#[test]
fn noscript_fragment_rawtext_survives_every_scalar_split() {
    for (input, expected) in [
        ("a<b>&amp;", "a<b>&amp;"),
        ("x</NoScRiPt><i>y", "x</NoScRiPt><i>y"),
        ("x</noscripts>y</noscript >z", "x</noscripts>y</noscript >z"),
        ("a\0b</NoSc", "a�b</NoSc"),
        ("𐐷é漢字</NoScRiPt>", "𐐷é漢字</NoScRiPt>"),
    ] {
        assert_eq!(streamed_noscript(&[input]), expected);
        for split in input.char_indices().map(|(offset, _)| offset).chain([input.len()]) {
            assert_eq!(streamed_noscript(&[&input[..split], &input[split..]]),
                expected, "split={split}, input={input:?}");
        }
    }
    assert_eq!(streamed_noscript(&["x</No", "Sc", "Ri", "Pt>", "<i>y"]),
        "x</NoScRiPt><i>y");
}
