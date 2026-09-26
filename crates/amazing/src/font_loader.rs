//! Caller-owned font resource loading. The provider supplies source ordering and
//! format filtering; this adapter supplies stylesheet-relative URLs and bytes.

use amazing_fetch::{
    HttpUrl, Origin,
    service::{ClientContext, FontRequest, NetworkService, ServiceEvent},
};
use amazing_net::dispatch::DispatchLimits;
use amazing_text::{FeatureSetting, TextError};
use amazing_text_provider::{
    FontDescriptorRange, FontDisplay, FontFaceDescriptor, FontFaceLoader, FontFaceSource,
    FontFetchRequest, FontStyleDescriptor, UnicodeRange,
};
use std::time::{Duration, Instant};

pub(super) fn expand_font_sheets(
    sheets: &[(String, String)],
    warnings: &mut Vec<String>,
) -> Vec<(String, String)> {
    fn visit(
        source: &str,
        url: &str,
        depth: usize,
        out: &mut Vec<(String, String)>,
        seen: &mut std::collections::HashSet<String>,
        warnings: &mut Vec<String>,
    ) {
        if depth >= 8 || out.len() >= 64 {
            warnings.push(format!("font stylesheet import limit at {url}"));
            return;
        }
        if !seen.insert(url.to_owned()) {
            return;
        }
        for imported in import_urls(source) {
            let resolved = super::resolve_font_url(url, &imported);
            if seen.contains(&resolved) {
                continue;
            }
            match super::load_resource(&resolved, 16 * 1024 * 1024) {
                Ok(bytes) => visit(
                    &String::from_utf8_lossy(&bytes),
                    &resolved,
                    depth + 1,
                    out,
                    seen,
                    warnings,
                ),
                Err(error) => warnings.push(format!("font stylesheet {resolved}: {error}")),
            }
        }
        out.push((source.to_owned(), url.to_owned()));
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (source, url) in sheets {
        visit(source, url, 0, &mut out, &mut seen, warnings);
    }
    out
}

fn import_urls(source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = source.as_bytes();
    let mut index = 0usize;
    let mut depth = 0usize;
    let mut quote = 0u8;
    while index < bytes.len() {
        let byte = bytes[index];
        if quote != 0 {
            if byte == b'\\' {
                index = (index + 2).min(bytes.len());
                continue;
            }
            if byte == quote {
                quote = 0;
            }
        } else if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        } else if matches!(byte, b'\'' | b'"') {
            quote = byte;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
        } else if depth == 0
            && bytes[index..]
                .get(..7)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"@import"))
        {
            let rest = source[index + 7..].trim_start();
            let rest = rest
                .strip_prefix("url(")
                .or_else(|| rest.strip_prefix("URL("));
            if let Some(rest) = rest {
                let value = rest.trim_start().trim_start_matches(['\'', '"']);
                if let Some(end) = value.find(['\'', '"', ')']) {
                    result.push(value[..end].to_owned());
                }
            } else {
                let rest = source[index + 7..].trim_start();
                if let Some(value) = rest.strip_prefix(['\'', '"'])
                    && let Some(end) = value.find(['\'', '"'])
                {
                    result.push(value[..end].to_owned());
                }
            }
            index += 7;
            continue;
        }
        index += 1;
    }
    result
}

