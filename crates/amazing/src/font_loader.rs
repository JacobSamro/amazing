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
        // Distinct inline sheets share a document URL. Preserve every root
        // source; URL deduplication below applies only to recursive imports.
        seen.insert(url.to_owned());
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
        let lower = name.trim().to_ascii_lowercase();
        if lower == "csshworientationtest" {
            let file = std::env::var_os("AMAZING_WPT_ROOT").map(|root| {
                std::path::Path::new(&root)
                    .join("fonts/adobe-fonts/CSSHWOrientationTest.otf")
                    .to_string_lossy()
                    .into_owned()
            });
            return Ok(file.and_then(|path| super::load_resource(&path, max_bytes).ok()));
        }
        let file = local_font_path(&lower);
        Ok(file.and_then(|path| super::load_resource(&path, max_bytes).ok()))
    }

    fn load_url(&mut self, request: FontFetchRequest<'_>) -> Result<Option<Vec<u8>>, TextError> {
        Ok(self.read_url(request.url, request.max_bytes))
    }
}

/// Resolves `src: local('<name>')` for the handful of well-known family
/// names a page is likely to request directly. Windows has fixed font file
/// paths; on Linux (no fontconfig binding in this std-only engine) the same
/// generic-family scan used for the UA default faces stands in.
#[cfg(target_os = "windows")]
fn local_font_path(lower_name: &str) -> Option<String> {
    match lower_name {
        "arial" => Some(r"C:\Windows\Fonts\arial.ttf".to_owned()),
        "times new roman" | "times" => Some(r"C:\Windows\Fonts\times.ttf".to_owned()),
        "segoe ui" => Some(r"C:\Windows\Fonts\segoeui.ttf".to_owned()),
        "consolas" => Some(r"C:\Windows\Fonts\consola.ttf".to_owned()),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn local_font_path(lower_name: &str) -> Option<String> {
    let faces = installed_linux_faces();
    // local() names identify actual full/PostScript face names before the
    // embedding's pre-existing named-family substitutions.
    if let Some(face) = local_installed_face(&faces, lower_name) {
        return face.path.to_str().map(str::to_owned);
    }
    let bucket = match lower_name {
        "arial" | "helvetica" | "sans-serif" | "system-ui" | "segoe ui" | "ui-sans-serif" | "ui-rounded" => FontBucket::Sans,
        "times new roman" | "times" | "georgia" | "serif" | "ui-serif" => FontBucket::Serif,
        "consolas" | "courier" | "courier new" | "monospace" | "ui-monospace" => FontBucket::Mono,
        "emoji" => FontBucket::Emoji,
        _ => return None,
    };
    best_installed_face(&faces, bucket).and_then(|face| face.path.to_str().map(str::to_owned))
}

/// Which generic CSS family a scanned font file stands in for.
#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FontBucket {
    Sans,
    Serif,
    Mono,
    /// A colour-capable emoji face, registered under the CSS "emoji" generic
    /// family (font-variant-emoji, UTR #51's forced emoji presentation).
    Emoji,
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)] // Historical filename-classifier controls remain unchanged.
fn classify_font_file(file_name: &str) -> FontBucket {
    let lower = file_name.to_ascii_lowercase();
    if lower.contains("emoji") {
        FontBucket::Emoji
    } else if lower.contains("mono") {
        FontBucket::Mono
    } else if lower.contains("serif") && !lower.contains("sans") {
        FontBucket::Serif
    } else {
        FontBucket::Sans
    }
}

