//! Style, layout, raster, and audio commands.
use std::{cell::RefCell, rc::Rc, process::ExitCode, time::Duration};
use crate::{dom_host::BrowserDomHost, page::{collect_style_text, collect_author_css, load_page_media, load_page_text_from_sheets, PagePaint}, gui};
use super::{parse::{read_bytes, read_source}, js::{execute_inline_scripts, prepare_page_scripts}};
use crate::document::{DocumentKind, PageError, parse_document_bytes};
use crate::animation_scene::AnimationScene;
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
    let styled = amazing_css::compute_with_environment(&dom, &sheets,
        &amazing_css::MediaEnvironment {
            viewport_width_px: width.max(1.0),
            ..amazing_css::MediaEnvironment::default()
        });
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
    let mut oracle = crate::render_oracle::RenderCoordinator::default();
    let first_updates = sample_animation_frame(&host, html_path, &mut animations, width, height, 0.)?;
    let first = paint_host_surface_with_updates(&host, html_path, width as f32, Some(height), "",
        animations.animated(), false, crate::render_oracle::enabled().then_some(&mut oracle))?;
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
        sampled_at = (frame * 16) as f64;
        // The compositor ticks once per frame from the EventLoop's post-rAF
        // render step, so the retained property scene samples the same virtual
        // clock the harness advances and stays aligned with the page's rAF
        // mutations. Events are dispatched after the loop step releases.
        let mut handoff_events = Vec::new();
        {
            let dom_host = host.clone();
            let oracle_ref = &mut oracle;
            let (timeline, animated) = animations.handoff_state_mut();
            scheduler.pump_at_with_render(
                Duration::from_millis(frame * 16),
                true,
                |phase, timestamp| {
                    if phase != amazing_dom::event_loop::RenderPhase::AfterAnimationFrames {
                        return;
                    }
                    let at = timestamp.as_secs_f64() * 1000.;
                    let dom = dom_host.borrow();
                    if let Some((_, events)) = oracle_ref.on_vsync(&dom.dom, timeline, animated, at) {
                        handoff_events.extend(events);
                    }
                },
            )
            .map_err(|error| format!("reftest-wait checkpoint failed: {}", error.describe()))?;
        }
        for event in handoff_events {
            use amazing_compositor::CssAnimationEventKind as Kind;
            use amazing_css::animation::AnimationMainThreadEventKind as MainKind;
            let kind = match event.kind {
                MainKind::AnimationEnd => Kind::AnimationEnd,
                MainKind::AnimationCancel => Kind::AnimationCancel,
                MainKind::TransitionEnd => Kind::TransitionEnd,
                MainKind::TransitionCancel => Kind::TransitionCancel,
            };
            scheduler.dispatch_animation_event(&amazing_compositor::CssAnimationEvent {
                element: event.node.0 as u64, kind, name: event.name,
                elapsed_time_ms: event.elapsed_time_ms,
            }).map_err(|error| format!("animation listener failed: {}", error.describe()))?;
        }
        let output = sample_animation_frame(&host, html_path, &mut animations, width, height, sampled_at)?;
        for event in &output.events {
            scheduler.dispatch_animation_event(event)
                .map_err(|error| format!("animation listener failed: {}", error.describe()))?;
        }
    }
    if root_has_reftest_wait(&host)? {
        return Err(PageError::ReftestWaitTimeout);
    }
    let final_updates = sample_animation_frame(&host, html_path, &mut animations, width, height, sampled_at)?;
    for event in &final_updates.events {
        scheduler.dispatch_animation_event(event)
            .map_err(|error| format!("animation listener failed: {}", error.describe()))?;
    }
    paint_host_surface_with_updates(&host, html_path, width as f32, Some(height), "",
        animations.animated(), false, crate::render_oracle::enabled().then_some(&mut oracle))
}

