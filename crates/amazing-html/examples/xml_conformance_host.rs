//! Batch observation of the actual XML byte decoder and fatal-error parser.
//! Fixture expectations and profile selection belong to the pinned corpus runner.
use amazing_html::{xml, xml_encoding, xml_resources::XmlFileResolver};
use std::io::{self, BufRead};

fn hex(text: &str) -> String {
    text.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn observe(path: &str) -> (String, String) {
    let mut resolver = match std::env::var_os("AMAZING_XML_RESOURCE_ROOT") {
        Some(root) => match XmlFileResolver::new(std::path::Path::new(&root)) {
            Ok(value) => Some(value),
            Err(error) => return (format!("{:?}", error.kind), error.to_string()),
        },
        None => None,
    };
    let (bytes, source) = if let Some(resolver) = &resolver {
        match resolver.read_document(std::path::Path::new(path)) {
            Ok(value) => (value.bytes, Some(value.source)),
            Err(error) => return (format!("{:?}", error.kind), error.to_string()),
        }
    } else {
        match std::fs::read(path) {
            Ok(bytes) => (bytes, None),
            Err(error) => return ("io-error".into(), error.to_string()),
        }
    };
    // Files are document entities, not MIME responses carrying an authoritative
    // HTTP charset/BOM recovery contract. No fixture expectation participates.
    let decoded = match xml_encoding::decode_entity(&bytes) {
        Ok(decoded) => decoded,
        Err(error) => return ("decode-error".into(), error.to_string()),
    };
    let parsed = match (source, resolver.as_mut()) {
        (Some(source), Some(resolver)) => {
            xml::parse_xml_decoded_with_resolver(&decoded.text, &source, resolver)
        }
        _ => xml::parse_xml_decoded(&decoded.text),
    };
    match parsed {
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