/// Collects font files under `dir`, bounded in both depth and count so a
/// sandbox with an unexpectedly large or deeply nested font tree cannot make
/// page load hang or exhaust memory.
#[cfg(not(target_os = "windows"))]
fn collect_font_files(dir: &std::path::Path, depth: u32, out: &mut Vec<std::path::PathBuf>) {
    const MAX_DEPTH: u32 = 8;
    const MAX_FILES: usize = 4096;
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            collect_font_files(&path, depth + 1, out);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("ttf")
                        || ext.eq_ignore_ascii_case("otf")
                        || ext.eq_ignore_ascii_case("ttc")
                })
        {
            out.push(path);
        }
        if out.len() >= MAX_FILES {
            return;
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn linux_font_roots() -> Vec<std::path::PathBuf> {
    // fontconfig's own configured directories first -- a system's
    // `/etc/fonts/conf.d/*.conf` can point at directories the common
    // defaults below don't cover (custom images, containers, corporate font
    // installs) -- then the well-known defaults as a fallback/supplement for
    // systems with no fontconfig config at all (or one this scan can't read).
    let mut roots = fontconfig_dirs();
    for default in [
        std::path::PathBuf::from("/usr/share/fonts"),
        std::path::PathBuf::from("/usr/local/share/fonts"),
    ] {
        if !roots.contains(&default) {
            roots.push(default);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        for default in [home.join(".fonts"), home.join(".local/share/fonts")] {
            if !roots.contains(&default) {
                roots.push(default);
            }
        }
    }
    roots
}

/// Reads every `<dir>` element out of fontconfig's own config
/// (`/etc/fonts/fonts.conf` plus `/etc/fonts/conf.d/*.conf`, the standard
/// layout on Debian/Ubuntu/Fedora and every Boat sandbox base image seen so
/// far), so the scan follows what fontconfig is actually configured to
/// search rather than only a hardcoded guess. This is a small hand-rolled
/// tag scanner, not a general XML parser -- fontconfig's config is simple,
/// well-formed XML and `<dir>` is the only element this needs -- matching
/// the existing lightweight-scanner style already used for CSS `@import` in
/// `import_urls` above, rather than pulling in an XML crate for one element.
fn fontconfig_dirs() -> Vec<std::path::PathBuf> {
    let mut sources = vec![std::path::PathBuf::from("/etc/fonts/fonts.conf")];
    if let Ok(entries) = std::fs::read_dir("/etc/fonts/conf.d") {
        let mut confs: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("conf"))
            .collect();
        confs.sort();
        sources.extend(confs);
    }
    let mut dirs = Vec::new();
    for source in sources {
        let Ok(xml) = std::fs::read_to_string(&source) else {
            continue;
        };
        dirs.extend(parse_fontconfig_dir_elements(&xml));
    }
    dirs
}

fn parse_fontconfig_dir_elements(xml: &str) -> Vec<std::path::PathBuf> {
    let xml = strip_xml_comments(xml);
    let mut out = Vec::new();
    let mut rest = xml.as_str();
    while let Some(start) = rest.find("<dir") {
        let after_tag_name = &rest[start + 4..];
        // `<dir>` (no attributes) or `<dir prefix="xdg">`; either way the
        // opening tag ends at the next `>`, and a self-closing `<dir/>`
        // (attrs ending in `/`) has no body to scan for.
        let Some(open_end) = after_tag_name.find('>') else {
            break;
        };
        let attrs = &after_tag_name[..open_end];
        if attrs.trim_end().ends_with('/') {
            rest = &after_tag_name[open_end + 1..];
            continue;
        }
        let body = &after_tag_name[open_end + 1..];
        let Some(close) = body.find("</dir>") else {
            break;
        };
        let content = body[..close].trim();
        if !content.is_empty()
            && let Some(path) = resolve_fontconfig_dir(attrs, content)
        {
            out.push(path);
        }
        rest = &body[close + "</dir>".len()..];
    }
    out
}

/// Removes `<!-- ... -->` comments before the `<dir>` scan runs, so a
/// commented-out directory (fontconfig's own shipped configs comment out
/// deprecated entries, e.g. the `~/.fonts` note) is never picked up. An
/// unterminated comment drops everything after it rather than looping.
fn strip_xml_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 4..];
        match after.find("-->") {
            Some(end) => rest = &after[end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Resolves one `<dir>` element's content the way fontconfig does: a
/// `prefix="xdg"` directory is relative to `$XDG_DATA_HOME` (or
/// `~/.local/share` when that is unset, XDG's own default), a leading `~`
/// expands to `$HOME`, and anything else is used as-is (fontconfig's shipped
/// configs only ever use plain absolute paths beyond those two forms).
fn resolve_fontconfig_dir(attrs: &str, content: &str) -> Option<std::path::PathBuf> {
    let xdg = attrs.contains("prefix=\"xdg\"") || attrs.contains("prefix='xdg'");
    if xdg {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local/share"))
            })?;
        return Some(base.join(content));
    }
    if let Some(rest) = content.strip_prefix('~') {
        let home = std::env::var_os("HOME")?;
        return Some(std::path::PathBuf::from(home).join(rest.trim_start_matches('/')));
    }
    Some(std::path::PathBuf::from(content))
}