fn sample_animation_frame(
    host: &Rc<RefCell<BrowserDomHost>>,
    html_path: &str,
    animations: &mut AnimationScene,
    width: u32,
    height: u32,
    now_ms: f64,
) -> Result<amazing_compositor::AnimationFrameOutput, PageError> {
    let dom = host.borrow();
    let mut sources = Vec::new();
    collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut sources)?;
    let sheets = sources.into_iter().map(|(_, _, sheet)| sheet).collect::<Vec<_>>();
    let environment = amazing_css::MediaEnvironment {
        viewport_width_px: width.max(1) as f32,
        viewport_height_px: height.max(1) as f32,
        ..amazing_css::MediaEnvironment::default()
    };
    animations.frame_with_environment(&dom.dom, &sheets, &environment, now_ms)
        .map_err(PageError::Render)
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
    let production = std::env::var_os("AMAZING_INCREMENTAL_PRODUCTION").is_some_and(|value| value == "1");
    let parallel = std::env::var_os("AMAZING_PARALLEL_TILES").is_some_and(|value| value == "1");
    let mut oracle = if production && parallel {
        crate::render_oracle::RenderCoordinator::production_parallel()
    } else if production { crate::render_oracle::RenderCoordinator::production() }
        else { crate::render_oracle::RenderCoordinator::default() };
    paint_host_surface_with_updates(host, html_path, width, fixed_height, extra_css,
        &amazing_css::AnimatedValues::default(), true,
        (production || crate::render_oracle::enabled()).then_some(&mut oracle))
}

