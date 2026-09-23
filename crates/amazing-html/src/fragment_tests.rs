use amazing_dom::NodeIdx;

use crate::{parse, parse_fragment};

fn dump(context: &str, src: &str) -> String {
    parse_fragment(context, src).unwrap().dump(NodeIdx(0))
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
    for context in ["template", "noscript"] {
        assert_eq!(parse_fragment(context, "x").unwrap_err(),
            "unsupported HTML fragment context", "context={context}");
    }
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
