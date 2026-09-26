#[path="../tests/support/paint_filter_parity.rs"]
mod corpus;
fn main(){let legacy=std::env::args().any(|a|a=="--legacy");let failures=corpus::run(legacy);if !legacy{assert_eq!(failures,0);}}
