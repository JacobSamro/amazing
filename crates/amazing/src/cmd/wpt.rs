//! Fixed-viewport rendering for WPT reftests.
use std::process::ExitCode;
use super::{parse::read_bytes, render::render_surface};
/// Deterministic rendering primitive for WPT reftests. Unlike `render`, the
/// output surface is always exactly the requested viewport and only the PPM
/// artifact is written. Test orchestration and comparison live outside the
/// engine so this command remains a reusable style → layout → paint boundary.
pub(crate) fn cmd_render_wpt(args: &[String]) -> ExitCode {
    if args.len() != 4 {
        eprintln!("usage: amazing render-wpt <page.html> <out.ppm> <width> <height>");
        return ExitCode::FAILURE;
    }
    let html_path = &args[0];
    let out_path = &args[1];
    let dimension = |value: &str, name: &str| -> Result<u32, String> {
        value
            .parse::<u32>()
            .ok()
            .filter(|parsed| (1..=8192).contains(parsed))
            .ok_or_else(|| format!("{name} must be an integer from 1 through 8192"))
    };
    let width = match dimension(&args[2], "width") {
        Ok(value) => value,
        Err(error) => {
            eprintln!("amazing render-wpt: {error}");
            return ExitCode::FAILURE;
        }
    };
    let height = match dimension(&args[3], "height") {
        Ok(value) => value,
        Err(error) => {
            eprintln!("amazing render-wpt: {error}");
            return ExitCode::FAILURE;
        }
    };
    let html = match read_bytes(Some(html_path), "render-wpt") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let surface = match render_surface(html_path, &html, width as f32, Some(height), "") {
        Ok(surface) => surface,
        Err(error) => {
            eprintln!("amazing render-wpt: {error}");
            return error.exit_code();
        }
    };
    if let Err(error) = surface.save_ppm(out_path) {
        eprintln!("amazing render-wpt: cannot write {out_path}: {error}");
        return ExitCode::FAILURE;
    }
    println!("rendered fixed viewport {width}x{height} px → {out_path}");
    ExitCode::SUCCESS
}
