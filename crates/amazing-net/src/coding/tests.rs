use super::*;

fn fixtures() -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let mut input: &[u8] = include_bytes!("../../tests/coding/vectors.bin");
    fn number(input: &mut &[u8]) -> usize {
        let n = u32::from_le_bytes(input[..4].try_into().unwrap()) as usize;
        *input = &input[4..];
        n
    }
    fn bytes(input: &mut &[u8]) -> Vec<u8> {
        let len = number(input);
        let result = input[..len].to_vec();
        *input = &input[len..];
        result
    }
    let count = number(&mut input);
    (0..count)
        .map(|_| {
            (
                String::from_utf8(bytes(&mut input)).unwrap(),
                bytes(&mut input),
                bytes(&mut input),
            )
        })
        .collect()
}
pub(crate) fn fixture(name: &str) -> (Vec<u8>, Vec<u8>) {
    fixtures()
        .into_iter()
        .find(|c| c.0 == name)
        .map(|c| (c.1, c.2))
        .unwrap()
}
fn decompress(name: &str, data: &[u8], limit: usize) -> Result<Vec<u8>, NetError> {
    if name.ends_with("raw") {
        let mut output = Output::new(limit);
        let used = inflate(data, &mut output, 32768)?;
        assert_eq!(used, data.len(), "{name}");
        Ok(output.bytes)
    } else {
        decode(
            data,
            if name.ends_with("gzip") {
                "gzip"
            } else {
                "deflate"
            },
            limit,
        )
    }
}
#[test]
fn independent_encoder_corpus_covers_all_blocks_framings_and_window_history() {
    let cases = fixtures();
    assert_eq!(cases.len(), 57);
    for (name, plain, encoded) in cases {
        assert_eq!(
            decompress(&name, &encoded, plain.len()).unwrap(),
            plain,
            "{name}"
        );
    }
    let (_, dynamic) = fixture("dynamic-default-raw");
    assert_eq!(
        (dynamic[0] >> 1) & 3,
        2,
        "fixture must exercise dynamic Huffman"
    );
}
#[test]
fn all_fixture_prefix_truncations_fail_closed() {
    // Every prefix of representative stored/fixed/dynamic gzip/zlib streams.
    for name in [
        "hello-stored-gzip",
        "hello-fixed-gzip",
        "dynamic-default-gzip",
        "hello-stored-zlib",
        "hello-fixed-zlib",
        "dynamic-default-zlib",
    ] {
        let (plain, encoded) = fixture(name);
        for end in 0..encoded.len() {
            assert!(
                decompress(name, &encoded[..end], plain.len()).is_err(),
                "{name} prefix {end}"
            );
        }
    }
}
#[test]
fn exact_output_limits_and_zero_length_streams() {
    for (name, plain, encoded) in fixtures() {
        if !plain.is_empty() {
            assert!(
                matches!(
                    decompress(&name, &encoded, plain.len() - 1),
                    Err(NetError::ResponseTooLarge)
                ),
                "{name}"
            );
        }
    }
}
#[test]
fn gzip_checks_crc_isize_method_flags_and_garbage() {
    let (_, original) = fixture("hello-fixed-gzip");
    for index in [0, 2, original.len() - 8, original.len() - 4] {
        let mut bad = original.clone();
        bad[index] ^= 1;
        assert!(gzip(&bad, 1024).is_err());
    }
    let mut bad = original.clone();
    bad[3] |= 32;
    assert!(gzip(&bad, 1024).is_err());
    let mut bad = original;
    bad.push(0);
    assert!(gzip(&bad, 1024).is_err());
}
#[test]
fn gzip_optional_headers_and_header_crc() {
    let (plain, original) = fixture("hello-fixed-gzip");
    let mut header = original[..10].to_vec();
    header[3] = 0x1e;
    header.extend_from_slice(b"\x03\x00abcname\0comment\0");
    let checksum = crc32(&header) as u16;
    header.extend_from_slice(&checksum.to_le_bytes());
    let crc_pos = header.len() - 2;
    header.extend_from_slice(&original[10..]);
    assert_eq!(gzip(&header, plain.len()).unwrap(), plain);
    header[crc_pos] ^= 1;
    assert!(gzip(&header, 1024).is_err());
}
#[test]
fn gzip_concatenated_members_share_budget_but_not_history() {
    let (plain, first) = fixture("hello-fixed-gzip");
    let mut two = first.clone();
    two.extend_from_slice(&first);
    assert_eq!(gzip(&two, plain.len() * 2).unwrap(), plain.repeat(2));
    assert!(matches!(
        gzip(&two, plain.len() * 2 - 1),
        Err(NetError::ResponseTooLarge)
    ));
}
#[test]
fn zlib_checks_header_dictionary_adler_and_trailing_bytes() {
    let (_, original) = fixture("hello-fixed-zlib");
    for index in [0, 1, original.len() - 1] {
        let mut bad = original.clone();
        bad[index] ^= 1;
        assert!(zlib(&bad, 1024).is_err());
    }
    let mut bad = original.clone();
    bad[..2].copy_from_slice(&[0x78, 0x20]);
    assert!(zlib(&bad, 1024).is_err()); // valid FCHECK, unsupported FDICT
    let mut bad = original;
    bad.push(0);
    assert!(zlib(&bad, 1024).is_err());
    assert!(zlib(&fixture("hello-fixed-raw").1, 1024).is_err());
}
#[test]
fn checksum_known_answers() {
    assert_eq!(crc32(b"123456789"), 0xcbf43926);
    assert_eq!(adler32(b"Wikipedia"), 0x11e60398);
}
#[test]
fn malformed_block_types_and_stored_lengths() {
    for data in [&[7][..], &[1, 1, 0, 1, 0, 65][..], &[1, 1, 0, 254, 255][..]] {
        assert!(inflate(data, &mut Output::new(1024), 32768).is_err());
    }
    let mut out = Output::new(10);
    assert!(matches!(
        inflate(&[1, 255, 255, 0, 0], &mut out, 32768),
        Err(NetError::ResponseTooLarge)
    ));
    assert_eq!(out.bytes.capacity(), 0);
}