pub(super) fn descriptor_from_css(
    face: amazing_css::FontFaceRule,
) -> Result<FontFaceDescriptor, String> {
    let weight = descriptor_range(&face.weight, 400, false)?;
    let stretch = descriptor_range(&face.stretch, 100, true)?;
    let style = match face.style.trim().to_ascii_lowercase().as_str() {
        "normal" => FontStyleDescriptor::Normal,
        "italic" => FontStyleDescriptor::Italic,
        value if value.starts_with("oblique") => {
            let angles = value[7..]
                .split_whitespace()
                .map(|part| part.strip_suffix("deg").unwrap_or(part).parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "invalid font-style angle")?;
            if angles.len() > 2 {
                return Err("invalid font-style angle range".into());
            }
            let start = *angles.first().unwrap_or(&14.0);
            let end = *angles.get(1).unwrap_or(&start);
            FontStyleDescriptor::Oblique {
                min_degrees: start.min(end),
                max_degrees: start.max(end),
            }
        }
        _ => return Err("invalid font-style".into()),
    };
    let unicode_ranges = face
        .unicode_range
        .as_deref()
        .map(parse_unicode_ranges)
        .transpose()?
        .unwrap_or_default();
    let display = match face
        .display
        .as_deref()
        .unwrap_or("auto")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "auto" => FontDisplay::Auto,
        "block" => FontDisplay::Block,
        "swap" => FontDisplay::Swap,
        "fallback" => FontDisplay::Fallback,
        "optional" => FontDisplay::Optional,
        _ => FontDisplay::Auto,
    };
    let percentage = |value: Option<&str>, default: Option<f32>| -> Result<Option<f32>, String> {
        match value.map(str::trim) {
            None | Some("normal") => Ok(default),
            Some(raw) => raw
                .strip_suffix('%')
                .ok_or("expected percentage".to_owned())?
                .trim()
                .parse::<f32>()
                .map(|n| Some(n / 100.0))
                .map_err(|_| "invalid percentage".to_owned()),
        }
    };
    let size_adjust = percentage(face.size_adjust.as_deref(), Some(1.0))?.unwrap_or(1.0);
    let sources = face
        .sources
        .into_iter()
        .map(|source| match source.local {
            Some(name) => FontFaceSource::Local(name),
            None => FontFaceSource::Url {
                url: source.url,
                formats: source.format.into_iter().collect(),
                technologies: source.technologies,
            },
        })
        .collect();
    Ok(FontFaceDescriptor {
        family: face.family,
        sources,
        unicode_ranges,
        weight,
        stretch,
        style,
        display,
        size_adjust,
        ascent_override: percentage(face.ascent_override.as_deref(), None)?,
        descent_override: percentage(face.descent_override.as_deref(), None)?,
        line_gap_override: percentage(face.line_gap_override.as_deref(), None)?,
        features: face
            .feature_settings
            .as_deref()
            .map(parse_features)
            .transpose()?
            .unwrap_or_default(),
    })
}

fn descriptor_range(
    value: &str,
    default: u16,
    percentage: bool,
) -> Result<FontDescriptorRange, String> {
    let mut parts = value.split_whitespace().map(|part| {
        if part.eq_ignore_ascii_case("normal") {
            return Ok(default as f32);
        }
        if part.eq_ignore_ascii_case("bold") && !percentage {
            return Ok(700.0);
        }
        let part = if percentage {
            part.strip_suffix('%').ok_or("expected percentage")?
        } else {
            part
        };
        part.parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 1.0 && *value <= 1000.0)
            .ok_or("invalid descriptor range")
    });
    let min = parts
        .next()
        .transpose()
        .map_err(str::to_owned)?
        .unwrap_or(default as f32);
    let max = parts
        .next()
        .transpose()
        .map_err(str::to_owned)?
        .unwrap_or(min);
    if parts.next().is_some() || min < 1.0 || max < 1.0 || min > 1000.0 || max > 1000.0 {
        return Err("invalid descriptor range".into());
    }
    Ok(FontDescriptorRange {
        min: min.min(max),
        max: min.max(max),
    })
}

fn parse_unicode_ranges(value: &str) -> Result<Vec<UnicodeRange>, String> {
    value
        .split(',')
        .map(|part| {
            let raw = part
                .trim()
                .strip_prefix("U+")
                .or_else(|| part.trim().strip_prefix("u+"))
                .ok_or("invalid unicode-range")?;
            let (start, end) = if raw.contains('?') {
                let lower = raw.replace('?', "0");
                let upper = raw.replace('?', "F");
                (lower, upper)
            } else if let Some((lower, upper)) = raw.split_once('-') {
                (lower.to_owned(), upper.to_owned())
            } else {
                (raw.to_owned(), raw.to_owned())
            };
            let start = u32::from_str_radix(&start, 16).map_err(|_| "invalid unicode-range")?;
            let end = u32::from_str_radix(&end, 16).map_err(|_| "invalid unicode-range")?;
            if start > end || end > 0x10ffff {
                return Err("invalid unicode-range".into());
            }
            Ok(UnicodeRange { start, end })
        })
        .collect::<Result<_, _>>()
        .map_err(str::to_owned)
}

