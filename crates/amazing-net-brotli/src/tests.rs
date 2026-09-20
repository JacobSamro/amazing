use super::*;
fn vectors() -> Vec<(Vec<u8>, Vec<u8>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors");
    let mut result = Vec::new();
    for name in ["empty", "hello", "unicode", "binary", "bomb", "random"] {
        let plain = std::fs::read(root.join(format!("{name}.raw"))).unwrap();
        for quality in [0, 5, 11] {
            result.push((
                plain.clone(),
                std::fs::read(root.join(format!("{name}-{quality}.br"))).unwrap(),
            ));
        }
    }
    result
}
#[test]
fn independent_google_brotli_vectors_roundtrip_at_exact_limit() {
    for (plain, encoded) in vectors() {
        assert_eq!(decode(&encoded, plain.len()).unwrap(), plain);
    }
}
#[test]
fn all_nonempty_vectors_fail_one_byte_below_limit() {
    for (plain, encoded) in vectors() {
        if !plain.is_empty() {
            assert_eq!(
                decode(&encoded, plain.len() - 1),
                Err(DecodeError::OutputLimit)
            );
            assert_eq!(decode(&encoded, 0), Err(DecodeError::OutputLimit));
        }
    }
}
#[test]
fn every_prefix_of_small_vectors_is_truncated_or_invalid() {
    for (plain, encoded) in vectors() {
        if encoded.len() > 512 {
            continue;
        }
        for end in 0..encoded.len() {
            assert!(
                decode(&encoded[..end], plain.len()).is_err(),
                "prefix {end}/{}",
                encoded.len()
            );
        }
    }
}
#[test]
fn rejects_trailing_bytes_and_concatenated_streams() {
    for (plain, encoded) in vectors() {
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert_eq!(
            decode(&trailing, plain.len()),
            Err(DecodeError::TrailingData)
        );
        let mut doubled = encoded.clone();
        doubled.extend_from_slice(&encoded);
        assert_eq!(
            decode(&doubled, plain.len() * 2),
            Err(DecodeError::TrailingData)
        );
    }
}
#[test]
fn malformed_and_large_window_extensions_fail_closed() {
    for data in [&[0xff, 0xff][..], &[0x11, 0xff], &[0x11, 0x18, 0x00]] {
        assert!(decode(data, 1024).is_err());
    }
}
#[test]
fn output_rejection_precedes_allocation_and_preserves_bytes() {
    let mut output = Output {
        bytes: Vec::new(),
        limit: 3,
    };
    assert_eq!(output.append(&[0; 4]), Err(DecodeError::OutputLimit));
    assert_eq!(output.bytes.capacity(), 0);
    output.append(b"abc").unwrap();
    let capacity = output.bytes.capacity();
    assert_eq!(output.append(b"d"), Err(DecodeError::OutputLimit));
    assert_eq!(output.bytes.capacity(), capacity);
    assert_eq!(output.bytes, b"abc");
}
#[test]
fn scratch_rejection_precedes_allocation_including_integer_overflow() {
    let budget = Rc::new(Budget::new(8));
    let mut alloc = BoundedAlloc(budget.clone());
    let memory: Memory<u32> = alloc.alloc_cell(3);
    assert_eq!(memory.data.capacity(), 0);
    assert_eq!(budget.peak.get(), 0);
    let memory: Memory<u32> = alloc.alloc_cell(usize::MAX);
    assert_eq!(memory.data.capacity(), 0);
    assert_eq!(budget.peak.get(), 0);
    assert_eq!(budget.error.get(), Some(DecodeError::ScratchLimit));
}
#[test]
fn shared_scratch_budget_releases_memory_and_fails_closed() {
    let budget = Rc::new(Budget::new(8));
    let mut alloc = BoundedAlloc(budget.clone());
    let a: Memory<u32> = alloc.alloc_cell(2);
    assert_eq!(budget.live.get(), 8);
    let b: Memory<u8> = alloc.alloc_cell(1);
    assert!(b.data.is_empty());
    drop(a);
    assert_eq!(budget.live.get(), 0);
    let encoded = include_bytes!("../tests/vectors/hello-5.br");
    assert_eq!(
        decode_with_budget(encoded, 128, budget.clone()),
        Err(DecodeError::ScratchLimit)
    );
    assert_eq!(budget.live.get(), 0);
}
#[test]
fn compression_bomb_keeps_scratch_bounded_and_returns_no_output() {
    let budget = Rc::new(Budget::new(MAX_SCRATCH_BYTES));
    let encoded = include_bytes!("../tests/vectors/bomb-11.br");
    assert!(encoded.len() < 100);
    assert_eq!(
        decode_with_budget(encoded, 16, budget.clone()),
        Err(DecodeError::OutputLimit)
    );
    assert!(budget.peak.get() <= MAX_SCRATCH_BYTES);
    assert_eq!(budget.live.get(), 0);
}

#[test]
fn valid_google_large_window_extension_is_rejected_by_strict_decoder() {
    assert_eq!(
        decode(include_bytes!("../tests/vectors/large-window.br"), 128),
        Err(DecodeError::Invalid)
    );
}
