//! Border-image resources use the page's existing bounded decode cache.
use super::*;

pub(super) fn collect(
    node: amazing_dom::NodeIdx,
    page: &str,
    styles: &amazing_css::StyledDom,
    media: &mut PageMedia,
    attempted: &mut HashSet<String>,
) {
    let mut collect_style = |style: &amazing_css::ComputedStyle| {
        let Some(plan) = amazing_paint::border_image::BorderImagePlan::from_style(style) else {
            return;
        };
        let Some(url) = plan.url else {
            return;
        };
        let resolved = resolve_script_url(plan.source_base.as_deref().unwrap_or(page), &url);
        load_image(media, &resolved, attempted);
        media
            .border_sources
            .insert((url, plan.source_base), resolved);
    };
    if let Some(style) = styles.get(node) {
        collect_style(style);
    }
    for pseudo in [
        amazing_css::selectors::PseudoElement::Before,
        amazing_css::selectors::PseudoElement::After,
        amazing_css::selectors::PseudoElement::Marker,
    ] {
        if let Some(style) = styles.get_pseudo(node, pseudo) {
            collect_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoded_border_sources_share_cache_and_keep_pseudo_base() {
        let source = "data:image/x-portable-pixmap,P6%0A1%201%0A255%0A%FF%00%00";
        let dom = amazing_html::parse("<div></div>");
        let mut sheet=amazing_css::parse_stylesheet(&format!("div{{border-image-source:url('{source}')}}div::before{{content:'x';border-image-source:url('{source}')}}"));
        sheet.base_url = Some("https://example.test/css/sheet.css".into());
        let styles = amazing_css::compute(&dom, &[sheet]);
        let media = load_page_media(&dom, "https://example.test/document.html", &styles);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        assert_eq!(media.decoded.len(), 1);
        assert_eq!(media.border_sources.len(), 1);
        let image = amazing_paint::ImageProvider::border_image(
            &media,
            None,
            source,
            Some("https://example.test/css/sheet.css"),
        )
        .unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.pixels, [255, 0, 0, 255]);
    }
}
#[cfg(test)]
mod relative_tests {
    use super::*;
    #[test]
    fn element_and_pseudo_relative_urls_use_distinct_real_stylesheet_bases() {
        let root =
            std::env::temp_dir().join(format!("amazing-border-bases-{}", std::process::id()));
        for (dir, pixel) in [("red", [255, 0, 0]), ("blue", [0, 0, 255])] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            let mut ppm = b"P6\n1 1\n255\n".to_vec();
            ppm.extend(pixel);
            std::fs::write(root.join(dir).join("image.ppm"), ppm).unwrap();
        }
        let dom = amazing_html::parse("<div></div>");
        let sheets = [
            ("red", "div{border-image-source:url(image.ppm)}"),
            (
                "blue",
                "div::before{content:'x';border-image-source:url(image.ppm)}",
            ),
        ]
        .map(|(dir, css)| {
            let mut sheet = amazing_css::parse_stylesheet(css);
            sheet.base_url = Some(
                root.join(dir)
                    .join("sheet.css")
                    .to_string_lossy()
                    .into_owned(),
            );
            sheet
        });
        let styles = amazing_css::compute(&dom, &sheets);
        let media = load_page_media(&dom, &root.join("page.html").to_string_lossy(), &styles);
        assert!(media.warnings.is_empty(), "{:?}", media.warnings);
        assert_eq!(media.decoded.len(), 2);
        for (dir, pixel) in [("red", [255, 0, 0, 255]), ("blue", [0, 0, 255, 255])] {
            let base = root
                .join(dir)
                .join("sheet.css")
                .to_string_lossy()
                .into_owned();
            let image =
                amazing_paint::ImageProvider::border_image(&media, None, "image.ppm", Some(&base))
                    .unwrap();
            assert_eq!(image.pixels, pixel);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
