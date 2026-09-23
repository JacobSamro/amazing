//! Style, layout, raster, and audio commands.
use std::{cell::RefCell, rc::Rc, process::ExitCode, time::Duration};
use crate::{dom_host::BrowserDomHost, page::{collect_style_text, collect_author_css, load_page_media, load_page_text_from_sheets, PagePaint}, gui};
use super::{parse::{read_bytes, read_source}, js::{execute_inline_scripts, prepare_page_scripts}};
use crate::document::{DocumentKind, PageError, parse_document_bytes};
use crate::animation_scene::{AnimationScene, apply_to_layout};
use amazing_compositor::PropertyUpdate;
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
    collect_style_text(&dom, amazing_dom::NodeIdx(0), html_path, &mut sources);
    for extra in &args[1..] {
        match read_source(Some(extra), "style") {
            Ok(s) => {
                let mut sheet = amazing_css::parse_stylesheet(&s);
                sheet.base_url = Some(html_path.to_owned());
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
    collect_style_text(&dom, amazing_dom::NodeIdx(0), html_path, &mut sources);
    for extra in args.get(2..).unwrap_or(&[]) {
        match read_source(Some(extra), "layout") {
            Ok(s) => {
                let mut sheet = amazing_css::parse_stylesheet(&s);
                sheet.base_url = Some(html_path.to_owned());
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
        native_forwarded: false,
    }));
    if scripting {
        execute_inline_scripts(&host, html_path)?;
    }

    paint_host_surface(&host, html_path, width, fixed_height, extra_css)
}

/// WPT's capture boundary follows its first paint and the page's explicit
/// reftest readiness signal. Keep the scheduler alive while handlers and
/// timers mutate the same DOM, then rebuild every paint input from that DOM.
pub(crate) fn render_wpt_surface(
    html_path: &str,
    html: &[u8],
    width: u32,
    height: u32,
) -> Result<amazing_paint::Surface, PageError> {
    render_wpt_surface_after_first_paint(html_path, html, width, height, |_| {})
}

fn render_wpt_surface_after_first_paint(
    html_path: &str,
    html: &[u8],
    width: u32,
    height: u32,
    after_first_paint: impl FnOnce(&amazing_paint::Surface),
) -> Result<amazing_paint::Surface, PageError> {
    const MAX_REFTEST_FRAMES: u64 = 120;
    let document = parse_document_bytes(html, html_path, None)?;
    document.require_static_xml()?;
    let scripting = document.kind == DocumentKind::Html;
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: document.dom,
        native_forwarded: false,
    }));
    let mut scheduler = if scripting {
        let mut scheduler = prepare_page_scripts(&host, html_path, true)?
            .expect("forced page scheduler");
        scheduler.js.set_viewport_size(width, height);
        // Flush the script task's microtasks and due zero-delay timers before
        // the first rendering opportunity. Frame callbacks wait until after it.
        scheduler.pump_at(Duration::ZERO, false)
            .map_err(|error| format!("pre-paint event loop failed: {}", error.describe()))?;
        Some(scheduler)
    } else {
        None
    };
    let mut animations = AnimationScene::new();
    let first_updates = sample_animation_frame(&host, html_path, &mut animations, 0.)?;
    let first = paint_host_surface_with_updates(&host, html_path, width as f32, Some(height), "", &first_updates.main_thread_updates)?;
    after_first_paint(&first);
    let Some(scheduler) = scheduler.as_mut() else {
        return Ok(first);
    };
    for event in &first_updates.events {
        scheduler.dispatch_animation_event(event)
            .map_err(|error| format!("animation listener failed: {}", error.describe()))?;
    }

    scheduler.dispatch_dom_event(0, "TestRendered")
        .map_err(|error| format!("TestRendered listener failed: {}", error.describe()))?;
    scheduler.pump_at(Duration::ZERO, false)
        .map_err(|error| format!("TestRendered checkpoint failed: {}", error.describe()))?;
    let mut sampled_at = 0.;
    for frame in 1..=MAX_REFTEST_FRAMES {
        if !root_has_reftest_wait(&host)? {
            break;
        }
        scheduler.pump_at(Duration::from_millis(frame * 16), true)
            .map_err(|error| format!("reftest-wait checkpoint failed: {}", error.describe()))?;
        sampled_at = (frame * 16) as f64;
        let output = sample_animation_frame(&host, html_path, &mut animations, sampled_at)?;
        for event in &output.events {
            scheduler.dispatch_animation_event(event)
                .map_err(|error| format!("animation listener failed: {}", error.describe()))?;
        }
    }
    if root_has_reftest_wait(&host)? {
        return Err(PageError::ReftestWaitTimeout);
    }
    let final_updates = sample_animation_frame(&host, html_path, &mut animations, sampled_at)?;
    for event in &final_updates.events {
        scheduler.dispatch_animation_event(event)
            .map_err(|error| format!("animation listener failed: {}", error.describe()))?;
    }
    paint_host_surface_with_updates(&host, html_path, width as f32, Some(height), "", &final_updates.main_thread_updates)
}

