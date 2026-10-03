//! Anonymous box used-unit context retains the originating document metrics.
use super::*;
#[test]
fn anonymous_context_preserves_local_properties_and_actual_used_units() {
    for (root_font, outer_zoom, inner_zoom) in [(24.0,2.0,1.5),(19.5,1.25,0.8),(16.0,0.5,2.0)] {
        let root = resolve_with_root_and_attrs(None,
            [("font-size".into(),format!("{root_font}px")),("zoom".into(),outer_zoom.to_string())].into(),root_font,None);
        let parent = resolve_with_root_and_attrs(Some(&root),
            [("zoom".into(),inner_zoom.to_string()),("color".into(),"green".into())].into(),root_font,None);
        let mut anonymous = ComputedStyle::default();
        anonymous.set("font-size","12px");
        anonymous.set("color","red");
        anonymous.set_url_base("background-image",Some("https://example.test/sheet.css"));
        let properties = anonymous.props.clone();
        let typed = anonymous.typed.clone();
        let urls = anonymous.url_bases.clone();
        anonymous.inherit_used_context_from(&parent);
        assert_eq!(anonymous.props,properties);
        assert_eq!(anonymous.typed,typed);
        assert_eq!(anonymous.url_bases,urls);
        assert_eq!(anonymous.effective_zoom(),outer_zoom*inner_zoom);
        assert_eq!(anonymous.root_font_size_px(),root_font);
        anonymous.set("padding-left","2rem");
        assert_eq!(anonymous.get("padding-left"),Some("2rem"));
        assert_eq!(anonymous.typed("padding-left"),Some(&ComputedValue::LengthPercentage(values::LengthPercentage::Px(f64::from(root_font)*2.0))));
        assert_eq!(anonymous.used_length_px("padding-left"),Some(f64::from(root_font)*2.0*outer_zoom*inner_zoom));
        assert_eq!(anonymous.get("zoom"),properties.get("zoom").map(String::as_str));
    }
}
