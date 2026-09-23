//! Style, layout, raster, and audio commands.
use std::{cell::RefCell, rc::Rc, process::ExitCode};
use crate::{dom_host::BrowserDomHost, page::{collect_style_text, collect_author_css, load_page_media, load_page_text}, gui};
use super::{parse::{read_bytes, read_source}, js::execute_inline_scripts};
use crate::document::{DocumentKind, PageError, parse_document_bytes};
/// Phase 2 demo artifact: compute styles for an HTML page.
///
/// Stylesheets come from every `<style>` element in document order plus any
/// extra CSS files given after the page path (`<link rel=stylesheet>` needs
/// fetching, so it waits for Phase 5).
///
/// Output prints each element in tree order as `<tag style="prop:value; …">`
/// containing only the properties whose computed value differs from the
/// parent element's (the readable delta view from `StyledDom`).
pub(crate) fn cmd_style(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing style <page.html> [extra.css…]");
            return ExitCode::FAILURE;
        }
    };

    let html = match read_bytes(Some(html_path), "style") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = match parse_document_bytes(&html, html_path, None) {
        Ok(document) => document.dom,
        Err(error) => {
            eprintln!("amazing style: {error}");
            return error.exit_code();
        }
    };

    // Gather author stylesheets: inline <style> elements first.
    let mut sources = Vec::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut sources);
    for extra in &args[1..] {
        match read_source(Some(extra), "style") {
            Ok(s) => {
                let sheet = amazing_css::parse_stylesheet(&s);
                sources.push((s, sheet));
            }
            Err(code) => return code,
        }
    }

    let (_, sheets): (Vec<_>, Vec<_>) = sources.into_iter().unzip();
    let styled = amazing_css::compute(&dom, &sheets);

    print_styled(&dom, amazing_dom::NodeIdx(0), 0, &styled);
    ExitCode::SUCCESS
}

fn print_styled(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    depth: usize,
    styled: &amazing_css::StyledDom,
) {
    // Only elements print; document/text/comment nodes pass through.
    let is_element = dom.name(node).is_some();
    if let Some(name) = dom.name(node) {
        for _ in 0..depth {
            print!("  ");
        }
        match styled.delta_from_parent(node, dom.get(node).parent) {
            delta if delta.is_empty() => println!("<{name}>"),
            delta => {
                let decls: Vec<String> = delta.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                println!("<{name} style=\"{}\">", decls.join("; "));
            }
        }
    }
    // Unprinted nodes (document/text/comments) don't advance the indent.
    let child_depth = if is_element { depth + 1 } else { depth };
    for child in dom.children(node) {
        print_styled(dom, child, child_depth, styled);
    }
}

/// Phase 3 demo artifact: lay out an HTML page and print the layout tree —
/// every box with its absolute border/content rectangles, plus line boxes
/// with placed word fragments.
///
/// Viewport width comes from the second argument (default 800px).
/// Stylesheets: inline `<style>` elements + extra CSS file arguments, same
/// as `amazing style`. Text measurement uses the built-in fallback font
/// metrics (a real TTF can be plugged in later phases).
pub(crate) fn cmd_layout(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing layout <page.html> [viewport_width]");
            return ExitCode::FAILURE;
        }
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_bytes(Some(html_path), "layout") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = match parse_document_bytes(&html, html_path, None) {
        Ok(document) => document.dom,
        Err(error) => {
            eprintln!("amazing layout: {error}");
            return error.exit_code();
        }
    };

    // Author CSS: <style> elements + extra files (same policy as `style`).
    let mut sources = Vec::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut sources);
    for extra in args.get(2..).unwrap_or(&[]) {
        match read_source(Some(extra), "layout") {
            Ok(s) => {
                let sheet = amazing_css::parse_stylesheet(&s);
                sources.push((s, sheet));
            }
            Err(code) => return code,
        }
    }

    let (_, sheets): (Vec<_>, Vec<_>) = sources.into_iter().unzip();
    let styled = amazing_css::compute(&dom, &sheets);
    let fonts = amazing_layout::font::FontSource::fallback();
    let mut root = amazing_layout::box_tree::build_layout_tree(&dom, &styled, &fonts);
    amazing_layout::block::layout_root_in_viewport(&mut root, width, None, &fonts);
    print!("{}", amazing_layout::dump_layout(&dom, &root));
    ExitCode::SUCCESS
}

