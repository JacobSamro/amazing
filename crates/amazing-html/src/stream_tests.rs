use crate::{parse, IncrementalParser};
use crate::tokenizer::{Token, Tokenizer};
use amazing_dom::{NodeIdx, NodeKind};

#[test]
fn doctype_identifiers_survive_character_chunks() {
    let input = "<!DOCTYPE Html PUBLIC 'p' \"s\"><p>x";
    let mut tokenizer = Tokenizer::streaming();
    let mut tokens = Vec::new();
    let mut parser = IncrementalParser::new();
    for scalar in input.chars() {
        tokenizer.push_chars([scalar]);
        while let Some(token) = tokenizer.next_available() { tokens.push(token); }
        parser.push_chunk(&scalar.to_string());
    }
    tokenizer.finish_input();
    while let Some(token) = tokenizer.next_available() { tokens.push(token); }
    assert_eq!(tokens[0], Token::Doctype {
        name: Some("html".into()), public_id: Some("p".into()),
        system_id: Some("s".into()), force_quirks: false,
    });
    let dom = parser.finish();
    let doctype = dom.document_doctype(NodeIdx(0)).unwrap().unwrap();
    assert_eq!(dom.get(doctype).kind, NodeKind::DocumentType {
        name: "html".into(), public_id: "p".into(), system_id: "s".into(),
    });
    assert_eq!(dom.dump(NodeIdx(0)), parse(input).dump(NodeIdx(0)));
}

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
fn malformed_table_and_tail_tokens_keep_chunk_equivalence() {
    for src in [
        "<table><tbody><tbody><tbody>",
        "<table><tr><td>x</tr><td>y</table>",
        "</html><!--abc--><table><tr>",
        "<colgroup><col><a></colgroup>",
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
fn cdata_namespace_and_closing_delimiter_survive_every_scalar_split() {
    for src in [
        "<svg><![CDATA[a<b>&amp;]]><circle/></svg>",
        "<math><mtext><![CDATA[x<y &amp;]]></mtext></math>",
        "<![CDATA[html]]><p>z",
        "<svg><foreignObject><![CDATA[foreign]]><div><![CDATA[html]]></div></foreignObject></svg>",
        "<math><mtext><![CDATA[foreign]]><b><![CDATA[html]]></b></mtext></math>",
        "<svg><![CDATA[a\0b]",
        "<svg><![CDATA[a]]",
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
fn pending_table_text_places_the_complete_character_run() {
    for (input, expected) in [
        ("<table> A<tr><td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      \" A\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
        ("<table> \n<tr><td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      <table>\n        \" \n\"\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
        ("<table>&#32;A<tr><td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      \" A\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
        ("<table><tbody> \nA<tr><td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      \" \nA\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
        ("<table><tr> A<td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      \" A\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
        ("<table><tr><td><table> B</table>C",
         "#document\n  <html>\n    <head>\n    <body>\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \" B\"\n              <table>\n              \"C\"\n"),
        ("<table> A",
         "#document\n  <html>\n    <head>\n    <body>\n      \" A\"\n      <table>\n"),
        ("<body>lead<table> A<tr><td>x",
         "#document\n  <html>\n    <head>\n    <body>\n      \"lead A\"\n      <table>\n        <tbody>\n          <tr>\n            <td>\n              \"x\"\n"),
    ] {
        assert_eq!(parse(input).dump(NodeIdx(0)), expected, "one-shot: {input:?}");
        assert_all_splits(input);
    }
}

#[test]
fn pending_table_text_keeps_reference_and_scalar_chunk_order() {
    let expected = parse("<table>&#32;A<tr><td>x").dump(NodeIdx(0));
    assert_eq!(streamed(&["<table>&", "#", "3", "2", ";", "A<tr><td>x"]), expected);
    assert_eq!(streamed(&["<table>", " ", "𐐷", "é", "<tr><td>x"]),
        parse("<table> 𐐷é<tr><td>x").dump(NodeIdx(0)));
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
