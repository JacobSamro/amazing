//! amazing — browser driver binary.
//!
//! Phase 0 artifact: builds and runs, prints the component pipeline status.
//! Each phase replaces one line of "not started" with a real capability
//! plus its demo subcommand (`parse` in Phase 1, `style` in Phase 2).

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, ExitCode, ExitStatus};
use std::rc::Rc;
use std::time::{Duration, Instant};

use amazing_js::DomHost;

mod gpu;
mod gui;

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
        "Phase 9: ICE + SDP + DTLS-SRTP nominated UDP + two-process gate",
    ),
    (
        "amazing-gpu",
        "Phase 9: D3D11 hardware capability — `gpu-info` subcommand",
    ),
];

pub(crate) struct BrowserDomHost {
    pub(crate) dom: amazing_dom::Dom,
}

struct PageImage {
    node: amazing_dom::NodeIdx,
    image: amazing_media::Image,
    used_size: amazing_layout::IntrinsicSize,
}

#[derive(Default)]
pub(crate) struct PageMedia {
    images: Vec<PageImage>,
    pub(crate) warnings: Vec<String>,
}

impl amazing_layout::ReplacedElementProvider for PageMedia {
    fn intrinsic_size(&self, node: amazing_dom::NodeIdx) -> Option<amazing_layout::IntrinsicSize> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| entry.used_size)
    }
}

impl amazing_paint::ImageProvider for PageMedia {
    fn image(&self, node: amazing_dom::NodeIdx) -> Option<&amazing_media::Image> {
        self.images
            .iter()
            .find(|entry| entry.node == node)
            .map(|entry| &entry.image)
    }
}

pub(crate) fn load_page_media(dom: &amazing_dom::Dom, page: &str) -> PageMedia {
    fn dimension(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, name: &str) -> Option<f32> {
        dom.attrs_of(node)?
            .iter()
            .find(|(attribute, _)| attribute == name)?
            .1
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|value| *value > 0)
            .map(|value| value as f32)
    }

    fn walk(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, page: &str, media: &mut PageMedia) {
        if dom.name(node) == Some("img") {
            let source = dom
                .attrs_of(node)
                .and_then(|attrs| attrs.iter().find(|(name, _)| name == "src"))
                .map(|(_, value)| value.as_str())
                .filter(|value| !value.trim().is_empty());
            if let Some(source) = source {
                let resolved = resolve_script_url(page, source);
                match amazing_net::fetch(&resolved, &amazing_net::FetchOptions::default())
                    .map_err(|error| error.to_string())
                    .and_then(|response| {
                        amazing_media::decode_image(&response.body)
                            .map_err(|error| error.to_string())
                    }) {
                    Ok(image) => {
                        let intrinsic_width = image.width as f32;
                        let intrinsic_height = image.height as f32;
                        let width_attr = dimension(dom, node, "width");
                        let height_attr = dimension(dom, node, "height");
                        let used_size = match (width_attr, height_attr) {
                            (Some(width), Some(height)) => {
                                amazing_layout::IntrinsicSize { width, height }
                            }
                            (Some(width), None) => amazing_layout::IntrinsicSize {
                                width,
                                height: intrinsic_height * width / intrinsic_width,
                            },
                            (None, Some(height)) => amazing_layout::IntrinsicSize {
                                width: intrinsic_width * height / intrinsic_height,
                                height,
                            },
                            (None, None) => amazing_layout::IntrinsicSize {
                                width: intrinsic_width,
                                height: intrinsic_height,
                            },
                        };
                        media.images.push(PageImage {
                            node,
                            image,
                            used_size,
                        });
                    }
                    Err(error) => media
                        .warnings
                        .push(format!("image {source:?} ({resolved}): {error}")),
                }
            }
        }
        for child in dom.children(node) {
            walk(dom, child, page, media);
        }
    }

    let mut media = PageMedia::default();
    walk(dom, amazing_dom::NodeIdx(0), page, &mut media);
    media
}

impl BrowserDomHost {
    fn serialize_node(&self, node: amazing_dom::NodeIdx, out: &mut String) {
        match &self.dom.get(node).kind {
            amazing_dom::NodeKind::Document => {
                for child in self.dom.children(node) {
                    self.serialize_node(child, out);
                }
            }
            amazing_dom::NodeKind::Element { name, attrs, .. } => {
                out.push('<');
                out.push_str(name);
                for (key, value) in attrs {
                    out.push(' ');
                    out.push_str(key);
                    out.push_str("=\"");
                    out.push_str(value);
                    out.push('"');
                }
                out.push('>');
                for child in self.dom.children(node) {
                    self.serialize_node(child, out);
                }
                out.push_str("</");
                out.push_str(name);
                out.push('>');
            }
            amazing_dom::NodeKind::Text { data } => out.push_str(data),
            amazing_dom::NodeKind::Comment { data } => {
                out.push_str("<!--");
                out.push_str(data);
                out.push_str("-->");
            }
            amazing_dom::NodeKind::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push_str("?>");
            }
        }
    }

    fn find_named_in(
        dom: &amazing_dom::Dom,
        node: amazing_dom::NodeIdx,
        name: &str,
    ) -> Option<amazing_dom::NodeIdx> {
        if dom.namespace(node) == Some(amazing_dom::Namespace::Html) && dom.name(node) == Some(name)
        {
            return Some(node);
        }
        for child in dom.children(node) {
            if let Some(found) = Self::find_named_in(dom, child, name) {
                return Some(found);
            }
        }
        None
    }

    fn clone_from(
        &mut self,
        source: &amazing_dom::Dom,
        source_node: amazing_dom::NodeIdx,
        parent: amazing_dom::NodeIdx,
    ) {
        let kind = source.get(source_node).kind.clone();
        let created = match kind {
            amazing_dom::NodeKind::Element {
                name,
                attrs,
                namespace,
            } => self.dom.append_child(
                parent,
                amazing_dom::NodeKind::Element {
                    name,
                    attrs,
                    namespace,
                },
            ),
            amazing_dom::NodeKind::Text { data } => self.dom.create_text(parent, &data),
            amazing_dom::NodeKind::Comment { data } => self
                .dom
                .append_child(parent, amazing_dom::NodeKind::Comment { data }),
            amazing_dom::NodeKind::ProcessingInstruction { target, data } => self.dom.append_child(
                parent,
                amazing_dom::NodeKind::ProcessingInstruction { target, data },
            ),
            amazing_dom::NodeKind::Document => return,
        };
        let children: Vec<_> = source.children(source_node).collect();
        for child in children {
            self.clone_from(source, child, created);
        }
    }

    fn text_content(&self, node: amazing_dom::NodeIdx, out: &mut String) {
        if let amazing_dom::NodeKind::Text { data } = &self.dom.get(node).kind {
            out.push_str(data);
        }
        for child in self.dom.children(node) {
            self.text_content(child, out);
        }
    }

    fn find(&self, node: amazing_dom::NodeIdx, selector: &str) -> Option<u32> {
        let selectors = Self::parse_selector_list(selector)?;
        self.find_any(node, &selectors)
    }

    fn parse_selector_list(selector: &str) -> Option<Vec<amazing_css::selectors::Selector>> {
        let mut parts = Vec::new();
        let mut start = 0usize;
        let mut parens = 0usize;
        let mut brackets = 0usize;
        let mut quote = None;
        let mut escaped = false;
        for (index, ch) in selector.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' && quote.is_some() {
                escaped = true;
                continue;
            }
            if let Some(active) = quote {
                if ch == active {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => parens += 1,
                ')' if parens > 0 => parens -= 1,
                '[' => brackets += 1,
                ']' if brackets > 0 => brackets -= 1,
                ',' if parens == 0 && brackets == 0 => {
                    parts.push(selector[start..index].trim());
                    start = index + ch.len_utf8();
                }
                _ => {}
            }
        }
        parts.push(selector[start..].trim());
        if parts.iter().any(|part| part.is_empty()) {
            return None;
        }
        parts
            .into_iter()
            .map(amazing_css::selectors::parse_selector)
            .collect()
    }

    fn find_any(
        &self,
        node: amazing_dom::NodeIdx,
        selectors: &[amazing_css::selectors::Selector],
    ) -> Option<u32> {
        if selectors
            .iter()
            .any(|selector| amazing_css::selectors::matches(&self.dom, node, selector))
        {
            return Some(node.0);
        }
        for child in self.dom.children(node) {
            if let Some(found) = self.find_any(child, selectors) {
                return Some(found);
            }
        }
        None
    }

    fn find_all(&self, node: amazing_dom::NodeIdx, selector: &str, out: &mut Vec<u32>) {
        let Some(selectors) = Self::parse_selector_list(selector) else {
            return;
        };
        self.find_all_parsed(node, &selectors, out);
    }

    fn find_all_parsed(
        &self,
        node: amazing_dom::NodeIdx,
        selectors: &[amazing_css::selectors::Selector],
        out: &mut Vec<u32>,
    ) {
        if selectors
            .iter()
            .any(|selector| amazing_css::selectors::matches(&self.dom, node, selector))
        {
            out.push(node.0);
        }
        for child in self.dom.children(node) {
            self.find_all_parsed(child, selectors, out);
        }
    }

    fn css_property_name(name: &str) -> String {
        let mut out = String::new();
        for ch in name.chars() {
            if ch.is_ascii_uppercase() {
                out.push('-');
                out.push(ch.to_ascii_lowercase());
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn style_declarations(&self, node: amazing_dom::NodeIdx) -> Vec<(String, String)> {
        let raw = self
            .dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        raw.split(';')
            .filter_map(|declaration| {
                let (name, value) = declaration.split_once(':')?;
                let name = name.trim().to_ascii_lowercase();
                if name.is_empty() {
                    return None;
                }
                Some((name, value.trim().to_string()))
            })
            .collect()
    }

    fn write_style_declarations(
        &mut self,
        node: amazing_dom::NodeIdx,
        declarations: &[(String, String)],
    ) -> Result<(), String> {
        let serialized = declarations
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, value)| format!("{name}: {value};"))
            .collect::<Vec<_>>()
            .join(" ");
        self.set_attribute(node.0, "style", &serialized)
    }
}