/// Phase 4 demo: `amazing render <page.html> <out.bmp> [viewport_width]`
/// (plus optional extra CSS files after the width). Lays out the page,
/// rasterizes it into the software framebuffer, and writes a 24-bit BMP
/// alongside a PPM twin (`<out>.ppm`).
pub(crate) fn cmd_render(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing render <page.html> <out.bmp> [viewport_width] [extra.css…]");
            return ExitCode::FAILURE;
        }
    };
    let out_path = args.get(1).map(String::as_str).unwrap_or("render.bmp");
    let width: f32 = args.get(2).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_bytes(Some(html_path), "render") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut extra_css = String::new();
    for extra in args.get(3..).unwrap_or(&[]) {
        match read_source(Some(extra), "render") {
            Ok(s) => {
                extra_css.push('\n');
                extra_css.push_str(&s);
            }
            Err(code) => return code,
        }
    }
    let surface = match render_surface(html_path, &html, width, None, &extra_css) {
        Ok(surface) => surface,
        Err(error) => {
            eprintln!("amazing render: {error}");
            return error.exit_code();
        }
    };

    if let Err(e) = surface.save_bmp(out_path) {
        eprintln!("amazing render: cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    let ppm_path = match out_path.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.ppm"),
        None => format!("{out_path}.ppm"),
    };
    if let Err(e) = surface.save_ppm(&ppm_path) {
        eprintln!("amazing render: cannot write {ppm_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "rendered {}x{} px → {out_path} (+{ppm_path})",
        surface.width, surface.height
    );
    ExitCode::SUCCESS
}

pub(crate) fn render_surface(
    html_path: &str,
    html: &[u8],
    width: f32,
    fixed_height: Option<u32>,
    extra_css: &str,
) -> Result<amazing_paint::Surface, PageError> {
    let document = parse_document_bytes(html, html_path, None)?;
    document.require_static_xml()?;
    let scripting = document.kind == DocumentKind::Html;
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: document.dom,
    }));
    if scripting {
        execute_inline_scripts(&host, html_path)?;
    }

    let mut sources = Vec::new();
    let dom = host.borrow();
    collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut sources)?;
    sources.push((extra_css.to_owned(), amazing_css::parse_stylesheet(extra_css)));
    let (sources, sheets): (Vec<_>, Vec<_>) = sources.into_iter().unzip();
    // Font discovery still uses the legacy text input. Style computation below
    // consumes the separate sheets, retaining each owner's implicit scope root.
    let css = sources.join("\n");

    let media = load_page_media(&dom.dom, html_path);
    for warning in &media.warnings {
        eprintln!("amazing render: {warning}");
    }
    let page_text = load_page_text(&css, html_path);
    for warning in &page_text.warnings {
        eprintln!("amazing render: {warning}");
    }
    let root = amazing_layout::layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport(
        &dom.dom,
        &sheets,
        width,
        fixed_height.map(|height| height as f32),
        &page_text.provider,
        &media,
    );
    let page_height = fixed_height.unwrap_or_else(|| {
        // Normal document rendering follows content height; WPT rendering
        // supplies a fixed viewport instead.
        amazing_layout::layout_overflow_bottom(&root)
            .ceil()
            .clamp(1.0, 8192.0) as u32
    });
    Ok(amazing_paint::paint_document_with_text_resources(
        &root,
        width.max(1.0) as u32,
        page_height,
        &page_text.provider,
        &media,
    ))
}

/// Phase 9.2 audio artifact: decode PCM WAV and report stream properties.
pub(crate) fn cmd_audio_info(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(path) => path,
        None => {
            eprintln!("usage: amazing audio-info <file.wav>");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("amazing audio-info: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let audio = match amazing_media::decode_wav(&bytes) {
        Ok(audio) => audio,
        Err(error) => {
            eprintln!("amazing audio-info: {error}");
            return ExitCode::FAILURE;
        }
    };
    let frames = audio.samples.len() / audio.channels as usize;
    let duration = frames as f64 / audio.sample_rate as f64;
    let peak = audio
        .samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    let channel_word = if audio.channels == 1 {
        "channel"
    } else {
        "channels"
    };
    println!(
        "PCM WAV: {} Hz, {} {channel_word}, {frames} frames, {duration:.6} s, peak {peak}",
        audio.sample_rate, audio.channels
    );
    ExitCode::SUCCESS
}

/// Validates a PCM WAV through the core decoder, then plays it via WinMM.
pub(crate) fn cmd_play_audio(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(path) => path,
        None => {
            eprintln!("usage: amazing play-audio <file.wav>");
            return ExitCode::FAILURE;
        }
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("amazing play-audio: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = amazing_media::decode_wav(&bytes) {
        eprintln!("amazing play-audio: {error}");
        return ExitCode::FAILURE;
    }
    gui::play_wav_file(path)
}