struct Writer {
    data: Vec<u8>,
    bit: usize,
}
impl Writer {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            bit: 0,
        }
    }
    fn bits(&mut self, n: usize, value: usize) {
        for i in 0..n {
            if self.bit % 8 == 0 {
                self.data.push(0);
            }
            let end = self.data.len() - 1;
            self.data[end] |= (((value >> i) & 1) as u8) << (self.bit % 8);
            self.bit += 1;
        }
    }
    fn code(&mut self, n: usize, value: usize) {
        for i in (0..n).rev() {
            self.bits(1, (value >> i) & 1);
        }
    }
}
#[test]
fn reserved_fixed_symbols_and_impossible_distance_fail() {
    for (length_bits, length_code, distance) in [(8, 198, None), (7, 1, Some(30)), (7, 1, Some(0))]
    {
        let mut w = Writer::new();
        w.bits(3, 3); // final fixed
        w.code(length_bits, length_code); // 286 reserved, or 257 length 3
        if let Some(d) = distance {
            w.code(5, d);
        }
        w.bits(16, 0);
        assert!(inflate(&w.data, &mut Output::new(1024), 32768).is_err());
    }
}
#[test]
fn dynamic_repeat_without_previous_and_repeat_overrun_fail() {
    for first in [true, false] {
        let mut w = Writer::new();
        w.bits(3, 5); // final dynamic
        w.bits(5, 0);
        w.bits(5, 0);
        w.bits(4, 0); // 257 literals, 1 distance, 4 CL lengths
                      // Complete CL tree: symbols 16 and 18, or 0 and 18.
        for len in if first { [1, 0, 1, 0] } else { [0, 0, 1, 1] } {
            w.bits(3, len);
        }
        if first {
            w.code(1, 0);
        } else {
            w.code(1, 1);
            w.bits(7, 127); // 138 zero lengths
            w.code(1, 1);
            w.bits(7, 127); // overruns 258 total
        }
        w.bits(16, 0);
        assert!(inflate(&w.data, &mut Output::new(1024), 32768).is_err());
    }
}
#[test]
fn malformed_byte_streams_never_panic_or_exceed_the_bound() {
    let mut seed = 0x19511952u32;
    for size in 0..256 {
        let mut data = vec![0; size];
        for byte in &mut data {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        for coding in ["gzip", "deflate"] {
            if let Ok(decoded) = decode(&data, coding, 128) {
                assert!(decoded.len() <= 128);
            }
        }
        let mut out = Output::new(128);
        let _ = inflate(&data, &mut out, 32768);
        assert!(out.bytes.len() <= 128);
        assert!(out.bytes.capacity() <= 128);
    }
}

#[test]
fn maximum_distance_can_reference_a_previous_stored_block() {
    let mut w = Writer::new();
    w.bits(3, 0);
    w.bits(5, 0); // nonfinal stored, align
    w.bits(16, 32768);
    w.bits(16, 32767);
    for i in 0..32768 {
        w.bits(8, i % 256);
    }
    w.bits(3, 3); // final fixed
    w.code(7, 1); // length 3
    w.code(5, 29);
    w.bits(13, 8191); // distance 32768
    w.code(7, 0); // end
    let mut output = Output::new(32771);
    inflate(&w.data, &mut output, 32768).unwrap();
    assert_eq!(&output.bytes[32768..], &[0, 1, 2]);
}
#[test]
fn dynamic_empty_distance_and_single_end_code_are_valid() {
    let mut w = Writer::new();
    w.bits(3, 5);
    w.bits(5, 0);
    w.bits(5, 0);
    w.bits(4, 14);
    for i in 0..18 {
        w.bits(3, if i == 3 || i == 17 { 1 } else { 0 });
    }
    for _ in 0..256 {
        w.code(1, 0);
    }
    w.code(1, 1); // literal 256 has length 1
    w.code(1, 0); // distance alphabet empty
    w.code(1, 0); // end-of-block
    let mut output = Output::new(0);
    inflate(&w.data, &mut output, 32768).unwrap();
    assert!(output.bytes.is_empty());
}