impl amazing_js::DomHost for BrowserDomHost {
    fn query_selector(&mut self, selector: &str) -> Option<u32> {
        self.find(amazing_dom::NodeIdx(0), selector)
    }

    fn query_selector_all(&mut self, selector: &str) -> Vec<u32> {
        let mut nodes = Vec::new();
        self.find_all(amazing_dom::NodeIdx(0), selector, &mut nodes);
        nodes
    }

    fn matches_selector(&self, node: u32, selector: &str) -> bool {
        let Some(selectors) = Self::parse_selector_list(selector) else {
            return false;
        };
        selectors.iter().any(|selector| {
            amazing_css::selectors::matches(&self.dom, amazing_dom::NodeIdx(node), selector)
        })
    }

    fn parent_node(&self, node: u32) -> Option<u32> {
        if node as usize >= self.dom.node_count() {
            return None;
        }
        self.dom
            .get(amazing_dom::NodeIdx(node))
            .parent
            .map(|parent| parent.0)
    }

    fn get_text_content(&self, node: u32) -> Option<String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() {
            return None;
        }
        let mut text = String::new();
        self.text_content(idx, &mut text);
        Some(text)
    }

    fn set_text_content(&mut self, node: u32, text: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("textContent requires an element".into());
        }
        let children: Vec<_> = self.dom.children(idx).collect();
        for child in children {
            self.dom.detach(child);
        }
        self.dom.create_text(idx, text);
        Ok(())
    }

    fn get_document_title(&self) -> String {
        let Some(title) = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "title") else {
            return String::new();
        };
        let mut text = String::new();
        self.text_content(title, &mut text);
        text
    }

    fn set_document_title(&mut self, title: &str) -> Result<(), String> {
        if let Some(node) = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "title") {
            return self.set_text_content(node.0, title);
        }
        let head = Self::find_named_in(&self.dom, amazing_dom::NodeIdx(0), "head")
            .ok_or_else(|| "document has no head element".to_string())?;
        let title_node = self.dom.create_element(head, "title");
        self.dom.create_text(title_node, title);
        Ok(())
    }

    fn fetch(&mut self, base_url: &str, input: &str) -> Result<amazing_js::HostResponse, String> {
        let target = if base_url.is_empty() {
            input.to_owned()
        } else {
            resolve_script_url(base_url, input)
        };
        let response = amazing_net::fetch(&target, &amazing_net::FetchOptions::default())
            .map_err(|error| error.to_string())?;
        let body = response
            .text()
            .ok_or_else(|| "fetch response was not text".to_string())?;
        Ok(amazing_js::HostResponse {
            url: response.url,
            status: response.status,
            body,
            headers: response
                .headers
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
        })
    }

    fn get_inner_html(&self, node: u32) -> Option<String> {
        if node as usize >= self.dom.node_count()
            || self.dom.name(amazing_dom::NodeIdx(node)).is_none()
        {
            return None;
        }
        let mut output = String::new();
        for child in self.dom.children(amazing_dom::NodeIdx(node)) {
            self.serialize_node(child, &mut output);
        }
        Some(output)
    }

    fn set_inner_html(&mut self, node: u32, html: &str) -> Result<(), String> {
        let target = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() || self.dom.name(target).is_none() {
            return Err("innerHTML requires an element".into());
        }
        let parsed = amazing_html::parse(html);
        let fragment_root = Self::find_named_in(&parsed, amazing_dom::NodeIdx(0), "body")
            .unwrap_or(amazing_dom::NodeIdx(0));
        let children: Vec<_> = self.dom.children(target).collect();
        for child in children {
            self.dom.detach(child);
        }
        let source_children: Vec<_> = parsed.children(fragment_root).collect();
        for child in source_children {
            self.clone_from(&parsed, child, target);
        }
        Ok(())
    }

    fn get_attribute(&self, node: u32, name: &str) -> Option<Option<String>> {
        if node as usize >= self.dom.node_count() {
            return None;
        }
        let attrs = self.dom.attrs_of(amazing_dom::NodeIdx(node))?;
        Some(
            attrs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone()),
        )
    }

    fn set_attribute(&mut self, node: u32, name: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        let mut attrs = self
            .dom
            .attrs_of(idx)
            .ok_or_else(|| "attribute requires an element".to_string())?
            .to_vec();
        if let Some((_, current)) = attrs.iter_mut().find(|(key, _)| key == name) {
            *current = value.to_string();
        } else {
            attrs.push((name.to_string(), value.to_string()));
        }
        self.dom.set_attrs(idx, attrs);
        Ok(())
    }

    fn remove_attribute(&mut self, node: u32, name: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        let mut attrs = self
            .dom
            .attrs_of(idx)
            .ok_or_else(|| "attribute requires an element".to_string())?
            .to_vec();
        attrs.retain(|(key, _)| key != name);
        self.dom.set_attrs(idx, attrs);
        Ok(())
    }

    fn create_element(&mut self, name: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(amazing_dom::NodeKind::Element {
                name: name.to_ascii_lowercase(),
                attrs: Vec::new(),
                namespace: amazing_dom::Namespace::Html,
            })
            .0)
    }

    fn create_text_node(&mut self, text: &str) -> Result<u32, String> {
        Ok(self
            .dom
            .alloc_unattached(amazing_dom::NodeKind::Text {
                data: text.to_owned(),
            })
            .0)
    }

    fn append_child(&mut self, parent: u32, child: u32) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        let child = amazing_dom::NodeIdx(child);
        if parent.0 as usize >= self.dom.node_count() || child.0 as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        if self.dom.name(parent).is_none() {
            return Err("append target is not an element".into());
        }
        let mut ancestor = Some(parent);
        while let Some(current) = ancestor {
            if current == child {
                return Err("cannot append an ancestor into its descendant".into());
            }
            ancestor = self.dom.get(current).parent;
        }
        if self.dom.get(child).parent.is_some() {
            self.dom.detach(child);
        }
        self.dom.attach_last(parent, child);
        Ok(())
    }

    fn append_text(&mut self, parent: u32, text: &str) -> Result<(), String> {
        let parent = amazing_dom::NodeIdx(parent);
        if parent.0 as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        if self.dom.name(parent).is_none() {
            return Err("append target is not an element".into());
        }
        self.dom.create_text(parent, text);
        Ok(())
    }

    fn remove_node(&mut self, node: u32) -> Result<(), String> {
        if node as usize >= self.dom.node_count() {
            return Err("unknown DOM node".into());
        }
        let idx = amazing_dom::NodeIdx(node);
        if idx.0 == 0 {
            return Err("cannot remove the document node".into());
        }
        self.dom.detach(idx);
        Ok(())
    }

    fn get_style(&self, node: u32, property: &str) -> Option<String> {
        if node as usize >= self.dom.node_count()
            || self.dom.name(amazing_dom::NodeIdx(node)).is_none()
        {
            return None;
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        if property == "csstext" {
            return self
                .dom
                .attrs_of(amazing_dom::NodeIdx(node))
                .and_then(|attrs| attrs.iter().find(|(key, _)| key == "style"))
                .map(|(_, value)| value.clone())
                .or_else(|| Some(String::new()));
        }
        self.style_declarations(amazing_dom::NodeIdx(node))
            .into_iter()
            .find(|(name, _)| name == &property)
            .map(|(_, value)| value)
    }

    fn set_style(&mut self, node: u32, property: &str, value: &str) -> Result<(), String> {
        let idx = amazing_dom::NodeIdx(node);
        if node as usize >= self.dom.node_count() || self.dom.name(idx).is_none() {
            return Err("style requires an element".into());
        }
        if property.eq_ignore_ascii_case("cssText") {
            return self.set_attribute(node, "style", value);
        }
        let property = BrowserDomHost::css_property_name(property).to_ascii_lowercase();
        let mut declarations = self.style_declarations(idx);
        if let Some((_, current)) = declarations.iter_mut().find(|(name, _)| name == &property) {
            *current = value.trim().to_string();
        } else {
            declarations.push((property, value.trim().to_string()));
        }
        self.write_style_declarations(idx, &declarations)
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    #[cfg(windows)]
    if args.len() == 1 {
        return gui::run(None);
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
        Some("parse") => cmd_parse(args.get(2).map(String::as_str)),
        Some("parse-fragment") => match args.get(2) {
            Some(context) => cmd_parse_fragment(context, args.get(3).map(String::as_str)),
            None => {
                eprintln!("usage: amazing parse-fragment <context> [file]");
                ExitCode::FAILURE
            }
        },
        Some("style") => cmd_style(&args[2..]),
        Some("layout") => cmd_layout(&args[2..]),
        Some("render") => cmd_render(&args[2..]),
        Some("audio-info") => cmd_audio_info(&args[2..]),
        Some("play-audio") => cmd_play_audio(&args[2..]),
        Some("webrtc-loopback") => cmd_webrtc_loopback(),
        Some("rtp-jitter-probe") => cmd_rtp_jitter_probe(),
        Some("opus-probe") => cmd_opus_probe(),
        Some("video-frame-probe") => cmd_video_frame_probe(),
        Some("vp8-decode-probe") => cmd_vp8_decode_probe(),
        Some("h264-roundtrip-probe") => cmd_h264_roundtrip_probe(),
        Some("secure-media-loopback") => cmd_secure_media_loopback(),
        Some("secure-media-processes") => cmd_secure_media_processes(),
        Some("__secure-media-peer") => cmd_secure_media_peer(args.get(2).map(String::as_str)),
        Some("gpu-info") => cmd_gpu_info(),
        Some("gpu-present-smoke") => cmd_gpu_present_smoke(),
        Some("sdp-roundtrip") => cmd_sdp_roundtrip(&args[2..]),
        Some("fetch") => cmd_fetch(&args[2..]),
        Some("fetch-parse") => cmd_fetch_parse(&args[2..]),
        Some("run") => cmd_run(args.get(2).map(String::as_str)),
        Some("domrun") => cmd_domrun(&args[2..]),
        Some("gui") => gui::run(args.get(2).map(String::as_str)),
        Some(other) => {
            eprintln!("unknown command: {other}");
            eprintln!(
                "usage: amazing [version|parse [file]|parse-fragment <context> [file]|style <page.html> [extra.css…]|layout <page.html> [viewport_width]|render <page.html> <out.bmp> [width]|fetch <url|file>|fetch-parse <http-url>|audio-info <file.wav>|play-audio <file.wav>|webrtc-loopback|rtp-jitter-probe|opus-probe|video-frame-probe|vp8-decode-probe|h264-roundtrip-probe|secure-media-loopback|secure-media-processes|sdp-roundtrip <offer.sdp>|gpu-info|gpu-present-smoke|run [script.js]|domrun <page.html> <script.js>|gui [url]]"
            );
            ExitCode::FAILURE
        }
    }
}

fn cmd_rtp_jitter_probe() -> ExitCode {
    match amazing_webrtc::run_jitter_codec_probe() {
        Ok(report) => {
            println!(
                "RTP jitter packets={} reordered={} duplicate={} codecs=L16+PCMU samples={}+{}; jitter/codec probe ok",
                report.emitted_packets,
                if report.reordered { "yes" } else { "no" },
                if report.duplicate_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                report.l16_samples,
                report.pcmu_samples,
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing rtp-jitter-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_opus_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_opus_probe() {
        Ok(report) => {
            println!(
                "Opus encoded={} bytes decoded={} samples PLC={} samples; opus probe ok",
                report.encoded_bytes, report.decoded_samples, report.concealed_samples
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing opus-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_video_frame_probe() -> ExitCode {
    match amazing_webrtc::run_video_frame_probe() {
        Ok(report) => {
            println!(
                "video RTP packets={} frame={} bytes reordered={} damaged={}; video frame probe ok",
                report.packets,
                report.frame_bytes,
                if report.reordered { "yes" } else { "no" },
                if report.damaged_dropped {
                    "dropped"
                } else {
                    "accepted"
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing video-frame-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_vp8_decode_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_vp8_probe(include_bytes!(
        "../../../examples/vp80-02-inter-1402.ivf"
    )) {
        Ok(report) => {
            println!(
                "VP8 frames={} interframes={} size={}x{} pixels_changed={}; vp8 decode probe ok",
                report.frames,
                report.interframes,
                report.width,
                report.height,
                if report.checksum_changed { "yes" } else { "no" },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing vp8-decode-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_h264_roundtrip_probe() -> ExitCode {
    match amazing_webrtc_codecs::run_h264_probe() {
        Ok(report) => {
            println!(
                "H264 first={} bytes delta={} bytes size={}x{} pixels_changed={}; h264 roundtrip probe ok",
                report.first_bytes,
                report.second_bytes,
                report.width,
                report.height,
                if report.pixels_changed { "yes" } else { "no" },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing h264-roundtrip-probe: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.9 artifact: vetted DTLS 1.2 key agreement and SRTP/SRTCP proof.
fn cmd_secure_media_loopback() -> ExitCode {
    match amazing_webrtc_crypto::run_secure_loopback() {
        Ok(report) => {
            println!(
                "ICE state={} credentials={}; UDP transport={} -> {}; SDP identity={} roles={}/{}; {} {}; SRTP profile={} keying={} bytes; fingerprints={}; SRTP payload={} bytes protected={} bytes; SRTCP packets={} protected={} bytes; SRTP tamper={} replay={}; SRTCP tamper={} replay={}; secure loopback ok",
                report.ice_state,
                if report.ice_credentials_bound {
                    "SDP"
                } else {
                    "local"
                },
                report.udp_client_address,
                report.udp_server_address,
                if report.sdp_identity_verified {
                    "ok"
                } else {
                    "failed"
                },
                report.client_setup,
                report.server_setup,
                report.dtls_version,
                report.cipher_suite,
                report.srtp_profile,
                report.exported_keying_material_bytes,
                if report.fingerprints_verified {
                    "ok"
                } else {
                    "failed"
                },
                report.rtp_payload_bytes,
                report.protected_rtp_bytes,
                report.rtcp_packet_count,
                report.protected_rtcp_bytes,
                if report.srtp_tamper_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtp_replay_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtcp_tamper_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
                if report.srtcp_replay_rejected {
                    "rejected"
                } else {
                    "accepted"
                },
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing secure-media-loopback: {error}");
            ExitCode::FAILURE
        }
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child guard is active")
    }

    fn finish(mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child_mut().try_wait() {
                Ok(Some(status)) => {
                    self.child.take();
                    return Ok(status);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let child = self.child_mut();
                    let _ = child.kill();
                    let reap = child.wait();
                    return Err(match reap {
                        Ok(_) => {
                            self.child.take();
                            "secure peer completion timed out; child killed and reaped".into()
                        }
                        Err(error) => format!(
                            "secure peer completion timed out; reap after kill failed and guard will retry: {error}"
                        ),
                    });
                }
                Err(wait_error) => {
                    let child = self.child_mut();
                    let _ = child.kill();
                    let reap = child.wait();
                    return Err(match reap {
                        Ok(_) => {
                            self.child.take();
                            format!("poll secure peer: {wait_error}; child killed and reaped")
                        }
                        Err(reap_error) => format!(
                            "poll secure peer: {wait_error}; reap after kill failed and guard will retry: {reap_error}"
                        ),
                    });
                }
            }
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn cmd_secure_media_processes() -> ExitCode {
    match run_secure_media_processes() {
        Ok(()) => {
            println!(
                "processes=2 signaling=tcp-loopback; ICE credentials=SDP; codecs=SDP Opus+L16+PCMU+H264; ICE live-loss=2 sends=3 487=resolved prflx=learned; consent=Fresh expiry=detected; RTP parent->child=ok; RTP child->parent=ok; playout=adaptive loss=1 conceal=160 Opus parent->child=960 PCMU child->parent=320; video=H264 secure frames=6 32x32 clock=90k changed=yes; SRTCP SR+CNAME child->parent=ok; RTCP RR+CNAME parent->child=ok; RTCP stats=computed; two-process interop ok"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing secure-media-processes: {error}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_secure_media_peer(control_address: Option<&str>) -> ExitCode {
    match run_secure_media_peer(control_address) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("amazing secure-media-peer: {error}");
            ExitCode::FAILURE
        }
    }
}

fn verify_ice_robustness_probe() -> Result<(), String> {
    let replacement_tiebreaker =
        amazing_webrtc_crypto::secure_random_ice_tiebreaker().map_err(|error| error.to_string())?;
    let report = amazing_webrtc::run_live_ice_robustness_probe(replacement_tiebreaker)
        .map_err(|error| error.to_string())?;
    if report.request_sends != 3
        || report.injected_drops != 2
        || !report.role_switched
        || !report.peer_reflexive_inserted
    {
        return Err("live ICE robustness probe failed".into());
    }
    Ok(())
}

fn negotiated_audio_decoder(
    codecs: &amazing_webrtc::NegotiatedAudioCodecs,
    payload_type: u8,
) -> Result<Box<dyn amazing_webrtc::RtpAudioDecoder>, String> {
    let codec = codecs
        .codec(payload_type)
        .map_err(|error| error.to_string())?;
    if codec.encoding.eq_ignore_ascii_case("OPUS") {
        Ok(Box::new(
            amazing_webrtc_codecs::OpusDecoderAdapter::from_negotiated(codec)
                .map_err(|error| error.to_string())?,
        ))
    } else {
        codecs
            .decoder(payload_type)
            .map_err(|error| error.to_string())
    }
}

fn negotiated_video_media<'a>(
    offer: &'a amazing_webrtc::SdpDescription,
    answer: &amazing_webrtc::SdpDescription,
    payload_type: u8,
) -> Result<&'a amazing_webrtc::SdpMediaSection, String> {
    let offered = offer
        .media_for_payload_type(payload_type)
        .map_err(|error| error.to_string())?;
    let accepted = answer
        .media_for_payload_type(payload_type)
        .map_err(|error| error.to_string())?;
    let offered_map = offered
        .rtp_maps
        .iter()
        .find(|mapping| mapping.payload_type == payload_type)
        .ok_or_else(|| "offered video rtpmap is missing".to_string())?;
    let accepted_map = accepted
        .rtp_maps
        .iter()
        .find(|mapping| mapping.payload_type == payload_type)
        .ok_or_else(|| "answered video rtpmap is missing".to_string())?;
    if offered.media != "video"
        || accepted.media != "video"
        || offered.mid != accepted.mid
        || offered_map != accepted_map
    {
        return Err("video offer/answer mapping changed".into());
    }
    Ok(offered)
}

fn run_secure_media_processes() -> Result<(), String> {
    verify_ice_robustness_probe()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("bind signaling listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("configure signaling listener: {error}"))?;
    let signaling_address = listener.local_addr().map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let child = Command::new(executable)
        .arg("__secure-media-peer")
        .arg(signaling_address.to_string())
        .spawn()
        .map_err(|error| format!("spawn secure peer: {error}"))?;
    let mut child = ChildGuard::new(child);
    let mut control = accept_control_connection(&listener, &mut child, Duration::from_secs(5))?;
    configure_control_stream(&control)?;
    let mut control_reader =
        BufReader::new(control.try_clone().map_err(|error| error.to_string())?);
    let offer = read_signaled_description(&mut control_reader, "OFFER")?;

    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| format!("bind parent UDP: {error}"))?;
    let parent_address = std_socket_v4(&socket)?;
    let child_address = description_candidate(&offer)?;
    socket
        .connect(child_address)
        .map_err(|error| format!("connect parent UDP: {error}"))?;
    let identity = amazing_webrtc_crypto::SecureIdentity::generate("ferric-parent")
        .map_err(|error| error.to_string())?;
    let answer = amazing_webrtc_crypto::build_local_description(
        parent_address,
        identity.fingerprint(),
        "active",
        "parenthost",
        "parent",
        "FerricParentPassword123456",
        400,
    )
    .map_err(|error| error.to_string())?;
    write_signaled_description(&mut control, "ANSWER", &answer)?;
    let negotiated_codecs = amazing_webrtc::negotiate_audio_codecs(&offer, &answer)
        .map_err(|error| error.to_string())?;
    negotiated_video_media(&offer, &answer, 121)?;

    let mut endpoint = amazing_webrtc_crypto::SecureUdpEndpoint::establish(
        amazing_webrtc_crypto::SecureEndpointConfig {
            socket,
            identity,
            local_description: answer,
            remote_description: offer,
            local_is_offerer: false,
            ice_role: amazing_webrtc_crypto::IceRole::Controlling,
        },
    )
    .map_err(|error| error.to_string())?;
    let opus_codec = negotiated_codecs
        .codec(111)
        .map_err(|error| error.to_string())?;
    let mut opus_encoder = amazing_webrtc_codecs::OpusEncoderAdapter::from_negotiated(opus_codec)
        .map_err(|error| error.to_string())?;
    let opus_pcm: Vec<i16> = (0..960)
        .map(|index| (((index % 96) - 48) * 400) as i16)
        .collect();
    let outbound = opus_encoder
        .encode(41, 960, 0x5041_5245, &opus_pcm)
        .map_err(|error| error.to_string())?;
    let outbound_wire = amazing_webrtc::encode_rtp(&outbound).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&outbound_wire)
        .map_err(|error| error.to_string())?;
    endpoint
        .wait_for_consent_due()
        .map_err(|error| error.to_string())?;
    if endpoint
        .refresh_consent()
        .map_err(|error| error.to_string())?
        != amazing_webrtc::ConsentState::Fresh
    {
        return Err("live consent refresh did not remain Fresh".into());
    }
    let expiry_probe =
        amazing_webrtc::ConsentFreshness::new(0, 5_000).map_err(|error| error.to_string())?;
    if expiry_probe.state(30_000) != amazing_webrtc::ConsentState::Expired {
        return Err("consent expiry probe did not expire at thirty seconds".into());
    }
    let child_rtp = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let child_rtp = amazing_webrtc::parse_rtp(&child_rtp).map_err(|error| error.to_string())?;
    let child_rtp_later = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let child_rtp_later =
        amazing_webrtc::parse_rtp(&child_rtp_later).map_err(|error| error.to_string())?;
    if child_rtp.payload_type != 0
        || child_rtp.sequence_number != 84
        || child_rtp_later.sequence_number != 86
        || child_rtp_later.ssrc != child_rtp.ssrc
    {
        return Err("child process returned unexpected RTP".into());
    }
    let child_config = negotiated_codecs
        .jitter_config(child_rtp.payload_type, child_rtp.ssrc, 60, 8)
        .map_err(|error| error.to_string())?;
    let mut child_jitter = amazing_webrtc::RtpJitterBuffer::new_adaptive(
        child_config,
        amazing_webrtc::AdaptivePlayoutConfig {
            max_delay_ms: 200,
            jitter_multiplier: 4,
        },
    )
    .map_err(|error| error.to_string())?;
    child_jitter
        .push(child_rtp.clone(), 0)
        .map_err(|error| error.to_string())?;
    child_jitter
        .push(child_rtp_later.clone(), 5)
        .map_err(|error| error.to_string())?;
    let first_playout = child_jitter.target_delay_ms();
    let Some(amazing_webrtc::JitterOutput::Packet(played_child_rtp)) =
        child_jitter.pop_ready(first_playout)
    else {
        return Err("child RTP did not reach jitter playout".into());
    };
    let mut child_decoder = negotiated_audio_decoder(&negotiated_codecs, child_rtp.payload_type)?;
    let child_audio = child_decoder
        .decode(&played_child_rtp)
        .map_err(|error| error.to_string())?;
    if child_audio.samples.len() != 160 {
        return Err("child audio playout sample count was incorrect".into());
    }
    let Some(amazing_webrtc::JitterOutput::Gap {
        estimated_timestamp: Some(missing_timestamp),
        duration_timestamp_units: Some(missing_samples),
        ..
    }) = child_jitter.pop_ready(first_playout + 40)
    else {
        return Err("child RTP loss did not reach jitter concealment".into());
    };
    let concealed = child_decoder
        .conceal(missing_timestamp, missing_samples as usize)
        .map_err(|error| error.to_string())?;
    if concealed.samples != vec![0; 160] {
        return Err("child PCMU concealment samples were incorrect".into());
    }
    let Some(amazing_webrtc::JitterOutput::Packet(played_child_later)) =
        child_jitter.pop_ready(first_playout + 40)
    else {
        return Err("child RTP after loss did not reach jitter playout".into());
    };
    let later_audio = child_decoder
        .decode(&played_child_later)
        .map_err(|error| error.to_string())?;
    if later_audio.samples != vec![32_124; 160] || child_jitter.stats().gaps != 1 {
        return Err("child PCMU post-loss samples were incorrect".into());
    }
    let mut reception_stats = amazing_webrtc::RtpReceptionStats::new(child_rtp.ssrc);
    reception_stats
        .observe(&child_rtp, child_rtp.timestamp)
        .map_err(|error| error.to_string())?;
    reception_stats
        .observe(&child_rtp_later, child_rtp_later.timestamp)
        .map_err(|error| error.to_string())?;
    let child_rtcp = endpoint.receive_rtcp().map_err(|error| error.to_string())?;
    let child_rtcp = amazing_webrtc::parse_rtcp_compound_sender_report(&child_rtcp)
        .map_err(|error| error.to_string())?;
    if child_rtcp.sender.ssrc != child_rtp.ssrc
        || child_rtcp.sender.rtp_timestamp != child_rtp_later.timestamp
        || child_rtcp.sender.packet_count != 2
        || child_rtcp.sender.octet_count != 320
        || child_rtcp.cname != "child@ferric"
    {
        return Err("child process returned inconsistent SRTCP".into());
    }
    read_control_marker(&mut control_reader, "READY_RR")?;
    let reception = reception_stats
        .report((child_rtcp.sender.ntp_timestamp >> 16) as u32, 0)
        .map_err(|error| error.to_string())?;
    let receiver_report = amazing_webrtc::RtcpCompoundReceiverReport {
        receiver_ssrc: 0x5252_4356,
        reception,
        cname: "parent@ferric".into(),
    };
    let receiver_report = amazing_webrtc::encode_rtcp_compound_receiver_report(&receiver_report)
        .map_err(|error| error.to_string())?;
    endpoint
        .send_rtcp(&receiver_report)
        .map_err(|error| format!("send compound receiver report: {error}"))?;

    let mut h264_encoder =
        amazing_webrtc_codecs::H264EncoderAdapter::new().map_err(|error| error.to_string())?;
    for phase in 0..6u16 {
        let mut rgb = Vec::with_capacity(32 * 32 * 3);
        for y in 0..32 {
            for x in 0..32 {
                rgb.extend_from_slice(&[
                    (x * 7 + phase as usize * 13) as u8,
                    (y * 9 + phase as usize * 17) as u8,
                    ((x + y) * 4 + phase as usize * 23) as u8,
                ]);
            }
        }
        let h264 = h264_encoder
            .encode_rgb(&rgb, (32, 32))
            .map_err(|error| error.to_string())?;
        let video_rtp = amazing_webrtc::RtpPacket {
            marker: true,
            payload_type: 121,
            sequence_number: 200 + phase,
            timestamp: 90_000 + phase as u32 * 45_000,
            ssrc: 0x5649_4445,
            payload: h264,
        };
        endpoint
            .send_rtp(&amazing_webrtc::encode_rtp(&video_rtp).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    }

    read_control_marker(&mut control_reader, "PEER_OK")?;
    endpoint
        .close()
        .map_err(|error| format!("close parent secure endpoint: {error}"))?;
    drop(control_reader);
    drop(control);
    let status = child.finish(Duration::from_secs(5))?;
    if !status.success() {
        return Err(format!("secure peer exited {status}"));
    }
    Ok(())
}

fn run_secure_media_peer(control_address: Option<&str>) -> Result<(), String> {
    let control_address =
        control_address.ok_or_else(|| "secure peer requires a control address".to_string())?;
    let control_address = control_address
        .parse()
        .map_err(|error| format!("parse signaling control address: {error}"))?;
    let mut control = TcpStream::connect_timeout(&control_address, Duration::from_secs(5))
        .map_err(|error| format!("connect signaling control: {error}"))?;
    configure_control_stream(&control)?;
    let mut control_reader =
        BufReader::new(control.try_clone().map_err(|error| error.to_string())?);
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .map_err(|error| format!("bind child UDP: {error}"))?;
    let child_address = std_socket_v4(&socket)?;
    let identity = amazing_webrtc_crypto::SecureIdentity::generate("ferric-child")
        .map_err(|error| error.to_string())?;
    let offer = amazing_webrtc_crypto::build_local_description(
        child_address,
        identity.fingerprint(),
        "actpass",
        "childhost",
        "child",
        "FerricChildPassword123456",
        300,
    )
    .map_err(|error| error.to_string())?;
    write_signaled_description(&mut control, "OFFER", &offer)?;
    let answer = read_signaled_description(&mut control_reader, "ANSWER")?;
    let negotiated_codecs = amazing_webrtc::negotiate_audio_codecs(&offer, &answer)
        .map_err(|error| error.to_string())?;
    let negotiated_video = negotiated_video_media(&offer, &answer, 121)?.clone();
    socket
        .connect(description_candidate(&answer)?)
        .map_err(|error| format!("connect child UDP: {error}"))?;
    let mut endpoint = amazing_webrtc_crypto::SecureUdpEndpoint::establish(
        amazing_webrtc_crypto::SecureEndpointConfig {
            socket,
            identity,
            local_description: offer,
            remote_description: answer,
            local_is_offerer: true,
            ice_role: amazing_webrtc_crypto::IceRole::Controlled,
        },
    )
    .map_err(|error| error.to_string())?;
    let parent_rtp = endpoint.receive_rtp().map_err(|error| error.to_string())?;
    let parent_rtp = amazing_webrtc::parse_rtp(&parent_rtp).map_err(|error| error.to_string())?;
    if parent_rtp.payload_type != 111 || parent_rtp.sequence_number != 41 {
        return Err("parent process sent unexpected RTP".into());
    }
    let parent_config = negotiated_codecs
        .jitter_config(parent_rtp.payload_type, parent_rtp.ssrc, 60, 8)
        .map_err(|error| error.to_string())?;
    let mut parent_jitter = amazing_webrtc::RtpJitterBuffer::new_adaptive(
        parent_config,
        amazing_webrtc::AdaptivePlayoutConfig {
            max_delay_ms: 200,
            jitter_multiplier: 4,
        },
    )
    .map_err(|error| error.to_string())?;
    parent_jitter
        .push(parent_rtp.clone(), 0)
        .map_err(|error| error.to_string())?;
    let Some(amazing_webrtc::JitterOutput::Packet(played_parent_rtp)) = parent_jitter.pop_ready(60)
    else {
        return Err("parent RTP did not reach jitter playout".into());
    };
    let mut parent_decoder = negotiated_audio_decoder(&negotiated_codecs, parent_rtp.payload_type)?;
    let parent_audio = parent_decoder
        .decode(&played_parent_rtp)
        .map_err(|error| error.to_string())?;
    if parent_audio.samples.len() != 960 {
        return Err("parent Opus playout sample count was incorrect".into());
    }
    endpoint
        .answer_consent()
        .map_err(|error| error.to_string())?;
    let response = amazing_webrtc::RtpPacket {
        marker: true,
        payload_type: 0,
        sequence_number: 84,
        timestamp: 1_920,
        ssrc: 0x4348_494c,
        payload: vec![0xff; 160],
    };
    let response_wire = amazing_webrtc::encode_rtp(&response).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&response_wire)
        .map_err(|error| error.to_string())?;
    let response_later = amazing_webrtc::RtpPacket {
        sequence_number: 86,
        timestamp: 2_240,
        payload: vec![0x80; 160],
        ..response.clone()
    };
    let response_later_wire =
        amazing_webrtc::encode_rtp(&response_later).map_err(|error| error.to_string())?;
    endpoint
        .send_rtp(&response_later_wire)
        .map_err(|error| error.to_string())?;
    let report = amazing_webrtc::RtcpSenderReport {
        ssrc: response.ssrc,
        ntp_timestamp: 0xe123_4567_89ab_cdef,
        rtp_timestamp: response_later.timestamp,
        packet_count: 2,
        octet_count: (response.payload.len() + response_later.payload.len()) as u32,
    };
    let compound_sender = amazing_webrtc::RtcpCompoundSenderReport {
        sender: report,
        cname: "child@ferric".into(),
    };
    let compound_sender = amazing_webrtc::encode_rtcp_compound_sender_report(&compound_sender)
        .map_err(|error| error.to_string())?;
    endpoint
        .send_rtcp(&compound_sender)
        .map_err(|error| error.to_string())?;
    writeln!(control, "READY_RR").map_err(|error| error.to_string())?;
    control.flush().map_err(|error| error.to_string())?;
    let receiver_report = match endpoint.receive_rtcp() {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(
                control,
                "PEER_ERROR receive compound receiver report: {error}"
            );
            let _ = control.flush();
            return Err(format!("receive compound receiver report: {error}"));
        }
    };
    let receiver_report =
        match amazing_webrtc::parse_rtcp_compound_receiver_report(&receiver_report) {
            Ok(report) => report,
            Err(error) => {
                let _ = writeln!(
                    control,
                    "PEER_ERROR parse compound receiver report: {error}"
                );
                let _ = control.flush();
                return Err(error.to_string());
            }
        };
    if receiver_report.reception.source_ssrc != response.ssrc
        || receiver_report.reception.extended_highest_sequence
            != response_later.sequence_number as u32
        || receiver_report.reception.cumulative_lost != 1
        || receiver_report.reception.last_sender_report != (report.ntp_timestamp >> 16) as u32
        || receiver_report.cname != "parent@ferric"
    {
        return Err("parent process returned inconsistent compound RTCP".into());
    }
    if negotiated_video.mid != "1" || negotiated_video.media != "video" {
        return Err("H.264 payload did not demultiplex to bundled video".into());
    }
    let video_clock = negotiated_video
        .rtp_maps
        .iter()
        .find(|mapping| mapping.payload_type == 121)
        .map(|mapping| mapping.clock_rate)
        .ok_or_else(|| "H.264 rtpmap is missing".to_string())?;
    let mut video_jitter =
        amazing_webrtc::RtpJitterBuffer::new(amazing_webrtc::JitterBufferConfig {
            ssrc: 0x5649_4445,
            payload_type: 121,
            clock_rate: video_clock,
            playout_delay_ms: 30,
            capacity: 16,
        })
        .map_err(|error| error.to_string())?;
    for phase in 0..6u16 {
        let video_wire = endpoint.receive_rtp().map_err(|error| error.to_string())?;
        let video_rtp =
            amazing_webrtc::parse_rtp(&video_wire).map_err(|error| error.to_string())?;
        if video_rtp.payload_type != 121
            || video_rtp.ssrc != 0x5649_4445
            || video_rtp.sequence_number != 200 + phase
            || video_rtp.timestamp != 90_000 + phase as u32 * 45_000
        {
            return Err("secure H.264 RTP sequence or timestamp was incorrect".into());
        }
        video_jitter
            .push(video_rtp, phase as u64 * 5)
            .map_err(|error| error.to_string())?;
    }
    let mut assembler =
        amazing_webrtc::VideoFrameAssembler::new(amazing_webrtc::VideoFrameConfig {
            ssrc: 0x5649_4445,
            payload_type: 121,
            max_packets: 8,
            max_bytes: 1_000_000,
        })
        .map_err(|error| error.to_string())?;
    let mut decoder =
        amazing_webrtc_codecs::H264DecoderAdapter::new().map_err(|error| error.to_string())?;
    let mut playout = amazing_webrtc::VideoPlayoutQueue::new(amazing_webrtc::VideoPlayoutConfig {
        clock_rate: video_clock,
        delay_ms: 30,
        capacity: 8,
    })
    .map_err(|error| error.to_string())?;
    while let Some(output) = video_jitter.pop_ready(3_000) {
        if let amazing_webrtc::VideoAssembly::Frame(frame) =
            assembler.push(output).map_err(|error| error.to_string())?
        {
            let timestamp = frame.timestamp;
            let decoded = decoder
                .decode(&frame.payload)
                .map_err(|error| error.to_string())?;
            if playout.push(timestamp, decoded, 0) != amazing_webrtc::VideoQueuePush::Queued {
                return Err("secure decoded video frame was not queued".into());
            }
        }
    }
    let mut decoded_frames = 0usize;
    let mut first_checksum = None;
    let mut pixels_changed = false;
    let mut previous_timestamp = None;
    while let Some(frame) = playout.pop_due(3_000) {
        if previous_timestamp.is_some_and(|previous| frame.timestamp <= previous)
            || (frame.frame.width, frame.frame.height) != (32, 32)
            || frame.frame.planes != 3
            || frame.frame.rgba.len() != 32 * 32 * 4
        {
            return Err("secure H.264 playout frame was invalid".into());
        }
        previous_timestamp = Some(frame.timestamp);
        match first_checksum {
            Some(first) if first != frame.frame.checksum => pixels_changed = true,
            None => first_checksum = Some(frame.frame.checksum),
            _ => {}
        }
        decoded_frames += 1;
    }
    if decoded_frames != 6 || !pixels_changed || playout.stats().emitted != 6 {
        return Err("secure H.264 clocked playout was incomplete".into());
    }
    endpoint
        .close()
        .map_err(|error| format!("close child secure endpoint: {error}"))?;
    writeln!(control, "PEER_OK").map_err(|error| error.to_string())?;
    control.flush().map_err(|error| error.to_string())?;
    Ok(())
}

fn accept_control_connection(
    listener: &TcpListener,
    child: &mut ChildGuard,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("accept signaling control: {error}")),
        }
        if let Some(status) = child
            .child_mut()
            .try_wait()
            .map_err(|error| format!("poll secure peer before signaling: {error}"))?
        {
            return Err(format!(
                "secure peer exited before signaling connection: {status}"
            ));
        }
        if Instant::now() >= deadline {
            return Err("secure peer signaling connection timed out".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn configure_control_stream(stream: &TcpStream) -> Result<(), String> {
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("configure signaling blocking mode: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("configure signaling read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("configure signaling write timeout: {error}"))
}

fn read_control_marker(reader: &mut impl BufRead, expected: &str) -> Result<(), String> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| format!("read control marker {expected}: {error}"))?;
    if line.trim() != expected {
        return Err(format!(
            "expected control marker {expected}, received {line:?}"
        ));
    }
    Ok(())
}

fn write_signaled_description(
    writer: &mut impl Write,
    label: &str,
    description: &amazing_webrtc::SdpDescription,
) -> Result<(), String> {
    let sdp = amazing_webrtc::encode_sdp(description).map_err(|error| error.to_string())?;
    writeln!(writer, "{label} {}", hex_encode(sdp.as_bytes()))
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn read_signaled_description(
    reader: &mut impl BufRead,
    label: &str,
) -> Result<amazing_webrtc::SdpDescription, String> {
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err(format!("missing {label} signaling line"));
    }
    let encoded = line
        .trim_end()
        .strip_prefix(&format!("{label} "))
        .ok_or_else(|| format!("invalid {label} signaling line"))?;
    let bytes = hex_decode(encoded)?;
    let sdp = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    amazing_webrtc::parse_sdp(&sdp).map_err(|error| error.to_string())
}

fn description_candidate(
    description: &amazing_webrtc::SdpDescription,
) -> Result<std::net::SocketAddrV4, String> {
    description
        .primary_media()
        .map_err(|error| error.to_string())?
        .candidates
        .first()
        .map(|candidate| candidate.address)
        .ok_or_else(|| "signaled SDP has no ICE candidate".to_string())
}

fn std_socket_v4(socket: &std::net::UdpSocket) -> Result<std::net::SocketAddrV4, String> {
    match socket.local_addr().map_err(|error| error.to_string())? {
        std::net::SocketAddr::V4(address) => Ok(address),
        std::net::SocketAddr::V6(_) => Err("secure process artifact requires IPv4".into()),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err("signaling hex has invalid length or encoding".into());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("ASCII checked");
            u8::from_str_radix(pair, 16).map_err(|_| "signaling hex is invalid".to_string())
        })
        .collect()
}

/// Reads a file (or stdin when `path` is None) as lossy UTF-8.
fn read_source(path: Option<&str>, what: &str) -> Result<String, ExitCode> {
    let mut bytes = Vec::new();
    match path {
        Some(p) => match std::fs::File::open(p) {
            Ok(mut f) => {
                if let Err(e) = f.read_to_end(&mut bytes) {
                    eprintln!("amazing {what}: cannot read {p}: {e}");
                    return Err(ExitCode::FAILURE);
                }
            }
            Err(e) => {
                eprintln!("amazing {what}: cannot open {p}: {e}");
                return Err(ExitCode::FAILURE);
            }
        },
        None => {
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("amazing {what}: cannot read stdin: {e}");
                return Err(ExitCode::FAILURE);
            }
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Phase 1 demo artifact: parse HTML with the WHATWG tokenizer + tree
/// builder, dump the DOM tree to stdout.
///
/// Bytes are decoded as lossy UTF-8 (documented Phase 1 deviation).
fn cmd_parse(path: Option<&str>) -> ExitCode {
    match read_source(path, "parse") {
        Ok(src) => {
            let dom = amazing_html::parse(&src);
            print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn cmd_parse_fragment(context: &str, path: Option<&str>) -> ExitCode {
    match read_source(path, "parse-fragment") {
        Ok(src) => match amazing_html::parse_fragment(context, &src) {
            Ok(dom) => {
                print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("amazing parse-fragment: {error}");
                ExitCode::FAILURE
            }
        },
        Err(code) => code,
    }
}

/// Phase 2 demo artifact: compute styles for an HTML page.
///
/// Stylesheets come from every `<style>` element in document order plus any
/// extra CSS files given after the page path (`<link rel=stylesheet>` needs
/// fetching, so it waits for Phase 5).
///
/// Output prints each element in tree order as `<tag style="prop:value; …">`
/// containing only the properties whose computed value differs from the
/// parent element's (the readable delta view from `StyledDom`).
fn cmd_style(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing style <page.html> [extra.css…]");
            return ExitCode::FAILURE;
        }
    };

    let html = match read_source(Some(html_path), "style") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = amazing_html::parse(&html);

    // Gather author stylesheets: inline <style> elements first.
    let mut css = String::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut css);
    for extra in &args[1..] {
        match read_source(Some(extra), "style") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let sheet = amazing_css::parse_stylesheet(&css);
    let styled = amazing_css::compute(&dom, &[sheet]);

    print_styled(&dom, amazing_dom::NodeIdx(0), 0, &styled);
    ExitCode::SUCCESS
}

fn collect_style_text(dom: &amazing_dom::Dom, node: amazing_dom::NodeIdx, out: &mut String) {
    if dom.name(node) == Some("style") {
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                out.push_str(data);
                out.push('\n');
            }
        }
        return;
    }
    for child in dom.children(node) {
        collect_style_text(dom, child, out);
    }
}

fn collect_author_css(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    page: &str,
    out: &mut String,
) -> Result<(), String> {
    if dom.name(node) == Some("style") {
        collect_style_text(dom, node, out);
        return Ok(());
    }
    if dom.name(node) == Some("link") {
        let attrs = dom.attrs_of(node).unwrap_or(&[]);
        let rel = attrs
            .iter()
            .find(|(name, _)| name == "rel")
            .map(|(_, value)| value.to_ascii_lowercase())
            .unwrap_or_default();
        if rel
            .split_ascii_whitespace()
            .any(|token| token == "stylesheet")
            && let Some(href) = attrs
                .iter()
                .find(|(name, _)| name == "href")
                .map(|(_, value)| value)
        {
            let url = resolve_script_url(page, href);
            let response = amazing_net::fetch(&url, &amazing_net::FetchOptions::default())
                .map_err(|error| format!("stylesheet {href}: {error}"))?;
            out.push_str(
                &response
                    .text()
                    .ok_or_else(|| format!("stylesheet {href} was not text"))?,
            );
            out.push('\n');
        }
    }
    for child in dom.children(node) {
        collect_author_css(dom, child, page, out)?;
    }
    Ok(())
}

struct PageScript {
    src: Option<String>,
    body: String,
}

fn collect_script_text(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    scripts: &mut Vec<PageScript>,
) {
    if dom.name(node) == Some("script") {
        let mut source = String::new();
        for child in dom.children(node) {
            if let amazing_dom::NodeKind::Text { data } = &dom.get(child).kind {
                source.push_str(data);
            }
        }
        let src = dom
            .attrs_of(node)
            .and_then(|attrs| attrs.iter().find(|(name, _)| name == "src"))
            .map(|(_, value)| value.clone());
        if src.is_some() || !source.trim().is_empty() {
            scripts.push(PageScript { src, body: source });
        }
        return;
    }
    for child in dom.children(node) {
        collect_script_text(dom, child, scripts);
    }
}

fn collect_inline_handlers(
    dom: &amazing_dom::Dom,
    node: amazing_dom::NodeIdx,
    handlers: &mut Vec<(u32, String, String)>,
) {
    if let Some(attrs) = dom.attrs_of(node) {
        for (name, body) in attrs {
            if let Some(event) = name.strip_prefix("on").filter(|event| !event.is_empty()) {
                handlers.push((node.0, event.to_owned(), body.clone()));
            }
        }
    }
    for child in dom.children(node) {
        collect_inline_handlers(dom, child, handlers);
    }
}

fn execute_inline_scripts(host: &Rc<RefCell<BrowserDomHost>>, page: &str) -> Result<(), String> {
    let scripts = {
        let host_ref = host.borrow();
        let mut scripts = Vec::new();
        collect_script_text(&host_ref.dom, amazing_dom::NodeIdx(0), &mut scripts);
        scripts
    };
    if scripts.is_empty() {
        return Ok(());
    }
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    js.set_location_url(page);
    let handlers = {
        let host_ref = host.borrow();
        let mut handlers = Vec::new();
        collect_inline_handlers(&host_ref.dom, amazing_dom::NodeIdx(0), &mut handlers);
        handlers
    };
    for (node, event, body) in handlers {
        js.install_inline_handler(node, &event, &body)
            .map_err(|error| format!("inline onclick failed: {}", error.describe()))?;
    }
    for script in scripts {
        let source = match script.src {
            Some(src) => {
                let script_url = resolve_script_url(page, &src);
                let response =
                    amazing_net::fetch(&script_url, &amazing_net::FetchOptions::default())
                        .map_err(|error| format!("external script {src}: {error}"))?;
                response
                    .text()
                    .ok_or_else(|| format!("external script {src} was not text"))?
            }
            None => script.body,
        };
        js.run_script(&source)
            .map_err(|error| format!("page script failed: {}", error.describe()))?;
    }
    js.dispatch_dom_event(0, "DOMContentLoaded")
        .map_err(|error| format!("DOMContentLoaded listener failed: {}", error.describe()))?;
    js.dispatch_dom_event(0, "load")
        .map_err(|error| format!("load listener failed: {}", error.describe()))?;
    let body = host.borrow_mut().query_selector("body");
    if let Some(body) = body {
        js.dispatch_dom_event(body, "load")
            .map_err(|error| format!("body load listener failed: {}", error.describe()))?;
    }
    Ok(())
}

fn resolve_script_url(page: &str, source: &str) -> String {
    if source.contains("://") || source.starts_with("file:") {
        return source.to_owned();
    }
    if (page.starts_with("http://") || page.starts_with("https://") || page.starts_with("file:"))
        && let Ok(base) = amazing_net::Url::parse(page)
        && let Ok(resolved) = amazing_net::resolve(source, &base)
    {
        return resolved.to_string();
    }
    std::path::Path::new(page)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(source)
        .to_string_lossy()
        .into_owned()
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
fn cmd_layout(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing layout <page.html> [viewport_width]");
            return ExitCode::FAILURE;
        }
    };
    let width: f32 = args.get(1).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_source(Some(html_path), "layout") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let dom = amazing_html::parse(&html);

    // Author CSS: <style> elements + extra files (same policy as `style`).
    let mut css = String::new();
    collect_style_text(&dom, amazing_dom::NodeIdx(0), &mut css);
    for extra in args.get(2..).unwrap_or(&[]) {
        match read_source(Some(extra), "layout") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let fonts = amazing_layout::font::FontSource::fallback();
    let root = amazing_layout::layout_document(&html, &css, width, &fonts);
    print!("{}", amazing_layout::dump_layout(&dom, &root));
    ExitCode::SUCCESS
}

/// Phase 4 demo: `amazing render <page.html> <out.bmp> [viewport_width]`
/// (plus optional extra CSS files after the width). Lays out the page,
/// rasterizes it into the software framebuffer, and writes a 24-bit BMP
/// alongside a PPM twin (`<out>.ppm`).
fn cmd_render(args: &[String]) -> ExitCode {
    let html_path = match args.first() {
        Some(p) => p,
        None => {
            eprintln!("usage: amazing render <page.html> <out.bmp> [viewport_width] [extra.css…]");
            return ExitCode::FAILURE;
        }
    };
    let out_path = args.get(1).map(String::as_str).unwrap_or("render.bmp");
    let width: f32 = args.get(2).and_then(|w| w.parse().ok()).unwrap_or(800.0);

    let html = match read_source(Some(html_path), "render") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(&html),
    }));
    if let Err(error) = execute_inline_scripts(&host, html_path) {
        eprintln!("amazing render: {error}");
        return ExitCode::FAILURE;
    }

    let mut css = String::new();
    let dom = host.borrow();
    if let Err(error) = collect_author_css(&dom.dom, amazing_dom::NodeIdx(0), html_path, &mut css) {
        eprintln!("amazing render: {error}");
        return ExitCode::FAILURE;
    }
    for extra in args.get(3..).unwrap_or(&[]) {
        match read_source(Some(extra), "render") {
            Ok(s) => {
                css.push('\n');
                css.push_str(&s);
            }
            Err(code) => return code,
        }
    }

    let fonts = amazing_layout::font::FontSource::fallback();
    let media = load_page_media(&dom.dom, html_path);
    for warning in &media.warnings {
        eprintln!("amazing render: {warning}");
    }
    let root =
        amazing_layout::layout_dom_document_with_replaced(&dom.dom, &css, width, &fonts, &media);
    // Page height = root margin-box bottom edge (what layout_root returns),
    // clamped so a runaway document cannot exhaust memory.
    let page_h = (root.border_box.y + root.border_box.h + root.margin.bottom.unwrap_or(0.0))
        .ceil()
        .clamp(1.0, 8192.0) as u32;
    let surface = amazing_paint::paint_document_with_resources(
        &root,
        width.max(1.0) as u32,
        page_h,
        &fonts,
        &amazing_paint::RectInkProvider,
        &media,
    );

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

/// Phase 5 demo: `amazing fetch <url|file|path> [-i]`. Performs an HTTP/1.1
/// GET (or reads a file:// / bare path), prints status + headers + body.
/// `-i` includes the response headers.
fn cmd_fetch(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(p) if !p.starts_with('-') => p,
        _ => {
            eprintln!("usage: amazing fetch <url|file> [-i]");
            return ExitCode::FAILURE;
        }
    };
    let show_headers = args.iter().any(|a| a == "-i");

    match amazing_net::fetch(target, &amazing_net::FetchOptions::default()) {
        Ok(resp) => {
            println!("{} {} ({})", resp.status, resp.reason, resp.url);
            if show_headers {
                for (name, value) in resp.headers.iter() {
                    println!("{name}: {value}");
                }
                println!();
            }
            print!("{}", resp.text().unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("amazing fetch: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 5 integration artifact: fetch an HTTP response and feed its decoded
/// body directly into the Phase 1 parser before dumping the resulting DOM.
fn cmd_fetch_parse(args: &[String]) -> ExitCode {
    let target = match args.first() {
        Some(target) if target.starts_with("http://") => target,
        _ => {
            eprintln!("usage: amazing fetch-parse <http-url>");
            return ExitCode::FAILURE;
        }
    };
    let response = match amazing_net::fetch(target, &amazing_net::FetchOptions::default()) {
        Ok(response) if response.is_success() => response,
        Ok(response) => {
            eprintln!("amazing fetch-parse: HTTP status {}", response.status);
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("amazing fetch-parse: {error}");
            return ExitCode::FAILURE;
        }
    };
    let source = response.text().unwrap_or_default();
    let dom = amazing_html::parse(&source);
    print!("{}", dom.dump(amazing_dom::NodeIdx(0)));
    ExitCode::SUCCESS
}

/// Phase 9.2 audio artifact: decode PCM WAV and report stream properties.
fn cmd_audio_info(args: &[String]) -> ExitCode {
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
fn cmd_play_audio(args: &[String]) -> ExitCode {
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

/// Phase 9.3 artifact: STUN discovery followed by one RTP echo over UDP.
fn cmd_webrtc_loopback() -> ExitCode {
    fn run() -> Result<
        (
            std::net::SocketAddrV4,
            amazing_webrtc::RtpPacket,
            amazing_webrtc::ChecklistState,
            amazing_webrtc::RtcpSenderReport,
        ),
        String,
    > {
        use std::net::{SocketAddr, UdpSocket};
        use std::time::Duration;

        let server = UdpSocket::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let server_address = server.local_addr().map_err(|error| error.to_string())?;
        let SocketAddr::V4(server_v4) = server_address else {
            return Err("loopback artifact requires IPv4".into());
        };
        let transaction = amazing_webrtc::TransactionId(*b"ferric-stun!");
        let ice_key = b"ferric-loopback-password";
        let client = UdpSocket::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let SocketAddr::V4(client_address) =
            client.local_addr().map_err(|error| error.to_string())?
        else {
            return Err("loopback artifact requires IPv4".into());
        };
        let local_candidate = amazing_webrtc::IceCandidate {
            foundation: "local-host".into(),
            component: 1,
            priority: 100,
            address: client_address,
        };
        let remote_candidate = amazing_webrtc::IceCandidate {
            foundation: "remote-host".into(),
            component: 1,
            priority: 80,
            address: server_v4,
        };
        let mut checklist =
            amazing_webrtc::IceChecklist::new(&[local_candidate], &[remote_candidate], true);
        let check_index = checklist
            .next_check()
            .ok_or_else(|| "ICE checklist had no candidate pair".to_string())?;
        let request = amazing_webrtc::encode_ice_binding_request(
            transaction,
            "remote:local",
            100,
            0x1020_3040_5060_7080,
            true,
            ice_key,
        )
        .map_err(|error| error.to_string())?;
        client
            .send_to(&request, server_address)
            .map_err(|error| error.to_string())?;
        let mut server_buffer = [0u8; 1500];
        let (length, peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        let request = amazing_webrtc::parse_stun(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        if !amazing_webrtc::verify_message_integrity(&server_buffer[..length], ice_key)
            .map_err(|error| error.to_string())?
            || !amazing_webrtc::verify_fingerprint(&server_buffer[..length])
                .map_err(|error| error.to_string())?
        {
            return Err("ICE request authentication failed".into());
        }
        if request.class != amazing_webrtc::StunClass::Request
            || request.transaction_id != transaction
        {
            return Err("unexpected STUN Binding request".into());
        }
        if request.username.as_deref() != Some("remote:local")
            || request.priority != Some(100)
            || request.ice_controlling != Some(0x1020_3040_5060_7080)
            || request.ice_controlled.is_some()
            || !request.use_candidate
        {
            return Err("ICE request attributes failed semantic validation".into());
        }
        let SocketAddr::V4(peer_v4) = peer else {
            return Err("loopback artifact requires IPv4".into());
        };
        let response = amazing_webrtc::encode_ice_binding_success(transaction, peer_v4, ice_key)
            .map_err(|error| error.to_string())?;
        server
            .send_to(&response, peer)
            .map_err(|error| error.to_string())?;

        let mut client_buffer = [0u8; 1500];
        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("STUN response came from the wrong endpoint".into());
        }
        let response = amazing_webrtc::parse_stun(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if !amazing_webrtc::verify_message_integrity(&client_buffer[..length], ice_key)
            .map_err(|error| error.to_string())?
            || !amazing_webrtc::verify_fingerprint(&client_buffer[..length])
                .map_err(|error| error.to_string())?
        {
            return Err("ICE response authentication failed".into());
        }
        if response.transaction_id != transaction
            || response.xor_mapped_address != Some(client_address)
        {
            return Err("STUN mapped address or transaction mismatch".into());
        }
        checklist
            .mark_succeeded(check_index, request.use_candidate)
            .map_err(|error| error.to_string())?;
        if checklist.state() != amazing_webrtc::ChecklistState::Completed {
            return Err("ICE checklist did not complete after nomination".into());
        }

        let outbound = amazing_webrtc::RtpPacket {
            marker: true,
            payload_type: 111,
            sequence_number: 7,
            timestamp: 960,
            ssrc: 0xf3_22_10_01,
            payload: b"ferric-loopback".to_vec(),
        };
        let wire = amazing_webrtc::encode_rtp(&outbound).map_err(|error| error.to_string())?;
        client
            .send_to(&wire, server_address)
            .map_err(|error| error.to_string())?;
        let (length, rtp_peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        if rtp_peer != peer {
            return Err("RTP arrived from a different endpoint".into());
        }
        let received = amazing_webrtc::parse_rtp(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        server
            .send_to(&server_buffer[..length], peer)
            .map_err(|error| error.to_string())?;

        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("RTP echo came from the wrong endpoint".into());
        }
        let echoed = amazing_webrtc::parse_rtp(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if echoed != outbound {
            return Err("RTP echo changed packet contents".into());
        }
        if received != outbound {
            return Err("loopback server decoded different RTP contents".into());
        }

        let sender_report = amazing_webrtc::RtcpSenderReport {
            ssrc: outbound.ssrc,
            ntp_timestamp: 0xe123_4567_89ab_cdef,
            rtp_timestamp: outbound.timestamp,
            packet_count: 1,
            octet_count: outbound.payload.len() as u32,
        };
        let rtcp = amazing_webrtc::encode_rtcp_sender_report(sender_report);
        client
            .send_to(&rtcp, server_address)
            .map_err(|error| error.to_string())?;
        let (length, rtcp_peer) = server
            .recv_from(&mut server_buffer)
            .map_err(|error| error.to_string())?;
        if rtcp_peer != peer {
            return Err("RTCP arrived from a different endpoint".into());
        }
        let server_report = amazing_webrtc::parse_rtcp_sender_report(&server_buffer[..length])
            .map_err(|error| error.to_string())?;
        server
            .send_to(&server_buffer[..length], peer)
            .map_err(|error| error.to_string())?;
        let (length, source) = client
            .recv_from(&mut client_buffer)
            .map_err(|error| error.to_string())?;
        if source != server_address {
            return Err("RTCP echo came from the wrong endpoint".into());
        }
        let echoed_report = amazing_webrtc::parse_rtcp_sender_report(&client_buffer[..length])
            .map_err(|error| error.to_string())?;
        if server_report != sender_report || echoed_report != sender_report {
            return Err("RTCP Sender Report changed during loopback".into());
        }
        Ok((client_address, echoed, checklist.state(), echoed_report))
    }

    match run() {
        Ok((mapped, packet, ice_state, report)) => {
            println!(
                "STUN mapped {mapped}; ICE state={ice_state:?} integrity=ok; RTP seq={} timestamp={} payload={} bytes; RTCP packets={} octets={}; loopback ok",
                packet.sequence_number,
                packet.timestamp,
                packet.payload.len(),
                report.packet_count,
                report.octet_count
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing webrtc-loopback: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.4 artifact: require a real Direct3D 11 hardware device.
fn cmd_gpu_info() -> ExitCode {
    match gpu::probe() {
        Ok(info) => {
            println!("{}, feature level {}", info.backend, info.feature_level);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing gpu-info: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.5 artifact: upload and present a BGRA surface through DXGI.
fn cmd_gpu_present_smoke() -> ExitCode {
    match gpu::presentation_smoke() {
        Ok(info) => {
            println!(
                "{}, presented {}x{} BGRA",
                info.backend, info.width, info.height
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("amazing gpu-present-smoke: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 9.8 artifact: parse, canonicalize, and reparse a bounded SDP offer.
fn cmd_sdp_roundtrip(args: &[String]) -> ExitCode {
    let path = match args.first() {
        Some(path) => path,
        None => {
            eprintln!("usage: amazing sdp-roundtrip <offer.sdp>");
            return ExitCode::FAILURE;
        }
    };
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let parsed = match amazing_webrtc::parse_sdp(&source) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: {error}");
            return ExitCode::FAILURE;
        }
    };
    let canonical = match amazing_webrtc::encode_sdp(&parsed) {
        Ok(canonical) => canonical,
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: {error}");
            return ExitCode::FAILURE;
        }
    };
    match amazing_webrtc::parse_sdp(&canonical) {
        Ok(reparsed) if reparsed == parsed => {
            let media = parsed
                .media_sections
                .iter()
                .map(|section| section.media.as_str())
                .collect::<Vec<_>>()
                .join("+");
            let payloads = parsed
                .media_sections
                .iter()
                .map(|section| section.payload_types.len())
                .sum::<usize>();
            let candidates = parsed
                .media_sections
                .iter()
                .map(|section| section.candidates.len())
                .sum::<usize>();
            let rtp_maps = parsed
                .media_sections
                .iter()
                .map(|section| section.rtp_maps.len())
                .sum::<usize>();
            println!(
                "SDP {} bundle={} payloads={} candidates={} rtpmaps={} ICE={} setup={}; roundtrip ok",
                media,
                parsed.bundle.len(),
                payloads,
                candidates,
                rtp_maps,
                parsed.ice_ufrag,
                parsed.setup
            );
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("amazing sdp-roundtrip: canonical SDP changed semantics");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("amazing sdp-roundtrip: canonical SDP failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Phase 6 demo: execute one classic JavaScript script and print captured
/// `console.log` lines followed by the script completion value.
fn cmd_run(path: Option<&str>) -> ExitCode {
    let source = match read_source(path, "run") {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut js = amazing_js::Interpreter::new();
    match js.run_script(&source) {
        Ok(value) => {
            for line in js.output.borrow().iter() {
                println!("{line}");
            }
            println!("=> {:?}", value);
            ExitCode::SUCCESS
        }
        Err(thrown) => {
            eprintln!("amazing run: {}", format_js_error(&thrown.0));
            ExitCode::FAILURE
        }
    }
}

/// Phase 7 demo: execute a script against a live arena DOM and print the
/// resulting tree. The bridge intentionally starts with the high-value DOM
/// surface: querySelector, textContent, attributes, createElement, append.
fn cmd_domrun(args: &[String]) -> ExitCode {
    let (html_path, script_path) = match (args.first(), args.get(1)) {
        (Some(html), Some(script)) => (html, script),
        _ => {
            eprintln!("usage: amazing domrun <page.html> <script.js>");
            return ExitCode::FAILURE;
        }
    };
    let html = match read_source(Some(html_path), "domrun html") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let script = match read_source(Some(script_path), "domrun script") {
        Ok(source) => source,
        Err(code) => return code,
    };
    let host = Rc::new(RefCell::new(BrowserDomHost {
        dom: amazing_html::parse(&html),
    }));
    let mut js = amazing_js::Interpreter::new();
    js.set_dom_host(host.clone());
    match js.run_script(&script) {
        Ok(value) => {
            for line in js.output.borrow().iter() {
                println!("{line}");
            }
            println!("=> {:?}", value);
            print!("{}", host.borrow().dom.dump(amazing_dom::NodeIdx(0)));
            ExitCode::SUCCESS
        }
        Err(thrown) => {
            eprintln!("amazing domrun: {}", format_js_error(&thrown.0));
            ExitCode::FAILURE
        }
    }
}

fn format_js_error(value: &amazing_js::value::JsValue) -> String {
    match value {
        amazing_js::value::JsValue::Object(object) => {
            let name = object
                .borrow()
                .get_own("name")
                .and_then(|v| match v {
                    amazing_js::value::JsValue::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| "Error".to_string());
            let message = object
                .borrow()
                .get_own("message")
                .and_then(|v| match v {
                    amazing_js::value::JsValue::Str(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            if message.is_empty() {
                name
            } else {
                format!("{name}: {message}")
            }
        }
        amazing_js::value::JsValue::Str(s) => s.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_title_ignores_svg_title_elements() {
        let mut host = BrowserDomHost {
            dom: amazing_html::parse("<svg><title>foreign</title></svg>"),
        };
        assert_eq!(
            <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
            ""
        );
        <BrowserDomHost as amazing_js::DomHost>::set_document_title(&mut host, "html title")
            .expect("set title");
        assert_eq!(
            <BrowserDomHost as amazing_js::DomHost>::get_document_title(&host),
            "html title"
        );
    }
}
