use amazing_dom::NodeIdx;

use crate::{parse, parse_fragment, IncrementalParser};

#[test]
fn non_whitespace_after_html_resumes_in_body_for_following_comments() {
    let expected = "#document\n  <html>\n    <head>\n    <body>\n      \"x\"\n      <!--  inside  -->\n  <!--  outside  -->\n";
    let source = "<html><body></body></html>x<!-- inside --></html><!-- outside -->";
    assert_eq!(parse(source).dump(NodeIdx(0)), expected);

    let after_body = "<html><body></body>x<!-- inside -->";
    assert_eq!(parse(after_body).dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      \"x\"\n      <!--  inside  -->\n");
}

#[test]
fn whitespace_and_comments_keep_their_tail_modes() {
    let source = "<html><body></body></html> \n<!-- outside -->";
    assert_eq!(parse(source).dump(NodeIdx(0)),
        "#document\n  <html>\n    <head>\n    <body>\n      \" \n\"\n  <!--  outside  -->\n");

    let fragment = parse_fragment("div", "</body></html>x<!-- inside -->").unwrap();
    assert_eq!(fragment.dump(NodeIdx(0)),
        "#document\n  \"x\"\n  <!--  inside  -->\n");
}

#[test]
fn tail_recovery_is_invariant_across_chunk_boundaries() {
    let source = "<html><body></body></html>x<!-- inside --></html><!-- outside -->";
    let expected = parse(source).dump(NodeIdx(0));
    for split in source.char_indices().map(|(i, _)| i).chain([source.len()]) {
        let mut parser = IncrementalParser::new();
        parser.push_chunk(&source[..split]);
        parser.push_chunk(&source[split..]);
        assert_eq!(parser.finish().dump(NodeIdx(0)), expected, "split at {split}");
    }
    let mut parser = IncrementalParser::new();
    for chunk in ["<html><bo", "dy></body></ht", "ml>x<!-- ins", "ide -->", "</html><!-- outside -->"] {
        parser.push_chunk(chunk);
    }
    assert_eq!(parser.finish().dump(NodeIdx(0)), expected);
}
