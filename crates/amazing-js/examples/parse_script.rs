//! Parse a supplied upstream script without executing its host-environment code.
use std::io::Read;
fn main() -> std::process::ExitCode {
    let mut source = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut source) {
        eprintln!("stdin: {error}");
        return std::process::ExitCode::FAILURE;
    }
    match amazing_js::parse_script(&source) {
        Ok(program) => {
            println!("parsed {} top-level statements", program.body.len());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