fn sample_animation_frame(
    host: &Rc<RefCell<BrowserDomHost>>,
    html_path: &str,
    animations: &mut AnimationScene,
    now_ms: f64,
) -> Result<amazing_compositor::AnimationFrameOutput, PageError> {
    let dom = host.borrow();
    let mut sources = Vec::new();
    collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut sources)?;
    let sheets = sources.into_iter().map(|(_, _, sheet)| sheet).collect::<Vec<_>>();
    animations.frame(&dom.dom, &sheets, now_ms).map_err(PageError::Render)
}

fn root_has_reftest_wait(host: &Rc<RefCell<BrowserDomHost>>) -> Result<bool, PageError> {
    let host = host.borrow();
    let root = host.dom.document_element(amazing_dom::NodeIdx(0))
        .map_err(|error| PageError::Render(format!("document root: {error:?}")))?;
    Ok(root
        .and_then(|root| host.dom.get_attribute_ns(root, None, "class"))
        .is_some_and(|classes| classes.split_ascii_whitespace().any(|name| name == "reftest-wait")))
}

fn paint_host_surface(
    host: &Rc<RefCell<BrowserDomHost>>,
    html_path: &str,
    width: f32,
    fixed_height: Option<u32>,
    extra_css: &str,
) -> Result<amazing_paint::Surface, PageError> {
    paint_host_surface_with_updates(host, html_path, width, fixed_height, extra_css, &[])
}

