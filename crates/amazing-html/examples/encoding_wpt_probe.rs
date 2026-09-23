//! Ported data selection from pinned WPT encoding tests; see evidence TSV.
use amazing_html::encoding::{decode, Source};

fn normalized(value: &str) -> String {
    value.chars().filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn main() {
    let mut labels = [0usize; 3];
    let mut indexes = [0usize; 3];
    for line in include_str!("../evidence/html15-wpt-selection.tsv").lines() {
        if line.starts_with('#') { continue; }
        let mut fields = line.splitn(3, '\t');
        let kind = fields.next().expect("case kind");
        let label = fields.next().expect("label");
        let expected = fields.next().expect("expected");
        // The pinned WPT view is 255 bytes long: 0x00 through 0xFE.
        let decoded = decode(&(0u8..255).collect::<Vec<_>>(), Some(label));
        let supported = decoded.source == Source::Transport;
        let (counts, status) = match kind {
            "L" => {
                let actual = format!("{:?}", decoded.encoding);
                let pass = normalized(&actual) == normalized(expected);
                (&mut labels, if !supported { "unsupported" } else if pass { "pass" } else { "fail" })
            }
            "S" => {
                let expected_chars: Vec<char> = expected.split(',').map(|part| {
                    if part == "-" { '\u{FFFD}' } else {
                        char::from_u32(part.parse().expect("code point")).expect("scalar")
                    }
                }).collect();
                let pass = decoded.text.chars().eq(
                    (0u8..128).map(char::from).chain(expected_chars.into_iter().take(127))
                );
                (&mut indexes, if !supported { "unsupported" } else if pass { "pass" } else { "fail" })
            }
            _ => panic!("unknown case kind"),
        };
        counts[match status { "pass" => 0, "fail" => 1, _ => 2 }] += 1;
        println!("{kind}\t{label}\t{status}");
    }
    println!("SUMMARY labels={labels:?} indexes={indexes:?}");
}
