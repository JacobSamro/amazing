//! Batch observation of the actual XML byte decoder and fatal-error parser.
//! Fixture expectations and profile selection belong to the pinned corpus runner.
use amazing_html::{xml, xml_encoding};
use std::io::{self, BufRead};

fn hex(text: &str) -> String {
    text.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect()
}
fn observe(path: &str) -> (String, String) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return ("io-error".into(), error.to_string()),
    };
    let decoded = match xml_encoding::decode(&bytes, None) {
        Ok(decoded) => decoded,
        Err(error) => return ("decode-error".into(), error.to_string()),
    };
    match xml::parse_xml_decoded(&decoded.text) {
        Ok(_) => ("accepted".into(), String::new()),
        Err(error) => (format!("{:?}", error.kind), error.to_string()),
    }
}
fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("corpus input protocol");
        let (id, path) = line.split_once('\t').expect("id and fixture path");
        let (status, message) = std::panic::catch_unwind(|| observe(path))
            .unwrap_or_else(|_| ("crash".into(), "XML parser panicked".into()));
        println!("{}\t{}\t{}", id, status, hex(&message));
    }
}
