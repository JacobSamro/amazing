//! Reproducible parser consistency fuzz. No clock, OS randomness, or external data.

use amazing_dom::NodeIdx;

use crate::{parse_bytes, IncrementalByteParser};

const SEED: u64 = 0x8a41_d72e_c09b_6153;
const CASES: usize = 256;
const MAX_GENERATED_BYTES: usize = 192;
const EVERY_SPLIT_CASES: usize = 24;

const PARTS: &[&[u8]] = &[
    b"<!doctype html>", b"<html><head>", b"<body>", b"</body></html>",
    b"<meta charset=utf-8>", b"<meta charset=windows-1252>",
    b"<meta http-equiv=content-type content='text/html; charset=utf-8'>",
    b"<!-- <meta charset=utf-16> -->", b"<div a='&ampx=' b=bare>",
    b"</div>", b"<p>", b"</p>", b"<table><tr><td>", b"</table>",
    b"<select><option>", b"<li><b>", b"</b></li>", b"<svg><desc>",
    b"<script>if (a < b) x = '&amp;';</script>", b"<style>a{content:'<x>'}</style>",
    b"<textarea>&amp; <b>literal</b></textarea>", b"<title>&#x1f600;</title>",
    b"&amp; &notin; &#65; &#x80; &ampx=", b"\r\n\rline\n", b"\xc3\xa9\xf0\x9f\x98\x80",
    b"\xe2(\xa1\xf0\x9f\x98", b"\x80\x81\x8d", b"<!--unfinished",
    b"<p a='unfinished &amp", b"<!DOCTYPE html PUB", b"<style>x</sty",
    b"</misnested><tr><td>x", b"<template><p>x</template>",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn index(&mut self, len: usize) -> usize { (self.next() as usize) % len }
}

fn generated_case(rng: &mut Rng, index: usize) -> (Vec<u8>, Option<&'static str>) {
    let mut bytes = Vec::new();
    for _ in 0..(2 + rng.index(7)) {
        let part = PARTS[rng.index(PARTS.len())];
        if bytes.len() + part.len() <= MAX_GENERATED_BYTES { bytes.extend_from_slice(part); }
    }
    for _ in 0..rng.index(4) {
        match rng.index(4) {
            0 if !bytes.is_empty() => { bytes.truncate(rng.index(bytes.len())); }
            1 if !bytes.is_empty() => {
                let at = rng.index(bytes.len());
                bytes[at] = [0, b'<', b'&', b'\r', 0x80, 0xc3, 0xff][rng.index(7)];
            }
            2 if !bytes.is_empty() => {
                let at = rng.index(bytes.len());
                let end = (at + 1 + rng.index(8)).min(bytes.len());
                bytes.drain(at..end);
            }
            _ if bytes.len() < MAX_GENERATED_BYTES => {
                let at = rng.index(bytes.len() + 1);
                bytes.insert(at, [b'>', b'\n', b'=', b'\'', 0xfe][rng.index(5)]);
            }
            _ => {}
        }
    }
    let label = [None, Some("utf-8"), Some("utf-16le"), Some("utf-16be"),
        Some("latin1"), Some("unsupported")][index % 6];
    // Exercise real UTF-16 code units and surrogate pairs, as well as mutated
    // ASCII-looking bytes interpreted as UTF-16 by a transport label.
    if index % 16 == 0 {
        bytes = "<p>😀\r\n&amp;</p>".encode_utf16()
            .flat_map(|unit| unit.to_le_bytes()).collect();
        if index % 32 == 0 { bytes.extend_from_slice(&[0x3d, 0xd8, 0x61]); }
        return (bytes, Some("utf-16le"));
    }
    if index % 16 == 1 {
        bytes = "<p>😀\r\n&amp;</p>".encode_utf16()
            .flat_map(|unit| unit.to_be_bytes()).collect();
        if index % 32 == 1 { bytes.extend_from_slice(&[0xd8, 0x3d, 0x61]); }
        return (bytes, Some("utf-16be"));
    }
    if index % 16 == 2 {
        bytes.truncate(MAX_GENERATED_BYTES - 3);
        bytes.splice(0..0, [0xef, 0xbb, 0xbf]);
    }
    if index % 16 == 3 || index % 16 == 4 {
        bytes.truncate(MAX_GENERATED_BYTES - 2);
        bytes.splice(0..0, if index % 16 == 3 { [0xff, 0xfe] } else { [0xfe, 0xff] });
    }
    (bytes, label)
}

