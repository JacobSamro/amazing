use amazing_dom::NodeIdx;

use crate::parse_fragment;

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
    for context in ["script", "template", "noscript", "plaintext"] {
        assert_eq!(parse_fragment(context, "x").unwrap_err(),
            "unsupported HTML fragment context", "context={context}");
    }
}