fn paint_host_surface_with_updates(
    host: &Rc<RefCell<BrowserDomHost>>,
    html_path: &str,
    width: f32,
    fixed_height: Option<u32>,
    extra_css: &str,
    animated: &amazing_css::AnimatedValues,
    // The WPT animation loop may have samples elsewhere in the document;
    // CSS-13's inline classifier does not account for those samples yet.
    property_only_allowed: bool,
    mut coordinator: Option<&mut crate::render_oracle::RenderCoordinator>,
) -> Result<amazing_paint::Surface, PageError> {

    let mut sources = Vec::new();
    let mut dom = host.borrow_mut();
    collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut sources)?;
    let mut extra_sheet = amazing_css::parse_stylesheet(extra_css);
    extra_sheet.base_url = Some(html_path.to_owned());
    sources.push((extra_css.to_owned(), html_path.to_owned(), extra_sheet));
    let font_sheets = sources.iter().map(|(source, url, _)| (source.clone(), url.clone())).collect::<Vec<_>>();
    let sheets = sources.into_iter().map(|(_, _, sheet)| sheet).collect::<Vec<_>>();
    let environment = amazing_css::MediaEnvironment {
        viewport_width_px: width.max(1.0) as u32 as f32,
        viewport_height_px: fixed_height.unwrap_or(600).max(1) as f32,
        ..amazing_css::MediaEnvironment::default()
    };
    // Font discovery retains each sheet's URL; style computation retains each
    // sheet owner's implicit scope root.

    if let Some(coordinator) = coordinator.as_deref_mut()
        && coordinator.is_production() && property_only_allowed
    {
        let viewport = (width.max(1.0) as u32, fixed_height.unwrap_or(600));
        let epoch = dom.dom.render_mutation_epoch();
        let journal = dom.dom.take_render_mutations(epoch);
        if let Some(surface) = coordinator.parallel_frame_if_idle(&journal) {
            return Ok(surface);
        }
        if let Some(surface) = coordinator.try_property_only(&dom.dom, &sheets, viewport, &journal) {
            return Ok(surface);
        }
        let candidate = coordinator.candidate(&dom.dom, &sheets, viewport, journal);
        let styled = candidate.styles.clone();
        coordinator.commit(&dom.dom, &sheets, viewport, &styled);
        let media = load_page_media(&dom.dom, html_path, &styled);
        let page_text = load_page_text_from_sheets(&font_sheets, html_path);
        let paint = PagePaint { media: &media, dom: &dom.dom, styles: &styled };
        coordinator.update_layout_production(&dom.dom, &styled, candidate,
            &page_text.provider, &paint, width, fixed_height.map(|height| height as f32));
        let height = fixed_height.unwrap_or_else(||
            amazing_layout::layout_overflow_bottom(coordinator.layout_root())
                .ceil().clamp(1.0, 8192.0) as u32);
        return match coordinator.compositor_candidate(&dom.dom, viewport.0, height,
            &page_text.provider, &paint, None) {
            Ok(frame) => Ok(frame.pixels),
            Err(_) => {
                coordinator.reset_scene();
                Ok(amazing_paint::paint_document_with_text_resources(
                    coordinator.layout_root(), viewport.0, height, &page_text.provider, &paint))
            }
        };
    }

    let mut style_divergence = None;
    let mut style_candidate = None;
    if let Some(coordinator) = coordinator.as_deref_mut() {
        let viewport = (width.max(1.0) as u32, fixed_height.unwrap_or(600));
        let epoch = dom.dom.render_mutation_epoch();
        let journal = dom.dom.take_render_mutations(epoch);
        if property_only_allowed {
            if let Some(surface) = coordinator.try_property_only(&dom.dom, &sheets, viewport, &journal) {
                return Ok(surface);
            }
        }
        let candidate = coordinator.candidate(&dom.dom, &sheets, viewport, journal);
        let full = amazing_css::compute_with_environment(&dom.dom, &sheets, &environment);
        style_divergence = crate::render_oracle::first_style_mismatch(&dom.dom, &candidate.styles, &full);
        crate::render_oracle::report_style_work(&candidate);
        coordinator.commit(&dom.dom, &sheets, viewport, &full);
        style_candidate = Some(candidate);
    }
    let styled = amazing_css::compute_with_animations_and_environment(
        &dom.dom, &sheets, animated, &environment);
    let media = load_page_media(&dom.dom, html_path, &styled);
    for warning in &media.warnings {
        eprintln!("amazing render: {warning}");
    }
    let page_text = load_page_text_from_sheets(&font_sheets, html_path);
    for warning in &page_text.warnings {
        eprintln!("amazing render: {warning}");
    }
    let root = amazing_layout::layout_styled_document(
        &dom.dom,
        &styled,
        &page_text.provider,
        &media,
        width,
        fixed_height.map(|height| height as f32),
    );
    let page_height = fixed_height.unwrap_or_else(|| {
        // Normal document rendering follows content height; WPT rendering
        // supplies a fixed viewport instead.
        amazing_layout::layout_overflow_bottom(&root)
            .ceil()
            .clamp(1.0, 8192.0) as u32
    });
    let paint = PagePaint { media: &media, dom: &dom.dom, styles: &styled };
    if let Some(coordinator) = coordinator {
        // The host borrow freezes the DOM through both computations. The
        // second layout and painter own scratch state; neither can dispatch
        // events or mutate the candidate compositor's retained resources.
        let (geometry, paint_update) = coordinator.update_layout_and_paint(
            &dom.dom, &styled, style_candidate.expect("oracle style candidate"),
            style_divergence.is_some(), &root, width.max(1.0) as u32, page_height,
            &page_text.provider, &paint, width, fixed_height.map(|height| height as f32),
        );
        eprintln!("amazing oracle: layout outcome={:?} dirty={} changed={} paint={:?}",
            geometry.outcome, geometry.dirty.len(), geometry.changed.len(),
            paint_update.as_ref().map(|update| match update {
                amazing_paint::display_list_view::PaintViewUpdate::Delta(delta) =>
                    format!("delta(changed={},ranges={})", delta.changes.len(), delta.changed_ranges.len()),
                amazing_paint::display_list_view::PaintViewUpdate::RequiresFullRebuild { reason, .. } =>
                    format!("full({reason:?})"),
            }));
        let layout_divergence = crate::render_oracle::first_layout_mismatch(coordinator.layout_root(), &root);
        let candidate = coordinator.compositor_candidate(
            &dom.dom, width.max(1.0) as u32, page_height,
            &page_text.provider, &paint, paint_update.as_ref(),
        );
        let work = coordinator.last_work();
        eprintln!("amazing oracle: work style_calls={} layout_calls={} paint_view_calls={} style_recalculated={} layout_dirty={} layout_full={} paint_changed={} paint_full={} tile_uploads={} tiles_dirtied={}",
            work.style_rebuild_calls, work.layout_update_calls, work.paint_view_rebuild_calls,
            work.style_recalculated, work.layout_dirty, work.layout_full,
            work.paint_changed, work.paint_full, work.tile_uploads, work.tiles_dirtied);
        let full_styles = amazing_css::compute_with_animations_and_environment(
            &dom.dom, &sheets, animated, &environment);
        let full_root = amazing_layout::layout_styled_document(
            &dom.dom, &full_styles, &page_text.provider, &media,
            width, fixed_height.map(|height| height as f32),
        );
        let full_paint = PagePaint { media: &media, dom: &dom.dom, styles: &full_styles };
        let full_view = amazing_paint::display_list_view::DisplayListView::with_dom_generations(
            &full_root, width.max(1.0) as u32, page_height, &page_text.provider,
            &full_paint, &amazing_paint::controls::NoControls, &dom.dom,
        );
        let full_items = crate::render_oracle::PaintSnapshot::from_view(&full_view);
        let full = amazing_paint::paint_document_with_text_resources(
            &full_root, width.max(1.0) as u32, page_height, &page_text.provider, &full_paint,
        );
        let divergence = style_divergence
            .or_else(|| crate::render_oracle::first_style_mismatch(&dom.dom, &styled, &full_styles))
            .or(layout_divergence)
            .or_else(|| crate::render_oracle::first_layout_mismatch(&root, &full_root))
            .or_else(|| candidate.as_ref().ok().and_then(|candidate| candidate.paint.first_mismatch(&full_items)));
        if let Some(divergence) = divergence {
            crate::render_oracle::parity_loss(&divergence, candidate.as_ref().ok().map(|candidate| &candidate.pixels), &full);
            coordinator.reset_scene();
            return Ok(full);
        }
        match candidate {
            Ok(candidate) => {
                eprintln!("amazing oracle: frame uploads={} dirtied_tiles={}",
                    candidate.uploads, candidate.dirtied_tiles);
                let parity = coordinator.pixel_parity(&candidate.pixels, &full);
                if parity.losses > 0 {
                    let detail = parity.first_loss.map_or_else(
                        || format!("pixel parity lost {} pixels or frame dimensions differ", parity.losses),
                        |first| format!("pixel ({}, {}): candidate {:?}, full {:?}; losses={}",
                            first.x, first.y, first.candidate, first.full, parity.losses),
                    );
                    crate::render_oracle::parity_loss(&detail, Some(&candidate.pixels), &full);
                    coordinator.reset_scene();
                } else {
                    if parity.layer_rounding > 0 {
                        crate::render_oracle::layer_rounding(parity.layer_rounding as usize);
                    }
                    crate::render_oracle::matched(full.width, full.height);
                    return Ok(candidate.pixels);
                }
            }
            Err(error) => {
                crate::render_oracle::full_fallback(&error);
                coordinator.reset_scene();
            }
        }
        return Ok(full);
    }
    Ok(amazing_paint::paint_document_with_text_resources(
        &root, width.max(1.0) as u32, page_height, &page_text.provider, &paint,
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
    fn important_width_overrides_animation_in_layout() {
        let html = "<!doctype html><html class='reftest-wait'><style>body{margin:0}@keyframes grow{from{width:8px}to{width:32px}}#box{width:8px!important;height:16px;background:lime;animation:grow 32ms linear forwards}</style><body><div id='box'></div><script>document.querySelector('#box').addEventListener('animationend',()=>document.documentElement.setAttribute('class','ready'))</script></body></html>";
        let result = render_wpt_surface("important-animation.html", html.as_bytes(), 64, 48).unwrap();
        let reference = "<!doctype html><html><style>body{margin:0}#box{width:8px;height:16px;background:lime}</style><body><div id='box'></div></body></html>";
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

    #[test]
    fn root_viewport_font_size_uses_the_render_width() {
        let html = b"<style>html{font-size:10vw}body{margin:0}.box{width:1rem;height:20px;background:black}</style><div class=box></div>";
        let narrow = render_surface("viewport.html", html, 200., Some(60), "").unwrap();
        let wide = render_surface("viewport.html", html, 400., Some(60), "").unwrap();
        assert_eq!(narrow.get_pixel(30, 10), Some([255; 4]));
        assert_eq!(wide.get_pixel(30, 10), Some([0, 0, 0, 255]));
    }
}

#[cfg(test)]
mod oracle_trace_tests {
    use super::*;
    use amazing_dom::NodeIdx;
    use std::time::Instant;

    const ARTICLE: &str = "<body><style>body{margin:0;background:white;color:#223}main{width:220px}main.changed{width:205px}p{margin:12px 0}</style><header>Long article</header><main><h1>Chapter one</h1><p>Text wraps through a narrow column of paragraphs.</p><p>More text follows beneath the fold.</p><p>And more lines continue for scrolling.</p><p>Final paragraph at the bottom.</p></main></body>";
    const EFFECTS: &str = "<body><style>body{margin:0;background:white}.fixed{position:fixed;left:2px;top:2px;background:red}.scroller{width:300px;height:180px;overflow:scroll;background:#ddd}.scroller.changed{width:280px}.moving{will-change:transform;background:blue;width:120px;height:80px}.fade{opacity:.5;background:green;width:120px;height:80px}</style><div class='fixed'>Fixed</div><section class='scroller'><div class='moving'>Move</div><div class='fade'>Fade</div><p>Overflow content</p><p>More overflow content</p></section></body>";
    const OPACITY: &str = "<body><style>body{margin:0}#box{width:120px;height:80px;background:green}.base{opacity:.4}.changed{opacity:.6}</style><div id='box' class='base'></div></body>";
    const OPACITY_CSS: &str = "";

    fn percentile(values: &[u128], quantile: f64) -> u128 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        sorted[((sorted.len() - 1) as f64 * quantile).ceil() as usize]
    }

    fn trace(name: &str, html: &str, css: &str, tag: &str, base_class: &str, production: bool) {
        let make_host = || Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(html), native_forwarded: false,
        }));
        let host = make_host();
        let direct_host = make_host();
        let find_node = |host: &Rc<RefCell<BrowserDomHost>>| {
            let dom = &host.borrow().dom;
            (0..dom.node_count()).map(|index| NodeIdx(index as u32))
                .find(|node| dom.name(*node) == Some(tag)).unwrap()
        };
        let node = find_node(&host);
        let direct_node = find_node(&direct_host);
        let mut coordinator = if production { crate::render_oracle::RenderCoordinator::production() }
            else { crate::render_oracle::RenderCoordinator::default() };
        let animated = amazing_css::AnimatedValues::default();
        let _ = paint_host_surface_with_updates(&host, "trace.html", 512., Some(512), css, &animated, true, Some(&mut coordinator)).unwrap();
        let _ = paint_host_surface_with_updates(&direct_host, "trace.html", 512., Some(512), css, &animated, true, None).unwrap();
        let mut candidate_us = Vec::new();
        let mut direct_us = Vec::new();
        let mut style_calls = 0;
        let mut layout_calls = 0;
        let mut paint_view_calls = 0;
        let mut layout_full = 0;
        let mut paint_full = 0;
        let mut uploads = 0;
        let mut dirtied = 0;
        let mut layer_rounding_pixels = 0;
        for index in 0..20 {
            let class = if index % 2 == 0 { format!("{base_class} changed").trim().to_owned() } else { base_class.to_owned() };
            let start = Instant::now();
            host.borrow_mut().dom.set_attribute(node, "class", &class).unwrap();
            let candidate = paint_host_surface_with_updates(&host, "trace.html", 512., Some(512), css, &animated, true, Some(&mut coordinator)).unwrap();
            candidate_us.push(start.elapsed().as_micros());
            let work = coordinator.last_work();
            style_calls += work.style_rebuild_calls;
            layout_calls += work.layout_update_calls;
            paint_view_calls += work.paint_view_rebuild_calls;
            layout_full += usize::from(work.layout_full);
            paint_full += usize::from(work.paint_full);
            uploads += work.tile_uploads;
            dirtied += work.tiles_dirtied;
            let start = Instant::now();
            direct_host.borrow_mut().dom.set_attribute(direct_node, "class", &class).unwrap();
            let direct = paint_host_surface_with_updates(&direct_host, "trace.html", 512., Some(512), css, &animated, true, None).unwrap();
            direct_us.push(start.elapsed().as_micros());
            let parity = coordinator.pixel_parity(&candidate, &direct);
            assert_eq!(parity.losses, 0, "{name} frame {index}: {:?}", parity.first_loss);
            if name != "effects-class" {
                assert_eq!(parity.layer_rounding, 0, "{name} frame {index}");
            }
            layer_rounding_pixels += parity.layer_rounding;
        }
        let mode = if production { "production" } else { "oracle" };
        eprintln!("{mode}_trace fixture={name} frames=20 p50_us={} p95_us={} full_p50_us={} full_p95_us={} style_calls={style_calls} layout_update_calls={layout_calls} layout_full={layout_full} paint_view_calls={paint_view_calls} paint_full={paint_full} uploads={uploads} dirtied_tiles={dirtied} layer_rounding_pixels={layer_rounding_pixels}",
            percentile(&candidate_us, 0.5), percentile(&candidate_us, 0.95),
            percentile(&direct_us, 0.5), percentile(&direct_us, 0.95));
        if name == "opacity-class" {
            assert_eq!((style_calls, layout_calls, paint_view_calls, uploads), (0, 0, 0, 0));
        }
    }

    #[test]
    #[ignore = "Windows timing trace; run explicitly on win-001"]
    fn incremental_oracle_mutation_trace() {
        trace("article-class", ARTICLE, "", "main", "", false);
        trace("effects-class", EFFECTS, "", "section", "scroller", false);
        trace("opacity-class", OPACITY, OPACITY_CSS, "div", "base", false);
    }

    #[test]
    #[ignore = "Windows release timing trace; run explicitly on win-001"]
    fn incremental_production_mutation_trace() {
        trace("article-class", ARTICLE, "", "main", "", true);
        trace("effects-class", EFFECTS, "", "section", "scroller", true);
        trace("opacity-class", OPACITY, OPACITY_CSS, "div", "base", true);
    }

    #[test]
    fn parallel_tiles_present_a_complete_frame_after_vsync() {
        let html = "<body><style>body{margin:0}.block{width:64px;height:64px;background:red}.block.changed{background:blue}</style><div class='block'></div></body>";
        let make_host = || Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(html), native_forwarded: false,
        }));
        let host = make_host();
        let direct_host = make_host();
        let node = (0..host.borrow().dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| host.borrow().dom.name(*node) == Some("div")).unwrap();
        let direct_node = (0..direct_host.borrow().dom.node_count())
            .map(|index| NodeIdx(index as u32))
            .find(|node| direct_host.borrow().dom.name(*node) == Some("div")).unwrap();
        let animated = amazing_css::AnimatedValues::default();
        let mut coordinator = crate::render_oracle::RenderCoordinator::production_parallel();
        paint_host_surface_with_updates(&host, "parallel.html", 512., Some(512), "",
            &animated, true, Some(&mut coordinator)).unwrap();
        host.borrow_mut().dom.set_attribute(node, "class", "block changed").unwrap();
        direct_host.borrow_mut().dom.set_attribute(direct_node, "class", "block changed").unwrap();
        let mut candidate = paint_host_surface_with_updates(&host, "parallel.html", 512., Some(512), "",
            &animated, true, Some(&mut coordinator)).unwrap();
        let scheduled = coordinator.last_work().tile_jobs_scheduled;
        assert!(scheduled > 0, "paint mutation should queue worker tiles");
        let full = paint_host_surface_with_updates(&direct_host, "parallel.html", 512., Some(512), "",
            &animated, true, None).unwrap();
        let mut timeline = amazing_compositor::CssCompositorTimeline::default();
        let mut animated_values = amazing_css::AnimatedValues::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while coordinator.pixel_parity(&candidate, &full).losses != 0 && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
            if let Some((ready, _)) = coordinator.on_vsync(
                &host.borrow().dom, &mut timeline, &mut animated_values, 16.) {
                candidate = ready;
            }
        }
        assert_eq!(coordinator.pixel_parity(&candidate, &full).losses, 0);
    }

    #[test]
    fn inline_opacity_uses_validated_property_scene() {
        const HTML: &str = "<body style='margin:0'><div id='box' style='width:64px;height:64px;background:green;opacity:.5'></div></body>";
        let make_host = || Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(HTML), native_forwarded: false,
        }));
        let host = make_host();
        let full_host = make_host();
        let node = (0..host.borrow().dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| host.borrow().dom.get_attribute_ns(*node, None, "id") == Some("box"))
            .unwrap();
        let mut coordinator = crate::render_oracle::RenderCoordinator::default();
        let animated = amazing_css::AnimatedValues::default();
        paint_host_surface_with_updates(&host, "opacity.html", 128., Some(128), "", &animated, true,
            Some(&mut coordinator)).unwrap();
        for opacity in [".25", ".75"] {
            let next = format!("width:64px;height:64px;background:green;opacity:{opacity}");
            host.borrow_mut().dom.set_attribute(node, "style", &next).unwrap();
            full_host.borrow_mut().dom.set_attribute(node, "style", &next).unwrap();
            let candidate = paint_host_surface_with_updates(&host, "opacity.html", 128., Some(128), "",
                &animated, true, Some(&mut coordinator)).unwrap();
            let full = paint_host_surface_with_updates(&full_host, "opacity.html", 128., Some(128), "",
                &animated, true, None).unwrap();
            assert_eq!(candidate.pixels, full.pixels);
            let work = coordinator.last_work();
            assert_eq!((work.style_rebuild_calls, work.layout_update_calls,
                work.paint_view_rebuild_calls, work.tile_uploads), (0, 0, 0, 0));
        }
    }

    #[test]
    fn class_opacity_uses_validated_property_scene() {
        const HTML: &str = "<body style='margin:0'><style>.base{opacity:.4}.changed{opacity:.6}</style><div id='box' class='base' style='width:64px;height:64px;background:green'></div></body>";
        const CSS: &str = "";
        let make_host = || Rc::new(RefCell::new(BrowserDomHost {
            dom: amazing_html::parse(HTML), native_forwarded: false,
        }));
        let host = make_host();
        let full_host = make_host();
        let node = (0..host.borrow().dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| host.borrow().dom.get_attribute_ns(*node, None, "id") == Some("box"))
            .unwrap();
        let mut coordinator = crate::render_oracle::RenderCoordinator::default();
        let animated = amazing_css::AnimatedValues::default();
        paint_host_surface_with_updates(&host, "opacity.html", 128., Some(128), CSS, &animated, true,
            Some(&mut coordinator)).unwrap();
        for class in ["changed", "base"] {
            host.borrow_mut().dom.set_attribute(node, "class", class).unwrap();
            full_host.borrow_mut().dom.set_attribute(node, "class", class).unwrap();
            let candidate = paint_host_surface_with_updates(&host, "opacity.html", 128., Some(128), CSS,
                &animated, true, Some(&mut coordinator)).unwrap();
            let full = paint_host_surface_with_updates(&full_host, "opacity.html", 128., Some(128), CSS,
                &animated, true, None).unwrap();
            assert_eq!(candidate.pixels, full.pixels);
            let work = coordinator.last_work();
            assert_eq!((work.style_rebuild_calls, work.layout_update_calls,
                work.paint_view_rebuild_calls, work.tile_uploads), (0, 0, 0, 0));
        }
    }
}