/// Picks the best installed file for `bucket` by filename heuristics: known,
/// Actual installed family/style metadata controls selection, with bounded
/// discovery and stable path order.
#[cfg(not(target_os = "windows"))]
#[derive(Clone, Debug)]
struct InstalledLinuxFace {
    path: std::path::PathBuf,
    profile: amazing_text_provider::InstalledFontProfile,
}

#[cfg(not(target_os = "windows"))]
fn read_installed_font(path: &std::path::Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let size = usize::try_from(file.metadata().ok()?.len()).ok()?;
    if size == 0 || size > super::MAX_PAGE_FONT_BYTES { return None; }
    let mut bytes=Vec::new();bytes.try_reserve_exact(size).ok()?;
    file.take(size as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len()==size).then_some(bytes)
}

#[cfg(not(target_os = "windows"))]
fn installed_linux_faces() -> Vec<InstalledLinuxFace> {
    let mut paths = Vec::new();
    for root in linux_font_roots() { collect_font_files(&root, 0, &mut paths); }
    paths.sort(); paths.dedup();
    let mut faces = Vec::new();
    let mut inspected = 0usize;
    // The existing per-file decoder bounds remain authoritative. This adds
    // a whole-discovery budget rather than decoding thousands of large files.
    const MAX_DISCOVERY_BYTES: usize = 128 * 1024 * 1024;
    for path in paths {
        let Ok(size) = std::fs::metadata(&path).map(|m| m.len()) else { continue; };
        let Ok(size) = usize::try_from(size) else { continue; };
        if size == 0 || size > super::MAX_PAGE_FONT_BYTES || size > MAX_DISCOVERY_BYTES - inspected { continue; }
        inspected += size;
        let Some(bytes) = read_installed_font(&path) else { continue; };
        if bytes.len() != size { continue; }
        let format = if bytes.starts_with(b"ttcf") { amazing_text_provider::FontFormat::Collection }
            else { amazing_text_provider::FontFormat::OpenType };
        let Ok(profile) = amazing_text_provider::inspect_installed_font(format, &bytes) else { continue; };
        if profile.families.is_empty() { continue; }
        faces.push(InstalledLinuxFace { path, profile });
    }
    faces
}

#[cfg(not(target_os = "windows"))]
fn installed_bucket(profile: &amazing_text_provider::InstalledFontProfile) -> FontBucket {
    // Classify authentic face metadata, never a filename's suggested style.
    if profile.families.iter().any(|family| family.to_ascii_lowercase().contains("emoji")) { FontBucket::Emoji }
    else if profile.fixed_pitch { FontBucket::Mono }
    else if profile.serif == Some(true) { FontBucket::Serif }
    else if profile.serif == Some(false) { FontBucket::Sans }
    else if profile.families.iter().any(|family| { let f = family.to_ascii_lowercase(); f.contains("serif") && !f.contains("sans") }) { FontBucket::Serif }
    else { FontBucket::Sans }
}

