use crate::{parse, IncrementalParser};
use crate::tokenizer::{Token, Tokenizer};
use amazing_dom::NodeIdx;

fn streamed(chunks: &[&str]) -> String {
    let mut parser = IncrementalParser::new();
    for chunk in chunks { parser.push_chunk(chunk); }
    parser.finish().dump(NodeIdx(0))
}

fn assert_all_splits(src: &str) {
    let expected = parse(src).dump(NodeIdx(0));
    assert_eq!(streamed(&[src]), expected, "one chunk: {src:?}");
    for split in src.char_indices().map(|(i, _)| i).chain([src.len()]) {
        assert_eq!(streamed(&[&src[..split], &src[split..]]), expected,
            "split at byte {split}: {src:?}");
    }
    let boundaries: Vec<usize> = src.char_indices().map(|(i, _)| i).chain([src.len()]).collect();
    let mut parser = IncrementalParser::new();
    let mut pos = 0;
    let mut step = 1;
    while pos + 1 < boundaries.len() {
        let end = (pos + step).min(boundaries.len() - 1);
        parser.push_chunk(&src[boundaries[pos]..boundaries[end]]);
        pos = end;
        step = step % 5 + 1;
    }
    assert_eq!(parser.finish().dump(NodeIdx(0)), expected, "multi-chunk: {src:?}");
}

#[test]
fn chunks_are_invariant_across_html_lexical_boundaries() {
    for src in [
        "<!doctype html><div class='a b' data-x=unquoted>text</div>",
        "<!--comment--><p>A<!--split-->B</p>",
        "<!DOCTYPE html PUBLIC '-//W3C//DTD HTML 4.01//EN' 'about:legacy-compat'><p>x",
        "<p title='&amp; &#x41; &ampx='>&notin; &#65; &amp;</p>",
        "<style>a{content:'</styLEz>'}</style><p>x",
        "<textarea>&amp; <b>literal</b></textarea><title>&#x1F600;</title>",
        "<script>let x = '</scr' + 'ipt>';</script><p>end",
        "<table><tr><td><b>x</table>after",
        "<p>line\r\nnext\rend\n𐐷é漢字</p>",
        "<div a='unfinished &amp",
        "<p>x<!--unfinished-",
        "<!DOCTYPE html PUB",
        "<style>x</sty",
        "<p>text &not",
        "<p>text <",
    ] {
        assert_all_splits(src);
    }
}

#[test]
fn plaintext_document_is_invariant_across_every_scalar_boundary() {
    for src in [
        "<plaintext>a<b>&amp;</plaintext><p>z",
        "<p>lead</p><plaintext>one</plaintext><b>two",
        "<plaintext>a\0b\r\nc<",
    ] { assert_all_splits(src); }
}

#[test]
fn script_data_is_invariant_across_every_scalar_boundary() {
    for src in [
        "<script><!-- x --> y </script><p>z",
        "<script><!-- <script>inner</script> --> tail </ScRiPt><p>z",
        "<script><!-- <ScRiPt/inner</SCRIPT/ --> tail </script>",
        "<script>a</ScR",
        "<script><!-- <scr",
        "<script>a\0b</script>",
    ] { assert_all_splits(src); }
}

#[test]
fn temporary_exhaustion_does_not_resolve_ambiguous_tokens() {
    let mut tok = Tokenizer::streaming();
    tok.push_chars("<meta char".chars());
    assert_eq!(tok.next_available(), None);
    tok.push_chars("set=utf-8>".chars());
    assert!(matches!(tok.next_available(), Some(Token::StartTag { name, .. }) if name == "meta"));
    assert_eq!(tok.next_available(), None);

    let mut reference = Tokenizer::streaming();
    reference.push_chars("&amp".chars());
    assert_eq!(reference.next_available(), None);
    reference.push_chars(";".chars());
    assert_eq!(reference.next_available(), Some(Token::Characters("&".into())));
    assert_eq!(reference.next_available(), None);

    let mut doctype = Tokenizer::streaming();
    doctype.push_chars("<!doct".chars());
    assert_eq!(doctype.next_available(), None);
    doctype.push_chars("ype html>".chars());
    assert!(matches!(doctype.next_available(), Some(Token::Doctype { name: Some(name), .. }) if name == "html"));

    let mut comment = Tokenizer::streaming();
    comment.push_chars("<!--unfinished".chars());
    assert_eq!(comment.next_available(), None);
    comment.push_chars("-->".chars());
    assert_eq!(comment.next_available(), Some(Token::Comment("unfinished".into())));
}

#[test]
fn final_eof_is_emitted_once() {
    let mut tok = Tokenizer::streaming();
    tok.push_chars("<p>x".chars());
    assert!(matches!(tok.next_available(), Some(Token::StartTag { .. })));
    assert_eq!(tok.next_available(), Some(Token::Characters("x".into())));
    assert_eq!(tok.next_available(), None);
    tok.finish_input();
    assert_eq!(tok.next_available(), Some(Token::Eof));
    assert_eq!(tok.next_available(), None);
}

#[test]
fn consumed_input_does_not_accumulate() {
    let mut parser = IncrementalParser::new();
    for _ in 0..2000 {
        parser.push_chunk("<p>1234567890</p>");
        assert!(parser.buffered_input_scalars() <= 12);
    }
    let dom = parser.finish();
    let dump = dom.dump(NodeIdx(0));
    assert_eq!(dump.matches("<p>").count(), 2000);
}
