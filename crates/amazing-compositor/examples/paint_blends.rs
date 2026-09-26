#[path = "../tests/support/paint_blends.rs"]
mod corpus;
fn main() {
    let legacy = std::env::args().any(|v| v == "--legacy");
    let (failures, losses, unexpected) = corpus::run(legacy);
    eprintln!("TOTAL: {failures}/85 cases with losses, {losses} pixels, {unexpected} unexpected cases");
    if !legacy { assert_eq!(unexpected, 0); }
}