fn streamed(bytes: &[u8], label: Option<&str>, sizes: &[usize]) -> String {
    let mut parser = IncrementalByteParser::new(label);
    let mut at = 0;
    let mut step = 0;
    while at < bytes.len() {
        let end = (at + sizes[step % sizes.len()]).min(bytes.len());
        parser.push_chunk(&bytes[at..end]);
        at = end;
        step += 1;
        let retained = parser.buffered_input_bytes();
        assert!(retained <= 1024, "undecoded input grew to {retained}");
        if at >= 1024 {
            assert!(retained <= 3, "post-sniff undecoded input grew to {retained}");
        }
        assert!(parser.buffered_input_scalars() <= bytes.len() + 1);
    }
    parser.push_chunk(&[]);
    parser.finish().dump(NodeIdx(0))
}

fn check(bytes: &[u8], label: Option<&str>, index: usize, every_split: bool) {
    let expected = parse_bytes(bytes, label).dump(NodeIdx(0));
    for sizes in [&[bytes.len().max(1)][..], &[1][..], &[2, 7, 3, 11][..], &[17, 5][..]] {
        assert_eq!(streamed(bytes, label, sizes), expected,
            "seed={SEED:#x} case={index} schedule={sizes:?} label={label:?} bytes={bytes:02x?}");
    }
    if every_split {
        for split in 0..=bytes.len() {
            let mut parser = IncrementalByteParser::new(label);
            parser.push_chunk(&bytes[..split]);
            parser.push_chunk(&bytes[split..]);
            assert_eq!(parser.finish().dump(NodeIdx(0)), expected,
                "seed={SEED:#x} case={index} split={split} label={label:?} bytes={bytes:02x?}");
        }
    }
}

#[test]
fn seeded_byte_grammar_and_mutations_match_all_stream_schedules() {
    let mut rng = Rng(SEED);
    for index in 0..CASES {
        let (bytes, label) = generated_case(&mut rng, index);
        assert!(bytes.len() <= MAX_GENERATED_BYTES);
        check(&bytes, label, index, index < EVERY_SPLIT_CASES);
    }
    // Cross the sniff limit with complete tokens so a whole-document byte
    // accumulator fails the retained-input assertion after every push.
    for index in 0..8 {
        let mut bytes = b"<meta charset=utf-8>".to_vec();
        bytes.extend_from_slice(b"<p>x</p>".repeat(140).as_slice());
        bytes.extend_from_slice(PARTS[index]);
        check(&bytes, None, CASES + index, false);
    }
}

#[test]
fn fixed_tree_oracles_are_independent_of_complete_stream_comparison() {
    for (bytes, label, tree) in [
        (&b"<p>x"[..], None, "#document\n  <html>\n    <head>\n    <body>\n      <p>\n        \"x\"\n"),
        (&b"<p>&amp;"[..], Some("utf-8"), "#document\n  <html>\n    <head>\n    <body>\n      <p>\n        \"&\"\n"),
        (&b"<p>a\r\nb"[..], Some("utf-8"), "#document\n  <html>\n    <head>\n    <body>\n      <p>\n        \"a\nb\"\n"),
    ] {
        assert_eq!(parse_bytes(bytes, label).dump(NodeIdx(0)), tree);
        assert_eq!(streamed(bytes, label, &[1]), tree);
    }
}