fn paint_host_surface_with_updates(
    host: &Rc<RefCell<BrowserDomHost>>,
    html_path: &str,
    width: f32,
    fixed_height: Option<u32>,
    extra_css: &str,
    updates: &[PropertyUpdate],
) -> Result<amazing_paint::Surface, PageError> {

    let mut sources = Vec::new();
    let dom = host.borrow();
    collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut sources)?;
    let mut extra_sheet = amazing_css::parse_stylesheet(extra_css);
    extra_sheet.base_url = Some(html_path.to_owned());
    sources.push((extra_css.to_owned(), html_path.to_owned(), extra_sheet));
    let font_sheets = sources.iter().map(|(source, url, _)| (source.clone(), url.clone())).collect::<Vec<_>>();
    let sheets = sources.into_iter().map(|(_, _, sheet)| sheet).collect::<Vec<_>>();
    // Font discovery retains each sheet's URL; style computation retains each
    // sheet owner's implicit scope root.

    let media = load_page_media(&dom.dom, html_path);
    for warning in &media.warnings {
        eprintln!("amazing render: {warning}");
    }
    let page_text = load_page_text_from_sheets(&font_sheets, html_path);
    for warning in &page_text.warnings {
        eprintln!("amazing render: {warning}");
    }
    let mut root = amazing_layout::layout_dom_document_with_stylesheets_and_text_and_replaced_in_viewport(
        &dom.dom,
        &sheets,
        width,
        fixed_height.map(|height| height as f32),
        &page_text.provider,
        &media,
    );
    if apply_to_layout(&mut root, updates) {
        amazing_layout::block::layout_root_in_viewport(&mut root, width,
            fixed_height.map(|height| height as f32), &amazing_layout::font::FontSource::fallback());
    }
    let page_height = fixed_height.unwrap_or_else(|| {
        // Normal document rendering follows content height; WPT rendering
        // supplies a fixed viewport instead.
        amazing_layout::layout_overflow_bottom(&root)
            .ceil()
            .clamp(1.0, 8192.0) as u32
    });
    let styled = amazing_css::compute(&dom.dom, &sheets);
    let paint = PagePaint { media: &media, dom: &dom.dom, styles: &styled };
    Ok(amazing_paint::paint_document_with_text_resources(
        &root,
        width.max(1.0) as u32,
        page_height,
        &page_text.provider,
        &paint,
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

#[cfg(test)]
mod reftest_protocol_tests {
    use super::*;

    const STYLE: &str = "<style>body{margin:0}#box{width:12px;height:12px;background:red}.ready #box{width:32px;height:24px;background:lime}</style>";

    fn page(root_class: &str, script: &str) -> String {
        format!("<!doctype html><html class='{root_class}'>{STYLE}<body><div id='box'></div><script>{script}</script></body></html>")
    }

    fn ready_reference() -> amazing_paint::Surface {
        let html = page("ready", "");
        render_surface("reference.html", html.as_bytes(), 64., Some(48), "").unwrap()
    }

    #[test]
    fn test_rendered_follows_first_paint_once_and_repaints_mutated_layout() {
        let html = page("reftest-wait", "let calls=0; document.addEventListener('TestRendered',()=>{ calls++; document.documentElement.setAttribute('class', calls===1?'ready':'bad'); });");
        let mut first = None;
        let final_frame = render_wpt_surface_after_first_paint(
            "sync.html", html.as_bytes(), 64, 48,
            |surface| first = Some(surface.pixels.clone()),
        ).unwrap();
        assert_ne!(first.unwrap(), final_frame.pixels);
        assert_eq!(final_frame.pixels, ready_reference().pixels);
    }

    #[test]
    fn timer_removes_wait_class_at_bounded_checkpoint() {
        let html = page("reftest-wait", "document.addEventListener('TestRendered',()=>setTimeout(()=>document.documentElement.setAttribute('class','ready'),32));");
        let final_frame = render_wpt_surface("timer.html", html.as_bytes(), 64, 48).unwrap();
        assert_eq!(final_frame.pixels, ready_reference().pixels);
    }

    #[test]
    fn animation_frame_removes_wait_class_after_first_paint() {
        let html = page("reftest-wait", "document.addEventListener('TestRendered',()=>requestAnimationFrame(()=>document.documentElement.setAttribute('class','ready')));");
        let final_frame = render_wpt_surface("frame.html", html.as_bytes(), 64, 48).unwrap();
        assert_eq!(final_frame.pixels, ready_reference().pixels);
    }

    #[test]
    fn css_animation_end_event_releases_wait_and_paints_final_sample() {
        let html = "<!doctype html><html class='reftest-wait'><style>body{margin:0}@keyframes grow{from{width:8px}to{width:32px}}#box{height:16px;background:lime;animation:grow 32ms linear forwards}</style><body><div id='box'></div><script>document.querySelector('#box').addEventListener('animationend',e=>{if(e.animationName==='grow'&&e.elapsedTime===0.032)document.documentElement.setAttribute('class','ready')})</script></body></html>";
        let result = render_wpt_surface("animation.html", html.as_bytes(), 64, 48).unwrap();
        let reference = "<!doctype html><html><style>body{margin:0}#box{width:32px;height:16px;background:lime}</style><body><div id='box'></div></body></html>";
        let expected = render_surface("reference.html", reference.as_bytes(), 64., Some(48), "").unwrap();
        assert_eq!(result.pixels, expected.pixels);
    }

    #[test]
    fn transition_end_event_follows_testrendered_style_change() {
        let html = "<!doctype html><html class='reftest-wait'><style>body{margin:0}#box{width:8px;height:16px;background:lime;transition:width 32ms linear}#box.wide{width:32px}</style><body><div id='box'></div><script>let box=document.querySelector('#box');document.addEventListener('TestRendered',()=>box.setAttribute('class','wide'));box.addEventListener('transitionend',e=>{if(e.propertyName==='width'&&e.elapsedTime===0.032)document.documentElement.setAttribute('class','ready')})</script></body></html>";
        let result = render_wpt_surface("transition.html", html.as_bytes(), 64, 48).unwrap();
        let reference = "<!doctype html><html><style>body{margin:0}#box{width:32px;height:16px;background:lime}</style><body><div id='box'></div></body></html>";
        let expected = render_surface("reference.html", reference.as_bytes(), 64., Some(48), "").unwrap();
        assert_eq!(result.pixels, expected.pixels);
    }

    #[test]
    fn color_and_opacity_samples_reach_final_paint() {
        let html = "<!doctype html><html class='reftest-wait'><style>body{margin:0}@keyframes appear{from{background-color:red;opacity:0}to{background-color:lime;opacity:1}}#box{width:32px;height:16px;animation:appear 32ms linear forwards}</style><body><div id='box'></div><script>document.querySelector('#box').addEventListener('animationend',()=>document.documentElement.setAttribute('class','ready'))</script></body></html>";
        let result = render_wpt_surface("color-opacity.html", html.as_bytes(), 64, 48).unwrap();
        let reference = "<!doctype html><html><style>body{margin:0}#box{width:32px;height:16px;background-color:lime;opacity:1}</style><body><div id='box'></div></body></html>";
        let expected = render_surface("reference.html", reference.as_bytes(), 64., Some(48), "").unwrap();
        assert_eq!(result.pixels, expected.pixels);
    }

    #[test]
    fn missing_wait_removal_is_explicit_unsupported_without_image() {
        let html = page("reftest-wait", "");
        assert!(matches!(
            render_wpt_surface("timeout.html", html.as_bytes(), 64, 48),
            Err(PageError::ReftestWaitTimeout)
        ));
    }

    #[test]
    fn page_and_event_exceptions_cannot_become_passing_images() {
        let thrown = page("reftest-wait", "throw new Error('page sentinel')");
        assert!(matches!(
            render_wpt_surface("throw.html", thrown.as_bytes(), 64, 48),
            Err(PageError::Render(message)) if message.contains("page sentinel")
        ));
        let event = page("reftest-wait", "document.addEventListener('TestRendered',()=>{throw new Error('event sentinel')})");
        // DOM event dispatch reports listener exceptions without propagating
        // them. The still-present wait class must prevent a passing image.
        assert!(matches!(
            render_wpt_surface("event.html", event.as_bytes(), 64, 48),
            Err(PageError::ReftestWaitTimeout)
        ));
    }

    #[test]
    fn static_page_keeps_ordinary_render_pixels() {
        let html = page("ready", "");
        let ordinary = render_surface("static.html", html.as_bytes(), 64., Some(48), "").unwrap();
        let wpt = render_wpt_surface("static.html", html.as_bytes(), 64, 48).unwrap();
        assert_eq!(ordinary.pixels, wpt.pixels);
    }
}
