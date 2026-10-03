//! Background URL resources retain the actual winning stylesheet identity.
use super::*;
pub(super) fn collect(node:amazing_dom::NodeIdx,page:&str,styled:&amazing_css::StyledDom,
    media:&mut PageMedia,attempted:&mut HashSet<String>) {
    for style in styled.get(node).into_iter().chain([
        amazing_css::selectors::PseudoElement::Before,amazing_css::selectors::PseudoElement::After,
        amazing_css::selectors::PseudoElement::Marker,
    ].into_iter().filter_map(|pseudo|styled.get_pseudo(node,pseudo))) {
        for layer in amazing_paint::background::BackgroundPlan::from_style(style).layers {
            if let Some(url)=layer.url {
                let resolved=resolve_script_url(layer.source_base.as_deref().unwrap_or(page),&url);
                load_image(media,&resolved,attempted);
                media.background_sources.insert((node,url,layer.source_base),resolved);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn principal_and_pseudo_same_relative_url_use_real_bases_and_paint_actual_pixels() {
        let root=std::env::temp_dir().join(format!("amazing-background-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for (folder,pixel) in [("red",[255,0,0]),("blue",[0,0,255])] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
            let mut bytes=b"P6\n1 1\n255\n".to_vec();bytes.extend(pixel);
            std::fs::write(root.join(folder).join("pixel.ppm"),bytes).unwrap();
        }
        let dom=amazing_html::parse("<body><div></div>");
        let sheets=[("red","body{margin:0}div{display:block;width:2px;height:1px;background-image:url(pixel.ppm)}"),
            ("blue","div::before{content:'';display:block;width:1px;height:1px;background-image:url(pixel.ppm)}")]
            .map(|(folder,source)| {let mut sheet=amazing_css::parse_stylesheet(source);
                sheet.base_url=Some(root.join(folder).join("sheet.css").to_string_lossy().into_owned());sheet});
        let styles=amazing_css::compute(&dom,&sheets);
        let media=load_page_media(&dom,&root.join("page.html").to_string_lossy(),&styles);
        assert!(media.warnings.is_empty(),"{:?}",media.warnings);assert_eq!(media.decoded.len(),2);
        let text=amazing_text::FallbackTextSystem::new();
        let layout=amazing_layout::layout_styled_document(&dom,&styles,&text,&media,2.0,Some(1.0));
        let paint=PagePaint {media:&media,dom:&dom,styles:&styles};
        let surface=amazing_paint::paint_document_with_text_resources(&layout,2,1,&text,&paint);
        assert_eq!(surface.pixels,[0,0,255,255,255,0,0,255]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
