//! Severity of an observed computed-style change. Keep every registered longhand
//! in this explicit table; the coverage test rejects unclassified additions.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum StyleChangeKind {
    #[default]
    None,
    CompositorOnly,
    PaintOnly,
    Layout,
    Rebuild,
}

pub fn classify_style_property(name: &str) -> Option<StyleChangeKind> {
    use StyleChangeKind::*;
    Some(match name {
        // CompositorOnly
        "opacity" |
        "transform" | "transform-origin" | "perspective-origin" | "transform-box" |
        "filter" |
        "backdrop-filter" |
        "mix-blend-mode" |
        "isolation" |
        "clip-path" |
        "mask-image" |
        "mask-mode" |
        "mask-repeat" |
        "mask-position" |
        "mask-clip" |
        "mask-origin" |
        "mask-size" |
        "mask-composite" |
        "mask-type" => CompositorOnly,
        // PaintOnly
        "background-attachment" |
        "background-clip" |
        "background-color" |
        "border-image" | "border-image-source" | "border-image-slice" | "border-image-width" | "border-image-outset" | "border-image-repeat" |
        "background-image" |
        "background-origin" |
        "background-position" |
        "background-position-x" |
        "background-position-y" |
        "background-repeat" |
        "background-size" |
        "border-bottom-color" |
        "border-bottom-left-radius" |
        "border-bottom-right-radius" |
        "border-left-color" |
        "border-right-color" |
        "border-top-color" |
        "border-top-left-radius" |
        "border-top-right-radius" |
        "box-shadow" |
        "clip-rule" |
        "color" |
        "color-scheme" |
        "cursor" |
        "fill" |
        "fill-opacity" |
        "fill-rule" |
        "font-palette" |
        "image-orientation" | "image-rendering" |
        "mask" |
        "object-fit" |
        "object-position" |
        // RC-32: unlike border, outline never affects box geometry at all
        // (drawn outside the border box, offset by outline-offset, with
        // no effect on any ancestor/sibling layout) -- every outline
        // longhand is PaintOnly, including outline-width/-offset.
        "outline-color" |
        "outline-style" |
        "outline-width" |
        "outline-offset" |
        "stroke" |
        "stroke-dasharray" |
        "stroke-dashoffset" |
        "stroke-linecap" |
        "stroke-linejoin" |
        "stroke-miterlimit" |
        "stroke-opacity" |
        "stroke-width" |
        "text-decoration" |
        "vector-effect" |
        "visibility" => PaintOnly,
        // Layout
        "align-content" |
        "align-items" |
        "align-self" |
        "aspect-ratio" |
        "block-size" |
        "contain-intrinsic-width" |
        "contain-intrinsic-height" |
        "contain-intrinsic-inline-size" |
        "contain-intrinsic-block-size" |
        "border-bottom-style" |
        "border-bottom-width" |
        "border-left-style" |
        "border-left-width" |
        "border-right-style" |
        "border-right-width" |
        "border-spacing" |
        "border-top-style" |
        "border-top-width" |
        "bottom" |
        "box-sizing" |
        "caption-side" |
        "clear" |
        "column-gap" |
        "direction" |
        "dominant-baseline" |
        "empty-cells" |
        "flex-basis" |
        "flex-direction" |
        "flex-grow" |
        "flex-shrink" |
        "flex-wrap" |
        "flow-tolerance" |
        "font-family" |
        "font-feature-settings" |
        "font-kerning" |
        "font-language-override" |
        "font-optical-sizing" |
        "font-size" |
        "font-size-adjust" |
        "font-stretch" |
        "font-style" |
        "font-synthesis-position" |
        "font-synthesis-small-caps" |
        "font-synthesis-style" |
        "font-synthesis-weight" |
        "font-variant" |
        "font-variant-alternates" |
        "font-variant-caps" |
        "font-variant-east-asian" |
        "font-variant-emoji" |
        "font-variant-ligatures" |
        "font-variant-numeric" |
        "font-variant-position" |
        "font-variation-settings" |
        "font-weight" |
        "grid-auto-columns" |
        "grid-auto-flow" |
        "grid-auto-rows" |
        "grid-column-end" |
        "grid-column-start" |
        "grid-row-end" |
        "grid-row-start" |
        "grid-template-areas" |
        "grid-template-columns" |
        "grid-template-rows" |
        "height" |
        "hyphens" |
        "inline-size" |
        "inset-block-end" |
        "inset-block-start" |
        "inset-inline-end" |
        "inset-inline-start" |
        "justify-content" |
        "justify-items" |
        "justify-self" |
        "left" |
        "letter-spacing" |
        "line-break" |
        "line-height" |
        "margin-block-end" |
        "margin-block-start" |
        "margin-bottom" |
        "margin-inline-end" |
        "margin-inline-start" |
        "margin-left" |
        "margin-right" |
        "margin-top" |
        "max-block-size" |
        "max-height" |
        "max-inline-size" |
        "max-width" |
        "min-block-size" |
        "min-height" |
        "min-inline-size" |
        "min-width" |
        "order" |
        "orphans" |
        "overflow" |
        "overflow-wrap" |
        "overflow-x" |
        "overflow-y" |
        "padding-block-end" |
        "padding-block-start" |
        "padding-bottom" |
        "padding-inline-end" |
        "padding-inline-start" |
        "padding-left" |
        "padding-right" |
        "padding-top" |
        "right" |
        "row-gap" |
        "ruby-align" |
        "ruby-merge" |
        "ruby-overhang" |
        "ruby-position" |
        "text-align" |
        "text-anchor" |
        "text-combine-upright" |
        "text-indent" |
        "text-orientation" |
        "text-transform" |
        "top" |
        "unicode-bidi" |
        "vertical-align" |
        "white-space-collapse" | "white-space-trim" | "text-wrap" | "text-wrap-mode" | "text-wrap-style" |
        "text-autospace" | "text-spacing" | "text-spacing-trim" | "hanging-punctuation" |
        "wrap-before" | "wrap-after" | "wrap-inside" | "word-space-transform" | "text-justify" | "text-group-align" |
        "white-space" |
        "widows" |
        "width" |
        "word-break" |
        "word-spacing" |
        "writing-mode" |
        "zoom" => Layout,
        // Rebuild
        "animation-composition" |
        "animation-delay" |
        "animation-direction" |
        "animation-duration" |
        "animation-fill-mode" |
        "animation-iteration-count" |
        "animation-name" |
        "animation-play-state" |
        "animation-timeline" |
        "animation-timing-function" |
        "border-collapse" |
        // Not evaluated (no size-container-query matching yet), but a real
        // change establishes/removes a query container and containment, so
        // it is conservatively a full rebuild rather than a narrower kind.
        "container-name" |
        "container-type" |
        "content" |
        "counter-increment" |
        "counter-reset" |
        "counter-set" |
        "display" |
        "float" |
        "list-style-image" |
        "list-style-position" |
        "list-style-type" |
        "position" |
        "quotes" |
        "translate" | "rotate" | "scale" |
        "transform-style" |
        "transition-behavior" |
        "transition-delay" |
        "transition-duration" |
        "transition-property" |
        "transition-timing-function" |
        "z-index" => Rebuild,
        _ => return Option::None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_registered_longhand_has_exactly_one_classification() {
        let names: Vec<_> = crate::computed::known_properties().collect();
        let unique: BTreeSet<_> = names.iter().copied().collect();
        assert_eq!(names.len(), unique.len(), "duplicate property registry entry");
        let mut counts = [0; 4];
        for name in &names {
            let kind = classify_style_property(name).unwrap_or_else(|| panic!("unclassified property: {name}"));
            counts[match kind {
                StyleChangeKind::CompositorOnly => 0,
                StyleChangeKind::PaintOnly => 1,
                StyleChangeKind::Layout => 2,
                StyleChangeKind::Rebuild => 3,
                StyleChangeKind::None => panic!("registered property classified None: {name}"),
            }] += 1;
        }
        // RC-32 added outline-color/-style/-width/-offset and CSS-30 added
        // `background-position-x`/`-y`, all PaintOnly. CSS39 adds thirteen
        // registered text policy axes, all Layout. CSS42 adds six image-border
        // properties, all PaintOnly.
        // The complete grid-template producer registers areas as one more Layout longhand.
        // Flow tolerance adds one more Layout longhand to the ready grid producer.
        assert_eq!(counts, [19, 53, 144, 34]);
        assert_eq!(classify_style_property("not-a-property"), None);
    }
}