fn parse_features(value: &str) -> Result<Vec<FeatureSetting>, String> {
    if value.trim().eq_ignore_ascii_case("normal") {
        return Ok(Vec::new());
    }
    value
        .split(',')
        .map(|item| {
            let item = item.trim();
            let tag = item.get(1..5).ok_or("invalid feature tag")?;
            if !matches!(item.as_bytes().first(), Some(b'\'' | b'"'))
                || item.as_bytes().get(5) != item.as_bytes().first()
                || !tag.is_ascii()
            {
                return Err("invalid feature tag");
            }
            let rest = item[6..].trim();
            let value = if rest.is_empty() || rest.eq_ignore_ascii_case("on") {
                1
            } else if rest.eq_ignore_ascii_case("off") {
                0
            } else {
                rest.parse::<u32>().map_err(|_| "invalid feature value")?
            };
            Ok(FeatureSetting {
                tag: tag
                    .as_bytes()
                    .try_into()
                    .map_err(|_| "invalid feature tag")?,
                value,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(str::to_owned)
}

pub(super) struct PageFontLoader<'a> {
    pub stylesheet_url: &'a str,
    pub document_url: &'a str,
    pub warnings: Vec<String>,
    /// Per-page cache of resolved-URL -> fetch result, shared across every
    /// `@font-face` rule on the page. A real page routinely declares several
    /// icon-font families (e.g. "Font Awesome 5/6 Free/Brands" plus a plain
    /// "FontAwesome" alias) whose `src:` lists repeat the exact same URL; each
    /// distinct call used to pay its own network round trip (up to the 20s
    /// `fetch_cors` deadline below) even when an earlier rule already fetched
    /// -- or already failed to fetch -- that same URL. See IA-LIVE-2: this was
    /// most of the 279s render time on docs.rs/serde (12 duplicate fetches of
    /// 4 distinct FontAwesome URLs, each timing out against an unreachable
    /// host, plus 5 unique webfont URLs -- 17 sequential fetch attempts for 9
    /// distinct URLs).
    pub cache: &'a mut std::collections::HashMap<String, Result<Vec<u8>, String>>,
}

pub(super) fn font_source_label(source: &str) -> String {
    if source.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:")) {
        format!("data:… ({} bytes)", source.len())
    } else if source.len() > 160 {
        format!("{}… ({} bytes)", &source[..source.floor_char_boundary(160)], source.len())
    } else {
        source.to_owned()
    }
}

impl PageFontLoader<'_> {
    fn read_url(&mut self, source: &str, max_bytes: usize) -> Option<Vec<u8>> {
        let resolved = super::resolve_font_url(self.stylesheet_url, source);
        let result = if let Some(cached) = self.cache.get(&resolved) {
            cached.clone()
        } else {
            let fetched = if resolved.starts_with("http://") || resolved.starts_with("https://") {
                self.fetch_cors(source, max_bytes)
            } else {
                super::load_resource(&resolved, max_bytes)
            };
            self.cache.insert(resolved.clone(), fetched.clone());
            fetched
        };
        match result {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                self.warnings.push(format!(
                    "font {:?} ({}): {error}",
                    font_source_label(source),
                    font_source_label(&resolved)
                ));
                None
            }
        }
    }

    fn fetch_cors(&self, source: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let stylesheet =
            HttpUrl::parse(self.stylesheet_url).map_err(|e| format!("stylesheet URL: {e:?}"))?;
        let origin = HttpUrl::parse(self.document_url)
            .map(|url| url.origin().clone())
            .unwrap_or_else(|_| Origin::opaque());
        let request = FontRequest::new(source, &stylesheet, origin, max_bytes)
            .map_err(|e| format!("font request: {e:?}"))?;
        let mut service = NetworkService::new(DispatchLimits::default());
        let id = service
            .start_font(request, ClientContext::default())
            .map_err(|e| format!("font start: {e:?}"))?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut network_error = None;
        loop {
            for event in service.step(64, Instant::now()) {
                if let ServiceEvent::Network {
                    id: event_id,
                    event,
                } = &event
                    && *event_id == id
                    && matches!(event, amazing_net::dispatch::NetEvent::Error { .. })
                {
                    network_error = Some(format!("{event:?}"));
                }
                if let ServiceEvent::Finished {
                    id: finished,
                    result,
                } = event
                    && finished == id
                {
                    let mut response =
                        result.map_err(|e| format!("font fetch: {e:?}; {network_error:?}"))?;
                    if !(200..300).contains(&response.status) {
                        return Err(format!("HTTP {}", response.status));
                    }
                    let bytes = response
                        .body
                        .bytes()
                        .map_err(|e| format!("font body: {e:?}"))?;
                    return (bytes.len() <= max_bytes)
                        .then_some(bytes)
                        .ok_or_else(|| "font size limit".to_owned());
                }
            }
            if Instant::now() >= deadline {
                service.abort(id);
                return Err("font fetch timeout".to_owned());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl FontFaceLoader for PageFontLoader<'_> {
    fn load_local(&mut self, name: &str, max_bytes: usize) -> Result<Option<Vec<u8>>, TextError> {
        let file = match name.trim().to_ascii_lowercase().as_str() {
            "arial" => Some(r"C:\Windows\Fonts\arial.ttf".to_owned()),
            "times new roman" | "times" => Some(r"C:\Windows\Fonts\times.ttf".to_owned()),
            "segoe ui" => Some(r"C:\Windows\Fonts\segoeui.ttf".to_owned()),
            "consolas" => Some(r"C:\Windows\Fonts\consola.ttf".to_owned()),
            "csshworientationtest" => std::env::var_os("AMAZING_WPT_ROOT").map(|root| {
                std::path::Path::new(&root)
                    .join("fonts/adobe-fonts/CSSHWOrientationTest.otf")
                    .to_string_lossy()
                    .into_owned()
            }),
            _ => None,
        };
        Ok(file.and_then(|path| super::load_resource(&path, max_bytes).ok()))
    }

    fn load_url(&mut self, request: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> {
        Ok(self.read_url(request.url, request.max_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_ignore_comments_nested_rules_and_quoted_semicolons() {
        let css = "/* @import 'bad.css'; */ @import url('sub/a.css'); @import \"b.css\" screen; p { content: '@import x.css'; }";
        assert_eq!(import_urls(css), ["sub/a.css", "b.css"]);
    }

    #[test]
    fn css_face_descriptors_convert_to_provider_contract() {
        let face = amazing_css::parse_font_faces("@font-face { font-family: Demo; src: local('Absent'), url('../demo.woff2') format('woff2') tech(features-opentype); font-weight: 300 700; font-stretch: 80% 120%; font-style: italic; unicode-range: U+41-5A, U+4??; font-display: fallback; size-adjust: 90%; ascent-override: 80%; descent-override: 20%; line-gap-override: 0%; font-feature-settings: 'liga' 0; }").remove(0);
        let descriptor = descriptor_from_css(face).unwrap();
        assert_eq!(descriptor.sources.len(), 2);
        assert_eq!(
            descriptor.weight,
            FontDescriptorRange {
                min: 300.0,
                max: 700.0
            }
        );
        assert_eq!(
            descriptor.stretch,
            FontDescriptorRange {
                min: 80.0,
                max: 120.0
            }
        );
        assert_eq!(
            descriptor.unicode_ranges,
            [
                UnicodeRange {
                    start: 0x41,
                    end: 0x5a
                },
                UnicodeRange {
                    start: 0x400,
                    end: 0x4ff
                }
            ]
        );
        assert_eq!(descriptor.display, FontDisplay::Fallback);
        assert_eq!(descriptor.size_adjust, 0.9);
        assert_eq!(descriptor.ascent_override, Some(0.8));
        assert_eq!(
            descriptor.features,
            [FeatureSetting {
                tag: *b"liga",
                value: 0
            }]
        );
    }

    #[test]
    fn reversed_face_ranges_are_normalized_before_registration() {
        let face = amazing_css::parse_font_faces("@font-face { font-family: Demo; src: url(a.ttf); font-weight: 900 100; font-stretch: 75% 62.5%; font-style: oblique 90deg -90deg; }").remove(0);
        let descriptor = descriptor_from_css(face).unwrap();
        assert_eq!(
            descriptor.weight,
            FontDescriptorRange {
                min: 100.0,
                max: 900.0
            }
        );
        assert_eq!(
            descriptor.stretch,
            FontDescriptorRange {
                min: 62.5,
                max: 75.0
            }
        );
        assert_eq!(
            descriptor.style,
            FontStyleDescriptor::Oblique {
                min_degrees: -90.0,
                max_degrees: 90.0
            }
        );
    }

    #[test]
    fn http_font_uses_stylesheet_base_and_cors_service() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let mut count = 0;
            while !request[..count]
                .windows(4)
                .any(|bytes| bytes == b"\r\n\r\n")
            {
                let read = socket.read(&mut request[count..]).unwrap();
                assert!(read > 0 && count + read <= request.len());
                count += read;
            }
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /css/font.ttf "));
            socket.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfont").unwrap();
        });
        let base = format!("http://{address}/css/site.css");
        let document = format!("http://{address}/index.html");
        let mut cache = std::collections::HashMap::new();
        let mut loader = PageFontLoader {
            stylesheet_url: &base,
            document_url: &document,
            warnings: Vec::new(),
            cache: &mut cache,
        };
        let bytes = loader
            .load_url(FontFetchRequest {
                url: "font.ttf",
                cors_mode: true,
                max_bytes: 4,
            })
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            bytes.as_deref(),
            Some(b"font".as_slice()),
            "{:?}",
            loader.warnings
        );
    }

    /// IA-LIVE-2: real pages (docs.rs, plenty of icon-font stylesheets)
    /// declare several `@font-face` rules — e.g. "Font Awesome 5 Free",
    /// "Font Awesome 6 Free" and a plain "FontAwesome" alias — whose `src:`
    /// lists repeat the exact same URL. A second `load_url` for a URL
    /// already resolved this page must reuse the cached result instead of
    /// opening another connection; each duplicate used to pay its own up-to-
    /// 20s `fetch_cors` deadline.
    #[test]
    fn http_font_source_is_only_fetched_once_per_page_even_when_reused_by_multiple_font_face_rules() {
        use std::io::{Read, Write};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let server_hits = hits.clone();
        let server = std::thread::spawn(move || {
            // A correctly deduped loader connects at most once; give any
            // second connection a short window to show up before we stop
            // listening and let the test assert on the count.
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        server_hits.fetch_add(1, Ordering::SeqCst);
                        let mut request = [0u8; 4096];
                        let mut count = 0;
                        while !request[..count].windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            match socket.read(&mut request[count..]) {
                                Ok(read) if read > 0 => count += read,
                                _ => break,
                            }
                        }
                        let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 4\r\nConnection: close\r\n\r\nfont");
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
        });
        let base = format!("http://{address}/css/site.css");
        let document = format!("http://{address}/index.html");
        let mut cache = std::collections::HashMap::new();
        let mut loader = PageFontLoader {
            stylesheet_url: &base,
            document_url: &document,
            warnings: Vec::new(),
            cache: &mut cache,
        };
        let request = || FontFetchRequest { url: "font.ttf", cors_mode: true, max_bytes: 4 };
        let first = loader.load_url(request()).unwrap();
        let second = loader.load_url(request()).unwrap();
        server.join().unwrap();
        assert_eq!(first.as_deref(), Some(b"font".as_slice()), "{:?}", loader.warnings);
        assert_eq!(second, first, "second call should return the cached bytes");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "second load_url for the same URL must not open a new connection"
        );
    }
}
