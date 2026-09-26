//! HTML responsive-image source selection without a DOM or network dependency.
//! A driver supplies preceding `<picture><source>` siblings in tree order and
//! reevaluates selection when viewport, DPR, media, or attributes change.

/// Attributes of one `<source>` preceding the `<img>` in its `<picture>`.
#[derive(Clone, Copy, Debug)]
pub struct PictureSource<'a> {
    pub srcset: &'a str,
    pub sizes: Option<&'a str>,
    pub media: Option<&'a str>,
    pub mime_type: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct ImageSources<'a> {
    pub src: Option<&'a str>,
    pub srcset: Option<&'a str>,
    pub sizes: Option<&'a str>,
    pub picture: &'a [PictureSource<'a>],
}

/// `slot_width` is the rendered CSS width when `sizes="auto"` is permitted.
#[derive(Clone, Copy, Debug)]
pub struct SourceEnvironment {
    pub viewport_width: f32,
    pub device_pixel_ratio: f32,
    pub slot_width: Option<f32>,
    pub allow_auto_sizes: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedSource {
    pub url: String,
    pub density: f32,
    pub from_picture: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Descriptor {
    Density(f32),
    Width(u32),
}

#[derive(Clone, Debug)]
struct Candidate<'a> {
    url: &'a str,
    descriptor: Descriptor,
}

/// Selects one image URL. `media_matches` evaluates a CSS media query, while
/// `supports_type` checks a MIME essence such as `image/avif`. Both are owned
/// by the embedding driver so this crate can remain independent of CSS/codecs.
pub fn select_source(
    sources: ImageSources<'_>,
    environment: SourceEnvironment,
    media_matches: impl Fn(&str) -> bool,
    supports_type: impl Fn(&str) -> bool,
) -> Option<SelectedSource> {
    select_source_with_sizes(sources, environment, media_matches, supports_type,
        |sizes, environment, media_matches| source_size(sizes, environment, media_matches))
}

/// Driver form that delegates `sizes` evaluation to the CSS media and length
/// parser. This keeps CSS grammar in the CSS crate while selection stays here.
pub fn select_source_with_sizes(
    sources: ImageSources<'_>,
    environment: SourceEnvironment,
    media_matches: impl Fn(&str) -> bool,
    supports_type: impl Fn(&str) -> bool,
    source_size: impl Fn(Option<&str>, SourceEnvironment, &dyn Fn(&str) -> bool) -> f32,
) -> Option<SelectedSource> {
    for source in sources.picture {
        if source.srcset.trim().is_empty()
            || source.media.is_some_and(|query| !media_matches(query))
            || source.mime_type.is_some_and(|mime| {
                !supports_type(mime.split(';').next().unwrap_or("").trim())
            })
        {
            continue;
        }
        if let Some(chosen) = choose(
            parse_srcset(source.srcset),
            None,
            source.sizes,
            environment,
            &media_matches,
            &source_size,
        ) {
            return Some(SelectedSource { from_picture: true, ..chosen });
        }
    }
    choose(
        sources.srcset.map(parse_srcset).unwrap_or_default(),
        sources.src,
        sources.sizes,
        environment,
        &media_matches,
        &source_size,
    )
}

fn choose<'a>(
    mut candidates: Vec<Candidate<'a>>,
    fallback: Option<&'a str>,
    sizes: Option<&str>,
    environment: SourceEnvironment,
    media_matches: &impl Fn(&str) -> bool,
    size_for: &impl Fn(Option<&str>, SourceEnvironment, &dyn Fn(&str) -> bool) -> f32,
) -> Option<SelectedSource> {
    let has_width = candidates.iter().any(|c| matches!(c.descriptor, Descriptor::Width(_)));
    if !has_width && !candidates.iter().any(|c| c.descriptor == Descriptor::Density(1.0)) {
        if let Some(url) = fallback.filter(|url| !url.is_empty()) {
            candidates.push(Candidate { url, descriptor: Descriptor::Density(1.0) });
        }
    }
    let slot = size_for(sizes, environment, media_matches);
    let mut ranked = candidates.into_iter().filter_map(|candidate| {
        let density = match candidate.descriptor {
            Descriptor::Density(value) if !has_width => value,
            Descriptor::Width(width) if has_width && slot > 0.0 => width as f32 / slot,
            _ => return None,
        };
        (density.is_finite() && density > 0.0).then_some((candidate.url, density))
    }).collect::<Vec<_>>();
    // The first occurrence wins when normalized densities tie.
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1));
    let target = if environment.device_pixel_ratio.is_finite() && environment.device_pixel_ratio > 0.0 {
        environment.device_pixel_ratio
    } else { 1.0 };
    let (url, density) = ranked.iter().find(|(_, d)| *d >= target)
        .or_else(|| ranked.last()).copied()?;
    Some(SelectedSource { url: url.to_owned(), density, from_picture: false })
}

