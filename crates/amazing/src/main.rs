//! Amazing browser driver: command dispatch.
use std::process::ExitCode;

mod browser_peer;
mod animation_scene;
mod cmd;
mod dom_host;
mod document;
mod gpu;
mod gui;
mod js_scheduler;
mod navigator;
mod navigation_response;
mod page;
mod font_metrics;
mod render_oracle;
mod runtime_binding_host;
mod runtime_host_a11y;
mod runtime_host_devtools;
mod runtime_host_ipc;
mod runtime_host_replaced_content;
#[cfg(test)]
mod driver_tests;
#[cfg(test)]
mod text_indent_native_tests;

// Shared adapters used by the fallback shell and WPT runner.
use dom_host::BrowserDomHost;
use page::{PageMedia, load_page_media, load_page_text};
use cmd::js::{PageScript, collect_script_text, collect_inline_handlers};
use cmd::webrtc::std_socket_v4;

const PIPELINE: &[(&str, &str)] = &[
    ("amazing-common", "byte streams + text utils"),
    (
        "amazing-html",
        "Phase 1: WHATWG tokenizer/tree builder — `parse` subcommand",
    ),
    (
        "amazing-css",
        "Phase 2: syntax 3 + selectors + cascade — `style` subcommand",
    ),
    (
        "amazing-layout",
        "Phase 3: block/inline FC layout — `layout` subcommand",
    ),
    (
        "amazing-paint",
        "Phase 4: software rasterizer — `render` subcommand",
    ),
    (
        "amazing-net",
        "Phase 5: HTTP/1.1 fetcher (library; TLS deferred)",
    ),
    (
        "amazing-js",
        "Phase 6: ECMA-262 parser + tree-walking interpreter — `run` subcommand",
    ),
    (
        "amazing-dom-js",
        "Phase 7: live DOM/JS bridge — `domrun` subcommand",
    ),
    (
        "amazing-gui",
        "Phase 8: native Windows shell — `gui` subcommand",
    ),
    (
        "amazing-media",
        "Phase 9: image + PCM WAV media — `render` / `audio-info` / `play-audio`",
    ),
    (
        "amazing-webrtc",
        "Phase 9: ICE/TURN + SDP + DTLS-SRTP nominated UDP + two-process gate",
    ),
    (
        "amazing-gpu",
        "Phase 9: D3D11 hardware capability — `gpu-info` subcommand",
    ),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--type=utility") {
        return runtime_host_replaced_content::run_image_utility_child();
    }
    #[cfg(windows)]
    if args.len() == 1 {
        return cmd::launch::default_gpui_shell();
    }
    match args.get(1).map(String::as_str) {
        Some("version") | None => {
            println!("amazing v{}", env!("CARGO_PKG_VERSION"));
            println!("zero-dependency browser engine — pipeline status:");
            for (crate_name, status) in PIPELINE {
                println!("  {crate_name:<15} {status}");
            }
            println!("dom: arena skeleton present (u32 indices, no Rc<RefCell>)");
            ExitCode::SUCCESS
        }
        Some("html5lib-tree") => cmd::html5lib_tree::cmd_html5lib_tree(&args[2..]),
        Some("parse") => cmd::parse::cmd_parse(args.get(2).map(String::as_str)),
        Some("parse-fragment") => match args.get(2) {
            Some(context) => cmd::parse::cmd_parse_fragment(context, args.get(3).map(String::as_str)),
            None => {
                eprintln!("usage: amazing parse-fragment <context> [file]");
                ExitCode::FAILURE
            }
        },
        Some("style") => cmd::render::cmd_style(&args[2..]),
        Some("layout") => cmd::render::cmd_layout(&args[2..]),
        Some("render") => cmd::render::cmd_render(&args[2..]),
        Some("render-page") => cmd::real_pages::cmd_render_page(&args[2..]),
        Some("render-wpt") => cmd::wpt::cmd_render_wpt(&args[2..]),
        Some("audio-info") => cmd::render::cmd_audio_info(&args[2..]),
        Some("play-audio") => cmd::render::cmd_play_audio(&args[2..]),
        Some("webrtc-loopback") => cmd::webrtc::cmd_webrtc_loopback(),
        Some("turn-probe") => cmd::webrtc::cmd_turn_probe(),
        Some("rtp-jitter-probe") => cmd::webrtc::cmd_rtp_jitter_probe(),
        Some("opus-probe") => cmd::webrtc::cmd_opus_probe(),
        Some("video-frame-probe") => cmd::webrtc::cmd_video_frame_probe(),
        Some("vp8-decode-probe") => cmd::webrtc::cmd_vp8_decode_probe(),
        Some("h264-roundtrip-probe") => cmd::webrtc::cmd_h264_roundtrip_probe(),
        Some("h264-rtp-probe") => cmd::webrtc::cmd_h264_rtp_probe(),
        Some("secure-media-loopback") => cmd::webrtc::cmd_secure_media_loopback(),
        Some("secure-media-processes") => cmd::webrtc::cmd_secure_media_processes(),
        Some("secure-video-stream") => cmd::webrtc::cmd_secure_video_stream(args.get(2).map(String::as_str)),
        Some("__secure-media-peer") => cmd::webrtc::cmd_secure_media_peer(
            args.get(2).map(String::as_str),
            args.get(3).map(String::as_str),
        ),
        Some("gpu-info") => cmd::gpu::cmd_gpu_info(),
        Some("gpu-present-smoke") => cmd::gpu::cmd_gpu_present_smoke(),
        Some("sdp-roundtrip") => cmd::webrtc::cmd_sdp_roundtrip(&args[2..]),
        Some("sdp-browser-answer") => cmd::webrtc::cmd_sdp_browser_answer(&args[2..]),
        Some("browser-peer") => cmd::webrtc::cmd_browser_peer(args.get(2).map(String::as_str)),
        Some("fetch") => cmd::net::cmd_fetch(&args[2..]),
        Some("fetch-parse") => cmd::net::cmd_fetch_parse(&args[2..]),
        Some("run") => cmd::js::cmd_run(args.get(2).map(String::as_str)),
        Some("domrun") => cmd::js::cmd_domrun(&args[2..]),
        Some("wpt-harness" | "wpt-harness-worker") => cmd::wpt_harness::run(&args[1..]),
        Some("a11y-tree") => runtime_host_a11y::cmd_a11y_tree(&args[2..]),
        Some("gui") => gui::run(args.get(2).map(String::as_str)),
        Some("devtools") => cmd_devtools(&args[2..]),
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!(
                "usage: amazing [version|html5lib-tree <enabled|disabled> <static|execute> [fragment-context]|parse [file]|parse-fragment <context> [file]|style <page.html> [extra.css…]|layout <page.html> [viewport_width]|render <page.html> <out.bmp> [width]|render-wpt <page.html> <out.ppm> <width> <height>|fetch <url|file>|fetch-parse <http-url>|audio-info <file.wav>|play-audio <file.wav>|webrtc-loopback|turn-probe|rtp-jitter-probe|opus-probe|video-frame-probe|vp8-decode-probe|h264-roundtrip-probe|h264-rtp-probe|secure-media-loopback|secure-media-processes|secure-video-stream <host:port>|sdp-roundtrip <offer.sdp>|sdp-browser-answer <offer.sdp>|browser-peer [http-port]|gpu-info|gpu-present-smoke|run [script.js]|domrun <page.html> <script.js>|gui [url]|devtools <url> [listen-addr]]"
            );
            ExitCode::FAILURE
        }
    }
}

/// `amazing devtools <url> [listen-addr]` — serve CDP for one fetched page in
/// the foreground until killed. `listen-addr` defaults to Chrome's
/// conventional `127.0.0.1:9222`.
fn cmd_devtools(args: &[String]) -> ExitCode {
    let url = match args.first() {
        Some(url) => url.as_str(),
        None => {
            eprintln!("usage: amazing devtools <url> [listen-addr]");
            return ExitCode::FAILURE;
        }
    };
    let addr = args.get(1).map(String::as_str).unwrap_or("127.0.0.1:9222");
    match runtime_host_devtools::spawn_devtools_server(url, addr) {
        Ok(handle) => {
            println!("DEVTOOLS_URL http://{}/json/version", handle.addr);
            println!("DEVTOOLS_BROWSER_ID {}", handle.browser_id);
            match handle.wait() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("amazing devtools: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        Err(error) => {
            eprintln!("amazing devtools: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod text_autospace_native_tests;