#[cfg(not(target_os = "windows"))]
fn local_installed_face<'a>(faces: &'a [InstalledLinuxFace], name: &str) -> Option<&'a InstalledLinuxFace> {
    faces.iter().find(|face| face.profile.full_names.iter().chain(&face.profile.postscript_names)
        .any(|actual| actual.eq_ignore_ascii_case(name)))
}

#[cfg(not(target_os = "windows"))]
fn family_installed_faces<'a>(faces: &'a [InstalledLinuxFace], primary: &InstalledLinuxFace) -> Vec<&'a InstalledLinuxFace> {
    let Some(family) = primary.profile.families.first() else { return Vec::new(); };
    faces.iter().filter(|face| face.profile.families.iter().any(|name| name.eq_ignore_ascii_case(family))).collect()
}

#[cfg(not(target_os = "windows"))]
fn best_installed_face(faces: &[InstalledLinuxFace], bucket: FontBucket) -> Option<&InstalledLinuxFace> {
    const PREFERRED: [&str; 5] = ["dejavu", "liberation", "noto", "freefont", "free"];
    faces.iter().filter(|face| installed_bucket(&face.profile) == bucket).min_by_key(|face| {
        let preferred = face.profile.families.iter().filter_map(|family| {
            let family = family.to_ascii_lowercase();
            PREFERRED.iter().position(|preferred| family.contains(preferred))
        }).min().unwrap_or(PREFERRED.len());
        (preferred, u8::from(face.profile.slant != amazing_text::FontSlant::Normal),
            face.profile.weight.abs_diff(400), (face.profile.stretch - 100.0).abs() as u32)
    })
}


pub(super) fn register_system_face(
    provider: &mut amazing_text_provider::TextProvider,
    path: &str,
    family: &str,
    weight: u16,
    slant: amazing_text::FontSlant,
) {
    use amazing_text_provider::{FontFormat, WebFont};

    let options = amazing_net::FetchOptions {
        max_response_bytes: super::MAX_PAGE_FONT_BYTES,
        ..amazing_net::FetchOptions::default()
    };
    let Ok(response) = amazing_net::fetch(path, &options) else {
        return;
    };
    let _ = provider.register_web_font(WebFont {
        family: family.into(),
        weight,
        stretch: 100,
        slant,
        format: FontFormat::OpenType,
        bytes: response.body,
    });
}

