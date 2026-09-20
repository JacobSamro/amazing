use super::*;
fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .collect::<String>()
        .as_bytes()
        .chunks_exact(2)
        .map(|x| u8::from_str_radix(std::str::from_utf8(x).unwrap(), 16).unwrap())
        .collect()
}
fn fields(pairs: &[(&str, &str)]) -> Vec<Field> {
    pairs
        .iter()
        .map(|(n, v)| (n.as_bytes().to_vec(), v.as_bytes().to_vec()))
        .collect()
}
#[test]
fn rfc7541_appendix_c_request_sequences_plain_and_huffman() {
    for sequence in [
        [
            "828684410f7777772e6578616d706c652e636f6d",
            "828684be58086e6f2d6361636865",
            "828785bf400a637573746f6d2d6b65790c637573746f6d2d76616c7565",
        ],
        [
            "828684418cf1e3c2e5f23a6ba0ab90f4ff",
            "828684be5886a8eb10649cbf",
            "828785bf408825a849e95ba97d7f8925a849e95bb8e8b4bf",
        ],
    ] {
        let mut d = Decoder::new(4096);
        let expected = [
            fields(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
            ]),
            fields(&[
                (":method", "GET"),
                (":scheme", "http"),
                (":path", "/"),
                (":authority", "www.example.com"),
                ("cache-control", "no-cache"),
            ]),
            fields(&[
                (":method", "GET"),
                (":scheme", "https"),
                (":path", "/index.html"),
                (":authority", "www.example.com"),
                ("custom-key", "custom-value"),
            ]),
        ];
        for (encoded, wanted) in sequence.into_iter().zip(expected) {
            assert_eq!(d.decode(&hex(encoded), 4096).unwrap(), wanted);
        }
        assert_eq!(d.table.size, 164);
        assert_eq!(d.table.entries.len(), 3);
    }
}
#[test]
fn rfc7541_response_huffman_sequence_eviction_at_256_bytes() {
    let blocks=["488264025885aec3771a4b6196d07abe941054d444a8200595040b8166e082a62d1bff6e919d29ad171863c78f0b97c8e9ae82ae43d3",
        "4883640effc1c0bf", "88c16196d07abe941054d444a8200595040b8166e084a62d1bffc05a839bd9ab77ad94e7821dd7f2e6c7b335dfdfcd5b3960d5af27087f3672c1ab270fb5291f9587316065c003ed4ee5b1063d5007"];
    let mut d = Decoder::new(256);
    for (index, block) in blocks.into_iter().enumerate() {
        let fields = d.decode(&hex(block), 4096).unwrap();
        assert_eq!(fields[0].1, [b"302".as_slice(), b"307", b"200"][index]);
        assert!(fields.contains(&(b"cache-control".to_vec(), b"private".to_vec())));
        assert!(fields.contains(&(b"location".to_vec(), b"https://www.example.com".to_vec())));
        if index == 2 {
            assert!(fields.contains(&(
                b"set-cookie".to_vec(),
                b"foo=ASDJKHQKBZXOQWEOPIUAXQWEOIU; max-age=3600; version=1".to_vec()
            )));
        }
        assert!(d.table.size <= 256);
    }
}
#[test]
fn integers_literal_modes_and_invalid_indices() {
    assert_eq!(integer(&hex("1f9a0a"), &mut 0, 5).unwrap(), 1337);
    assert_eq!(integer(&[42], &mut 0, 8).unwrap(), 42);
    for tag in [0x00, 0x10, 0x40] {
        let bytes = [tag, 1, b'x', 1, b'y'];
        let mut d = Decoder::new(4096);
        assert_eq!(d.decode(&bytes, 34).unwrap(), fields(&[("x", "y")]));
        assert_eq!(d.table.entries.len(), usize::from(tag == 0x40));
    }
    for b in [hex("80"), hex("be"), hex("ff"), vec![255; 32]] {
        assert!(matches!(
            Decoder::new(4096).decode(&b, 4096),
            Err(Error::Compression)
        ));
    }
}
#[test]
fn huffman_padding_eos_truncation_and_decoded_limit() {
    assert_eq!(
        huffman(&hex("f1e3c2e5f23a6ba0ab90f4ff"), 15).unwrap(),
        b"www.example.com"
    );
    assert!(matches!(
        huffman(&hex("f1e3c2e5f23a6ba0ab90f4ff"), 14),
        Err(Error::Limit)
    ));
    for bytes in [
        hex("ffffffff"),
        hex("ff"),
        hex("1e"),
        hex("f1e3c2e5f23a6ba0ab90f4fe"),
    ] {
        assert!(huffman(&bytes, 1024).is_err(), "{bytes:?}");
    }
    let literal = hex("400a637573746f6d2d6b65790c637573746f6d2d76616c7565");
    for end in 1..literal.len() {
        assert!(Decoder::new(4096).decode(&literal[..end], 1024).is_err());
    }
}
#[test]
fn dynamic_updates_only_at_start_and_overflow_before_insert() {
    let mut d = Decoder::new(4096);
    assert!(matches!(
        d.decode(&hex("3fe21f"), 65536),
        Err(Error::Compression)
    ));
    assert_eq!(d.table.size, 0);
    assert_eq!(d.table.entries.capacity(), 0);
    assert!(matches!(
        d.decode(&hex("8220"), 1024),
        Err(Error::Compression)
    ));
    let mut d = Decoder::new(32);
    assert!(matches!(
        d.decode(&[0x40, 1, b'x', 1, b'y'], 33),
        Err(Error::Limit)
    ));
    assert_eq!(d.table.size, 0);
    assert_eq!(d.table.entries.capacity(), 0);
    let mut d = Decoder::new(4096);
    d.decode(&[0x40, 1, b'x', 1, b'y'], 34).unwrap();
    d.decode(&[0x20], 1024).unwrap();
    assert!(d.table.entries.is_empty());
    assert!(matches!(d.decode(&[0xbe], 1024), Err(Error::Compression)));
}
#[test]
fn encoder_dynamic_indexing_sensitive_literals_and_minimum_size_update() {
    let mut e = Encoder::new();
    let mut d = Decoder::new(4096);
    let fields = fields(&[
        (":status", "200"),
        ("x-a", "value"),
        ("authorization", "secret"),
        ("cookie", "a=b"),
    ]);
    let first = e.encode(&fields, 4096).unwrap();
    assert_eq!(d.decode(&first, 4096).unwrap(), fields);
    let second = e.encode(&fields, 4096).unwrap();
    assert!(second.len() < first.len());
    assert_eq!(d.decode(&second, 4096).unwrap(), fields);
    assert!(e
        .table
        .entries
        .iter()
        .all(|(n, _)| n != b"authorization" && n != b"cookie"));
    e.resize(0);
    e.resize(128);
    let bytes = e.encode(&fields, 4096).unwrap();
    assert_eq!(bytes[0], 0x20);
    assert_eq!(d.decode(&bytes, 4096).unwrap(), fields);
    assert_eq!(d.table.max, 128);
    let previous = e.table.size;
    assert!(matches!(e.encode(&fields, 1), Err(Error::Limit)));
    assert_eq!(e.table.size, previous);
}
#[test]
fn malformed_byte_corpus_is_bounded_and_never_panics() {
    for length in 0..48 {
        for seed in 0..64u8 {
            let bytes = (0..length)
                .map(|n| seed.wrapping_mul(53).wrapping_add(n as u8 * 3))
                .collect::<Vec<_>>();
            let mut d = Decoder::new(128);
            if let Ok(fields) = d.decode(&bytes, 256) {
                assert!(
                    fields
                        .iter()
                        .map(|(n, v)| n.len() + v.len() + 32)
                        .sum::<usize>()
                        <= 256
                );
            }
            assert!(d.table.size <= 128);
        }
    }
}