fn parse_srcset(input: &str) -> Vec<Candidate<'_>> {
    let mut remaining = input;
    let mut result = Vec::new();
    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if remaining.is_empty() { break; }
        let url_end = remaining.find(char::is_whitespace).unwrap_or(remaining.len());
        let raw_url = &remaining[..url_end];
        remaining = &remaining[url_end..];
        let (url, descriptor_text) = if raw_url.ends_with(',') {
            (raw_url.trim_end_matches(','), "")
        } else {
            let end = remaining.find(',').unwrap_or(remaining.len());
            let descriptor = &remaining[..end];
            remaining = &remaining[end.min(remaining.len())..];
            (raw_url, descriptor)
        };
        if url.is_empty() { continue; }
        let tokens = descriptor_text.split_ascii_whitespace().collect::<Vec<_>>();
        let descriptor = match tokens.as_slice() {
            [] => Some(Descriptor::Density(1.0)),
            [one] if one.ends_with('w') => one[..one.len()-1].parse::<u32>().ok()
                .filter(|w| *w > 0).map(Descriptor::Width),
            [one] if one.ends_with('x') => one[..one.len()-1].parse::<f32>().ok()
                .filter(|d| d.is_finite() && *d > 0.0).map(Descriptor::Density),
            _ => None,
        };
        if let Some(descriptor) = descriptor {
            if !result.iter().any(|c: &Candidate<'_>| c.descriptor == descriptor) {
                result.push(Candidate { url, descriptor });
            }
        }
    }
    result
}

fn source_size(
    sizes: Option<&str>,
    env: SourceEnvironment,
    media_matches: &dyn Fn(&str) -> bool,
) -> f32 {
    let viewport = env.viewport_width.max(0.0);
    for entry in sizes.unwrap_or("").split(',') {
        let value = entry.trim();
        if value.eq_ignore_ascii_case("auto") && env.allow_auto_sizes {
            if let Some(width) = env.slot_width.filter(|width| width.is_finite() && *width > 0.0) {
                return width;
            }
        }
        // The source size is the last component. A driver evaluates the
        // preceding media condition using the same engine as CSS @media.
        let Some((length, media)) = value.rsplit_once(char::is_whitespace) else {
            if let Some(width) = parse_length(value, viewport) { return width; }
            continue;
        };
        if media_matches(length.trim()) {
            if let Some(width) = parse_length(media.trim(), viewport) { return width; }
        }
    }
    viewport
}

fn parse_length(value: &str, viewport: f32) -> Option<f32> {
    let (number, factor) = if let Some(n) = value.strip_suffix("vw") {
        (n, viewport / 100.0)
    } else if let Some(n) = value.strip_suffix("px") {
        (n, 1.0)
    } else { return None; };
    number.trim().parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n * factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn env(width: f32, dpr: f32) -> SourceEnvironment {
        SourceEnvironment { viewport_width: width, device_pixel_ratio: dpr, slot_width: None, allow_auto_sizes: false }
    }
    fn img<'a>(src: Option<&'a str>, srcset: Option<&'a str>, sizes: Option<&'a str>) -> ImageSources<'a> {
        ImageSources { src, srcset, sizes, picture: &[] }
    }
    fn select(sources: ImageSources<'_>, env: SourceEnvironment) -> SelectedSource {
        select_source(sources, env, |_| true, |_| true).unwrap()
    }
    #[test]
    fn density_and_fallback_select_at_dpr() {
        let sources = img(Some("fallback.png"), Some("small.png 0.5x, large.png 2x"), None);
        assert_eq!(select(sources, env(800.0, 1.0)).url, "fallback.png");
        assert_eq!(select(sources, env(800.0, 1.5)).url, "large.png");
    }
    #[test]
    fn width_uses_first_matching_size_and_viewport_default() {
        let sources = img(Some("fallback.png"), Some("small.png 400w, large.png 800w"), Some("(max-width: 600px) 400px, 50vw"));
        let selected = select_source(sources, env(1000.0, 1.5), |q| q != "(max-width: 600px)", |_| true).unwrap();
        assert_eq!(selected.url, "large.png");
        assert_eq!(select(img(None, Some("small.png 400w, large.png 800w"), None), env(500.0, 1.0)).url, "large.png");
    }
    #[test]
    fn picture_filters_media_and_type_in_tree_order() {
        let picture = [
            PictureSource { srcset: "a.avif", sizes: None, media: None, mime_type: Some("image/avif") },
            PictureSource { srcset: "b.webp", sizes: None, media: Some("(min-width: 600px)"), mime_type: Some("image/webp") },
        ];
        let sources = ImageSources { src: Some("fallback.png"), srcset: None, sizes: None, picture: &picture };
        let chosen = select_source(sources, env(800.0, 1.0), |_| true, |mime| mime == "image/webp").unwrap();
        assert_eq!(chosen.url, "b.webp");
        assert!(chosen.from_picture);
        let chosen = select_source(sources, env(400.0, 1.0), |_| false, |_| false).unwrap();
        assert_eq!(chosen.url, "fallback.png");
    }
    #[test]
    fn invalid_descriptors_and_duplicate_densities_are_discarded() {
        let chosen = select(img(None, Some("bad.png 1x 2x, first.png 1x, second.png 1x, high.png 2x"), None), env(500.0, 1.0));
        assert_eq!(chosen.url, "first.png");
    }
    #[test]
    fn auto_sizes_requires_permission_and_positive_slot() {
        let sources = img(None, Some("small.png 100w, large.png 1000w"), Some("auto, 100vw"));
        let mut settings = env(1000.0, 1.0);
        settings.allow_auto_sizes = true;
        settings.slot_width = Some(100.0);
        assert_eq!(select(sources, settings).url, "small.png");
    }
}
