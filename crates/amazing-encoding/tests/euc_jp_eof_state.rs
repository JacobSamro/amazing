use amazing_encoding::{Decoder, Encoding};

fn select_jis0212_at_eof(decoder: &mut Decoder, output: &mut String, fatal: bool) {
    decoder.push(&[0x8F, 0xA1], output).unwrap();
    let result = decoder.finish(output);
    assert_eq!(result.is_err(), fatal);
    if let Err(error) = result { assert_eq!(error.byte_offset, 0); }
    decoder.finish(output).unwrap();
}

#[test]
fn euc_jp_eof_keeps_jis0212_selector_separate_from_its_lead_byte() {
    for lead in 0xA1..=0xFE {
        let input = [0x8F, lead];
        for fatal in [false, true] {
            for split in 0..=input.len() {
                let mut decoder = Decoder::new(Encoding::EucJp, fatal);
                let mut output = String::new();
                decoder.push(&input[..split], &mut output).unwrap();
                decoder.push(&input[split..], &mut output).unwrap();
                let result = decoder.finish(&mut output);
                assert_eq!(result.is_err(), fatal);
                if let Err(error) = result { assert_eq!(error.byte_offset, 0); }
                assert_eq!(decoder.buffered_input_bytes(), 0);
                decoder.finish(&mut output).unwrap();
                // ASCII and valid halfwidth katakana leave the selector set.
                decoder.push(b"A", &mut output).unwrap();
                decoder.push(&[0x8E, 0xA1], &mut output).unwrap();
                // Pointer108 is U+02D8 in JIS0212 and null in JIS0208.
                decoder.push(&[0xA2, 0xAF], &mut output).unwrap();
                let second = decoder.push(&[0xA2, 0xAF], &mut output);
                assert_eq!(second.is_err(), fatal);
                if let Err(error) = second { assert_eq!(error.byte_offset, 7); }
                decoder.push(&[0xA4, 0xA2], &mut output).unwrap();
                decoder.finish(&mut output).unwrap();
                assert_eq!(output, if fatal { "A｡˘あ" } else { "�A｡˘�あ" },
                           "lead{lead:02x} fatal{fatal} split{split}");
            }
        }
    }
}

#[test]
fn euc_jp_valid_halfwidth_katakana_keeps_the_eof_selector() {
    for halfwidth in 0xA1..=0xDF {
        for fatal in [false, true] {
            let mut decoder = Decoder::new(Encoding::EucJp, fatal);
            let mut output = String::new();
            select_jis0212_at_eof(&mut decoder, &mut output, fatal);
            output.clear();
            decoder.push(&[0x8E, halfwidth], &mut output).unwrap();
            decoder.push(&[0xA2, 0xAF], &mut output).unwrap();
            decoder.finish(&mut output).unwrap();
            let expected = format!("{}˘", char::from_u32(0xFF61 + halfwidth as u32 - 0xA1).unwrap());
            assert_eq!(output, expected, "halfwidth{halfwidth:02x} fatal{fatal}");
        }
    }
}

#[test]
fn euc_jp_invalid_initial_bytes_do_not_clear_the_eof_selector() {
    for byte in (0x80..=0xA0).filter(|b| !matches!(*b, 0x8E | 0x8F)).chain([0xFF]) {
        for fatal in [false, true] {
            let mut decoder = Decoder::new(Encoding::EucJp, fatal);
            let mut output = String::new();
            select_jis0212_at_eof(&mut decoder, &mut output, fatal);
            output.clear();
            let result = decoder.push(&[byte], &mut output);
            assert_eq!(result.is_err(), fatal);
            if let Err(error) = result { assert_eq!(error.byte_offset, 2); }
            decoder.push(&[0xA2, 0xAF], &mut output).unwrap();
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, if fatal { "˘" } else { "�˘" }, "byte{byte:02x} fatal{fatal}");
        }
    }
}

#[test]
fn euc_jp_generic_pair_paths_reset_the_selector_on_success_and_error() {
    let resets: &[&[u8]] = &[
        &[0xA2, 0xAF], // Successful JIS0212 pair.
        &[0xA1, 0xA1], // Null JIS0212 pointer.
        &[0xA1, b' '], // Invalid ASCII trailing byte, restored.
        &[0x8E, 0xE0], // Invalid halfwidth trailing byte.
        &[0x8F, b' '], // Invalid JIS0212 second byte, restored.
        &[0x8F, 0xA1, b' '], // Invalid JIS0212 third byte, restored.
    ];
    for reset in resets {
        for fatal in [false, true] {
            let mut decoder = Decoder::new(Encoding::EucJp, fatal);
            let mut output = String::new();
            select_jis0212_at_eof(&mut decoder, &mut output, fatal);
            let _ = decoder.push(reset, &mut output);
            // Process any restored ASCII with EOF before the next pair, so
            // this control is independent of the queued-input ticket.
            decoder.finish(&mut output).unwrap();
            output.clear();
            let result = decoder.push(&[0xA2, 0xAF], &mut output);
            assert_eq!(result.is_err(), fatal, "reset{reset:?} fatal{fatal}");
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, if fatal { "" } else { "�" });
        }
    }
}

#[test]
fn euc_jp_regular_jis0208_jis0212_and_halfwidth_controls_remain_unchanged() {
    let input = b"A\x8F\xA2\xAF\xA4\xA2\x8E\xA1Z";
    for fatal in [false, true] {
        for split in 0..=input.len() {
            let mut decoder = Decoder::new(Encoding::EucJp, fatal);
            let mut output = String::new();
            decoder.push(&input[..split], &mut output).unwrap();
            decoder.push(&input[split..], &mut output).unwrap();
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, "A˘あ｡Z", "fatal{fatal} split{split}");
            decoder.finish(&mut output).unwrap();
            assert_eq!(output, "A˘あ｡Z");
        }
    }
}