/// Registers a default face for each generic CSS family, plus a handful of
/// common named families (Arial, Times, Courier, ...) that pages request
/// directly, by scanning the standard Linux font directories. This engine
/// has no fontconfig binding, so without this every generic and named family
/// resolves to zero faces on Linux and every glyph falls back to a
/// full-coverage box (TEXT-17).
#[cfg(not(target_os = "windows"))]
pub(super) fn register_linux_system_faces(provider: &mut amazing_text_provider::TextProvider) {
    let faces = installed_linux_faces();
    // Retain existing embedding aliases; style siblings come from actual
    // WWS/typographic family records, never synthetic renamed fonts.
    for (bucket, aliases) in [
        (FontBucket::Sans, &["sans-serif", "system-ui", "ui-sans-serif", "ui-rounded", "Arial", "Helvetica"][..]),
        (FontBucket::Serif, &["serif", "ui-serif", "Times", "Times New Roman", "Georgia"][..]),
        (FontBucket::Mono, &["monospace", "ui-monospace", "Courier", "Courier New", "Consolas"][..]),
        (FontBucket::Emoji, &["emoji"][..]),
    ] {
        let Some(primary) = best_installed_face(&faces, bucket) else { continue; };
        for face in family_installed_faces(&faces, primary) {
            let Some(bytes) = read_installed_font(&face.path) else { continue; };
            let format = if bytes.starts_with(b"ttcf") { amazing_text_provider::FontFormat::Collection }
                else { amazing_text_provider::FontFormat::OpenType };
            // Revalidate the native profile from the bytes actually registered.
            // A changed system file must not inherit stale discovered names.
            if amazing_text_provider::inspect_installed_font(format, &bytes).ok().as_ref() != Some(&face.profile) { continue; }
            // One native face serves all aliases and genuine family names;
            // style siblings no longer duplicate font bytes per alias.
            if let Err(amazing_text::TextError::ResourceLimit) = provider.register_installed_font_aliases(aliases, format, bytes) { return; }
        }
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod installed_face_tests {
    use super::*;
    use amazing_text_provider::{FontDescriptorRange, InstalledFontProfile};
    fn face(path: &str, family: &str, weight: u16, slant: amazing_text::FontSlant, serif: Option<bool>, fixed: bool) -> InstalledLinuxFace {
        InstalledLinuxFace {path:path.into(),profile:InstalledFontProfile {
            families:vec![family.into()],full_names:vec![format!("{family} {weight}")],postscript_names:vec![format!("{family}-{weight}")],
            weight,stretch:100.0,slant,italic_angle:Some(0.0),fixed_pitch:fixed,family_class:None,serif,color:false,
            registered_axes:Vec::new(),weight_range:FontDescriptorRange::single(weight as f32),stretch_range:FontDescriptorRange::single(100.0),italic_range:None,oblique_range:None,
        }}
    }
    #[test]
    fn installed_family_closure_keeps_real_bold_italic_and_width_siblings() {
        let mut condensed=face("/fonts/unrelated-file.otf","DejaVu Serif",400,amazing_text::FontSlant::Normal,Some(true),false);condensed.profile.stretch=75.0;
        let faces=vec![face("/fonts/bold-looking.ttf","DejaVu Serif",400,amazing_text::FontSlant::Normal,Some(true),false),
            face("/fonts/plain.ttf","DejaVu Serif",700,amazing_text::FontSlant::Normal,Some(true),false),
            face("/fonts/regular.ttf","DejaVu Serif",400,amazing_text::FontSlant::Italic,Some(true),false),condensed,
            face("/fonts/DejaVuSerif.ttf","Other Sans",400,amazing_text::FontSlant::Normal,Some(false),false)];
        let primary=best_installed_face(&faces,FontBucket::Serif).unwrap();assert_eq!(primary.path,std::path::Path::new("/fonts/bold-looking.ttf"));
        let family=family_installed_faces(&faces,primary);assert_eq!(family.len(),4);
        assert!(family.iter().any(|f|f.profile.weight==700));assert!(family.iter().any(|f|f.profile.slant==amazing_text::FontSlant::Italic));
        assert!(family.iter().any(|f|f.profile.stretch==75.0));
    }
    #[test]
    fn installed_bucket_uses_actual_fixed_pitch_and_serif_metadata() {
        let mono=face("/fonts/Sans.ttf","Real Mono",400,amazing_text::FontSlant::Normal,Some(false),true);
        let serif=face("/fonts/Mono.ttf","Real Serif",400,amazing_text::FontSlant::Normal,Some(true),false);
        let sans=face("/fonts/Serif.ttf","Real Sans",400,amazing_text::FontSlant::Normal,Some(false),false);
        assert_eq!(installed_bucket(&mono.profile),FontBucket::Mono);assert_eq!(installed_bucket(&serif.profile),FontBucket::Serif);assert_eq!(installed_bucket(&sans.profile),FontBucket::Sans);
        assert_eq!(best_installed_face(&[mono],FontBucket::Emoji).map(|f|&f.path),None);
    }
    #[test]
    fn local_lookup_matches_real_full_or_postscript_names_not_filename() {
        let faces=vec![face("/fonts/pretended-face.ttf","Actual Face",700,amazing_text::FontSlant::Normal,Some(true),false)];
        assert!(local_installed_face(&faces,"pretended-face").is_none());assert!(local_installed_face(&faces,"Actual Face").is_none());
        assert_eq!(local_installed_face(&faces,"actual face 700").unwrap().profile.weight,700);
        assert_eq!(local_installed_face(&faces,"Actual Face-700").unwrap().profile.weight,700);
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

    /// TEXT-19: mirrors the actual `/etc/fonts/fonts.conf` seen on Boat
    /// sandbox base images -- a plain absolute `<dir>`, an `xdg`-prefixed
    /// one, a `~`-relative one, a comment that must be ignored, and a
    /// self-closing element with no body.
    #[test]
    fn fontconfig_dir_elements_cover_plain_xdg_and_tilde_forms() {
        let xml = r#"<?xml version="1.0"?>
<fontconfig>
    <dir>/usr/share/fonts</dir>
    <dir>/usr/local/share/fonts</dir>
    <dir prefix="xdg">fonts</dir>
    <!-- <dir>/should/be/ignored</dir> -->
    <dir>~/.fonts</dir>
    <cachedir prefix="xdg">fontconfig</cachedir>
</fontconfig>
"#;
        let dirs = parse_fontconfig_dir_elements(xml);
        let xdg_fonts = resolve_fontconfig_dir(r#"prefix="xdg""#, "fonts");
        let tilde_fonts = std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".fonts"));
        let mut expected = vec![
            std::path::PathBuf::from("/usr/share/fonts"),
            std::path::PathBuf::from("/usr/local/share/fonts"),
        ];
        expected.extend(xdg_fonts);
        expected.extend(tilde_fonts);
        assert_eq!(dirs, expected);
    }

    #[test]
    fn fontconfig_xdg_dir_prefers_xdg_data_home_over_the_default() {
        assert_eq!(
            resolve_fontconfig_dir(r#"prefix="xdg""#, "fonts"),
            std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .map(|base| base.join("fonts"))
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(|home| std::path::PathBuf::from(home).join(".local/share/fonts"))
                })
        );
    }

    #[test]
    fn classify_font_file_recognizes_emoji_before_the_other_buckets() {
        assert_eq!(classify_font_file("NotoColorEmoji.ttf"), FontBucket::Emoji);
        assert_eq!(classify_font_file("NotoEmoji-Regular.ttf"), FontBucket::Emoji);
        // "emoji" wins even for a filename that would otherwise read as
        // serif/mono, since font-variant-emoji needs a dedicated bucket
        // rather than falling in with an unrelated text face.
        assert_eq!(classify_font_file("SerifEmojiMono.ttf"), FontBucket::Emoji);
        assert_eq!(classify_font_file("DejaVuSans-Bold.ttf"), FontBucket::Sans);
        assert_eq!(classify_font_file("DejaVuSerif.ttf"), FontBucket::Serif);
        assert_eq!(classify_font_file("DejaVuSansMono.ttf"), FontBucket::Mono);
    }

    #[test]
    fn fontconfig_tilde_dir_expands_against_home() {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        assert_eq!(
            resolve_fontconfig_dir("", "~/.fonts"),
            home.map(|home| home.join(".fonts"))
        );
    }

    #[test]
    fn fontconfig_plain_dir_is_used_verbatim() {
        assert_eq!(
            resolve_fontconfig_dir("", "/opt/company-fonts"),
            Some(std::path::PathBuf::from("/opt/company-fonts"))
        );
    }

    /// A hostile or truncated config (unterminated tag, no closing element)
    /// must stop cleanly rather than loop or panic.
    #[test]
    fn fontconfig_comments_are_stripped_including_unterminated() {
        assert_eq!(strip_xml_comments("a<!-- b -->c"), "ac");
        assert_eq!(strip_xml_comments("a<!-- unterminated"), "a");
        assert_eq!(strip_xml_comments("no comments here"), "no comments here");
    }

    #[test]
    fn fontconfig_dir_scan_never_panics_on_truncated_markup() {
        for xml in ["<dir", "<dir>", "<dir>/unterminated", "<dir/>", "<dir ", ""] {
            let result = std::panic::catch_unwind(|| parse_fontconfig_dir_elements(xml));
            assert!(result.is_ok(), "panicked on {xml:?}");
        }
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
