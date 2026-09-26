//! Specified-to-computed value processing.
//!
//! This is the deep style-value module: cascade supplies one winning string
//! per property, and callers receive a [`ComputedStyle`]. Custom-property
//! inheritance, dependency cycles, `var()` substitution, CSS-wide defaulting,
//! and supported shorthand expansion stay behind this seam.

use crate::syntax::{Token, serialize, tokenize};
#[path = "attr_values.rs"]
mod attr_values;
use crate::values::{self, ComputedValue};
use crate::media_queries::MediaEnvironment;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, PartialEq)]
enum CustomValue {
    Valid(Vec<Token>),
    Invalid,
}

/// Final computed properties for one element. Normal property names are
/// ASCII-lowercase. Custom properties remain private implementation state and
/// preserve their case-sensitive names for inheritance and `var()` lookup.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedStyle {
    props: BTreeMap<String, String>,
    typed: BTreeMap<String, ComputedValue>,
    custom: BTreeMap<String, CustomValue>,
    relative_colors: BTreeMap<String, String>,
    root_font_px: f64,
    effective_zoom: f64,
    author_specified: BTreeSet<String>,
    url_bases: BTreeMap<String, String>,
}

impl Default for ComputedStyle {
    fn default() -> Self {
        Self {
            props: BTreeMap::new(),
            typed: BTreeMap::new(),
            custom: BTreeMap::new(),
            relative_colors: BTreeMap::new(),
            root_font_px: 16.0,
            effective_zoom: 1.0,
            author_specified: BTreeSet::new(),
            url_bases: BTreeMap::new(),
        }
    }
}

impl ComputedStyle {
    pub(crate) fn apply_compositor_values(
        &mut self,
        opacity: Option<f32>,
        transform: Option<(&str, Vec<crate::values::Transform>)>,
    ) {
        if let Some(value) = opacity {
            let css = value.to_string();
            self.typed.insert("opacity".into(), ComputedValue::Number(css.parse().expect("finite opacity")));
            self.props.insert("opacity".into(), css);
        }
        if let Some((css, value)) = transform {
            self.props.insert("transform".into(), css.to_owned());
            self.typed.insert("transform".into(), ComputedValue::Transform(value));
        }
    }

    /// Source stylesheet URL of a winning URL-bearing property.
    pub fn url_base(&self, property: &str) -> Option<&str> {
        self.url_bases.get(property).map(String::as_str)
    }

    pub(crate) fn set_url_base(&mut self, property: &str, base: Option<&str>) {
        self.url_bases.remove(property);
        if let Some(base) = base {
            self.url_bases.insert(property.to_owned(), base.to_owned());
        }
    }
    /// Product of this element's zoom and its ancestors' zoom values.
    /// CSS Viewport applies this at the used-value stage, while computed
    /// lengths and `get()` remain unscaled for CSSOM-style consumers.
    pub fn effective_zoom(&self) -> f64 {
        self.effective_zoom
    }

    /// Scale an absolute CSS length at the used-value boundary.
    pub fn scale_used_length(&self, unscaled_px: f64) -> f64 {
        unscaled_px * self.effective_zoom
    }

    /// The zoomed used value of a typed absolute length. Percentages, `auto`,
    /// and expressions needing a layout basis stay with the layout consumer.
    pub fn used_length_px(&self, property: &str) -> Option<f64> {
        match self.typed(property) {
            Some(ComputedValue::LengthPercentage(values::LengthPercentage::Px(px))) => {
                Some(self.scale_used_length(*px))
            }
            _ => None,
        }
    }

    /// Parsed once during computation. Custom properties are never exposed.
    /// `Unparsed` marks a standard/unknown grammar outside the typed prototype.
    pub fn typed(&self, property: &str) -> Option<&ComputedValue> {
        self.typed.get(property)
    }

    /// Whether this element's final cascaded winner came from an author
    /// declaration (stylesheet or inline style), after origin/layer rollback.
    /// Query longhands; shorthands mark each expanded longhand. Custom names
    /// remain case-sensitive; normal property names are ASCII-insensitive.
    ///
    /// Explicit `initial`, `inherit`, and `unset` count, as do author winners
    /// that become invalid after variable substitution and default as unset.
    /// Inheritance without a local declaration, UA defaults, and presentational
    /// hints do not count. `revert`/`revert-layer` report the restored winner.
    /// Synthetic `set`/FromIterator values are not author declarations.
    pub fn is_author_specified(&self, property: &str) -> bool {
        if property.starts_with("--") {
            self.author_specified.contains(property)
        } else {
            self.author_specified.contains(&property.to_ascii_lowercase())
        }
    }

    pub(crate) fn mark_author_specified(&mut self, property: &str) {
        self.author_specified.insert(property.to_owned());
    }
    /// A logical and physical longhand name the same computed value after
    /// writing-mode and direction have been resolved by the cascade.
    pub(crate) fn mirror_logical(&mut self, logical: &str, physical: &str) {
        if let Some(value) = self.props.get(physical).cloned() {
            self.props.insert(logical.to_owned(), value);
        }
        if let Some(value) = self.typed.get(physical).cloned() {
            self.typed.insert(logical.to_owned(), value);
        }
        if self.author_specified.contains(physical) {
            self.author_specified.insert(logical.to_owned());
        } else {
            self.author_specified.remove(logical);
        }
    }

    fn rebuild_typed(&mut self) {
        self.typed = self
            .props
            .iter()
            .map(|(name, css)| {
                (
                    name.clone(),
                    values::parse(name, css).unwrap_or(ComputedValue::Unparsed),
                )
            })
            .collect();
    }

    fn resolve_relative_colors(&mut self) {
        let Some(color) = self.props.get("color") else { return };
        let Some([red, green, blue, alpha]) = crate::color::parse_css_color(color) else { return };
        let resolved = format!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}");
        for (property, source) in &self.relative_colors {
            let value = if relative_currentcolor_identity(source) {
                Some(resolved.clone())
            } else {
                resolve_currentcolor_mix(source, &resolved)
            };
            if let Some(value) = value {
                self.props.insert(property.clone(), value);
            }
        }
    }

    fn resolve_typed_units(&mut self) {
        let font = match self.typed.get("font-size") {
            Some(ComputedValue::LengthPercentage(values::LengthPercentage::Px(px))) => *px,
            _ => 16.0,
        };
        for (name, value) in &mut self.typed {
            values::resolve_font_units(value, font, self.root_font_px, name == "line-height");
        }
    }

    pub fn get(&self, property: &str) -> Option<&str> {
        self.props.get(property).map(String::as_str)
    }

    /// Sorted normal (property, value) pairs. Custom properties are omitted:
    /// they are substitution state, not values consumed by layout or paint.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.props
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Public mutation for layout-time synthesis. Layout only materializes
    /// normal used-value hints, so custom properties intentionally stay behind
    /// the computed-value seam.
    pub fn set(&mut self, property: &str, value: &str) {
        if property.starts_with("--") {
            return;
        }
        let prior_zoom = self.get("zoom").and_then(used_zoom_factor).unwrap_or(1.0);
        for (longhand, expanded) in expand_declaration(property, value) {
            let expanded = if longhand.starts_with("animation-") || longhand.starts_with("transition-") {
                let Some(canonical) = crate::animation::canonical(&longhand, &expanded) else { continue };
                canonical
            } else { expanded };
            let typed = values::parse(&longhand, &expanded).unwrap_or(ComputedValue::Unparsed);
            self.typed.insert(longhand.clone(), typed);
            // Layout-time synthesis replaces this value, not an author rule.
            self.author_specified.remove(&longhand);
            self.relative_colors.remove(&longhand);
            self.url_bases.remove(&longhand);
            self.props.insert(longhand, expanded);
        }
        let next_zoom = self.get("zoom").and_then(used_zoom_factor).unwrap_or(1.0);
        self.effective_zoom *= next_zoom / prior_zoom;
        self.resolve_typed_units();
    }

    #[cfg(test)]
    pub(crate) fn custom_serialized(&self, name: &str) -> Option<Option<String>> {
        self.custom.get(name).map(|value| match value {
            CustomValue::Valid(tokens) => Some(serialize_computed(tokens)),
            CustomValue::Invalid => None,
        })
    }
}

impl FromIterator<(String, String)> for ComputedStyle {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        let mut style = Self::default();
        for (name, value) in iter {
            if name.starts_with("--") {
                style
                    .custom
                    .insert(name, CustomValue::Valid(tokenize(&value)));
            } else {
                style.props.insert(name.to_ascii_lowercase(), value);
            }
        }
        style.rebuild_typed();
        style
    }
}

#[derive(Clone, Copy)]
struct PropertyMetadata {
    name: &'static str,
    initial: &'static str,
    inherited: bool,
}

// Every normal property for which the engine currently has cascade, layout,
// or paint behavior. Keeping initial/inheritance metadata in one registry is
// what makes CSS-wide keywords and `all` property-aware.
const PROPERTIES: &[PropertyMetadata] = &[
    PropertyMetadata { name: "animation-name", initial: "none", inherited: false },
    PropertyMetadata { name: "animation-duration", initial: "0s", inherited: false },
    PropertyMetadata { name: "animation-timing-function", initial: "ease", inherited: false },
    PropertyMetadata { name: "animation-delay", initial: "0s", inherited: false },
    PropertyMetadata { name: "animation-iteration-count", initial: "1", inherited: false },
    PropertyMetadata { name: "animation-direction", initial: "normal", inherited: false },
    PropertyMetadata { name: "animation-fill-mode", initial: "none", inherited: false },
    PropertyMetadata { name: "animation-play-state", initial: "running", inherited: false },
    PropertyMetadata { name: "animation-composition", initial: "replace", inherited: false },
    PropertyMetadata { name: "animation-timeline", initial: "auto", inherited: false },
    PropertyMetadata { name: "transition-property", initial: "all", inherited: false },
    PropertyMetadata { name: "transition-duration", initial: "0s", inherited: false },
    PropertyMetadata { name: "transition-timing-function", initial: "ease", inherited: false },
    PropertyMetadata { name: "transition-delay", initial: "0s", inherited: false },
    PropertyMetadata { name: "transition-behavior", initial: "normal", inherited: false },
    PropertyMetadata { name: "background-image", initial: "none", inherited: false },
    PropertyMetadata { name: "background-position", initial: "0% 0%", inherited: false },
    PropertyMetadata { name: "background-size", initial: "auto", inherited: false },
    PropertyMetadata { name: "background-repeat", initial: "repeat", inherited: false },
    PropertyMetadata { name: "background-origin", initial: "padding-box", inherited: false },
    PropertyMetadata { name: "background-attachment", initial: "scroll", inherited: false },
    PropertyMetadata { name: "image-orientation", initial: "from-image", inherited: true },
    PropertyMetadata { name: "object-fit", initial: "fill", inherited: false },
    PropertyMetadata { name: "object-position", initial: "50% 50%", inherited: false },
    PropertyMetadata { name: "aspect-ratio", initial: "auto", inherited: false },
    PropertyMetadata { name: "border-spacing", initial: "0px", inherited: true },
    PropertyMetadata { name: "content", initial: "normal", inherited: false },
    PropertyMetadata { name: "counter-increment", initial: "none", inherited: false },
    PropertyMetadata { name: "counter-reset", initial: "none", inherited: false },
    PropertyMetadata { name: "counter-set", initial: "none", inherited: false },
    PropertyMetadata {
        name: "align-content",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata {
        name: "align-items",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata {
        name: "align-self",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "background-color",
        initial: "transparent",
        inherited: false,
    },
    PropertyMetadata {
        name: "background-clip",
        initial: "border-box",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-bottom-color",
        initial: "currentcolor",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-bottom-style",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-bottom-width",
        initial: "medium",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-collapse",
        initial: "separate",
        inherited: true,
    },
    PropertyMetadata {
        name: "border-left-color",
        initial: "currentcolor",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-left-style",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-left-width",
        initial: "medium",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-right-color",
        initial: "currentcolor",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-right-style",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-right-width",
        initial: "medium",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-top-color",
        initial: "currentcolor",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-top-style",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-top-width",
        initial: "medium",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-top-left-radius",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-top-right-radius",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-bottom-right-radius",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "border-bottom-left-radius",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "bottom",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "box-sizing",
        initial: "content-box",
        inherited: false,
    },
    PropertyMetadata {
        name: "box-shadow",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "caption-side",
        initial: "top",
        inherited: true,
    },
    PropertyMetadata {
        name: "clear",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "color",
        initial: "black",
        inherited: true,
    },
    PropertyMetadata {
        name: "column-gap",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata {
        name: "cursor",
        initial: "auto",
        inherited: true,
    },
    PropertyMetadata {
        name: "direction",
        initial: "ltr",
        inherited: true,
    },
    PropertyMetadata {
        name: "display",
        initial: "inline",
        inherited: false,
    },
    PropertyMetadata {
        name: "empty-cells",
        initial: "show",
        inherited: true,
    },
    PropertyMetadata {
        name: "flex-basis",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "flex-direction",
        initial: "row",
        inherited: false,
    },
    PropertyMetadata {
        name: "flex-grow",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "flex-shrink",
        initial: "1",
        inherited: false,
    },
    PropertyMetadata {
        name: "flex-wrap",
        initial: "nowrap",
        inherited: false,
    },
    PropertyMetadata {
        name: "float",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "font-family",
        initial: "serif",
        inherited: true,
    },
    PropertyMetadata { name: "font-feature-settings", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-kerning", initial: "auto", inherited: true },
    PropertyMetadata { name: "font-language-override", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-optical-sizing", initial: "auto", inherited: true },
    PropertyMetadata { name: "font-palette", initial: "normal", inherited: true },
    PropertyMetadata {
        name: "font-size",
        initial: "medium",
        inherited: true,
    },
    PropertyMetadata {
        name: "font-stretch",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "font-style",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "font-variant",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata { name: "font-variant-ligatures", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-alternates", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-caps", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-east-asian", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-emoji", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-numeric", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variant-position", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-variation-settings", initial: "normal", inherited: true },
    PropertyMetadata { name: "font-synthesis-weight", initial: "auto", inherited: true },
    PropertyMetadata { name: "font-synthesis-style", initial: "auto", inherited: true },
    PropertyMetadata { name: "font-synthesis-small-caps", initial: "auto", inherited: true },
    PropertyMetadata { name: "font-synthesis-position", initial: "auto", inherited: true },
    PropertyMetadata {
        name: "font-weight",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "fill",
        initial: "black",
        inherited: true,
    },
    PropertyMetadata {
        name: "fill-opacity",
        initial: "1",
        inherited: true,
    },
    PropertyMetadata {
        name: "grid-auto-columns",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-auto-flow",
        initial: "row",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-auto-rows",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-column-end",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-column-start",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-row-end",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-row-start",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-template-columns",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "grid-template-rows",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "height",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "hyphens",
        initial: "manual",
        inherited: true,
    },
    PropertyMetadata {
        name: "justify-content",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata {
        name: "justify-items",
        initial: "legacy",
        inherited: false,
    },
    PropertyMetadata {
        name: "justify-self",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "letter-spacing",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "left",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "line-break",
        initial: "auto",
        inherited: true,
    },
    PropertyMetadata {
        name: "line-height",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "list-style-image",
        initial: "none",
        inherited: true,
    },
    PropertyMetadata {
        name: "list-style-position",
        initial: "outside",
        inherited: true,
    },
    PropertyMetadata {
        name: "list-style-type",
        initial: "disc",
        inherited: true,
    },
    PropertyMetadata {
        name: "margin-bottom",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-block-end",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-block-start",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-inline-end",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-inline-start",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-left",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-right",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "margin-top",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "max-height",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "max-width",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "min-height",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "min-width",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "opacity",
        initial: "1",
        inherited: false,
    },
    PropertyMetadata {
        name: "order",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "orphans",
        initial: "2",
        inherited: true,
    },
    PropertyMetadata {
        name: "overflow",
        initial: "visible",
        inherited: false,
    },
    PropertyMetadata {
        name: "overflow-x",
        initial: "visible",
        inherited: false,
    },
    PropertyMetadata {
        name: "overflow-y",
        initial: "visible",
        inherited: false,
    },
    PropertyMetadata {
        name: "overflow-wrap",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "padding-bottom",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "padding-left",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "padding-right",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "padding-top",
        initial: "0",
        inherited: false,
    },
    PropertyMetadata {
        name: "position",
        initial: "static",
        inherited: false,
    },
    PropertyMetadata {
        name: "quotes",
        initial: "auto",
        inherited: true,
    },
    PropertyMetadata {
        name: "right",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "row-gap",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata { name: "ruby-align", initial: "space-around", inherited: true },
    PropertyMetadata { name: "ruby-position", initial: "alternate", inherited: true },
    PropertyMetadata { name: "ruby-overhang", initial: "auto", inherited: true },
    PropertyMetadata { name: "ruby-merge", initial: "separate", inherited: true },
    PropertyMetadata {
        name: "text-align",
        initial: "start",
        inherited: true,
    },
    PropertyMetadata {
        name: "text-combine-upright",
        initial: "none",
        inherited: true,
    },
    PropertyMetadata {
        name: "text-decoration",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "text-indent",
        initial: "0",
        inherited: true,
    },
    PropertyMetadata {
        name: "text-orientation",
        initial: "mixed",
        inherited: true,
    },
    PropertyMetadata {
        name: "text-transform",
        initial: "none",
        inherited: true,
    },
    PropertyMetadata {
        name: "top",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "transform",
        initial: "none",
        inherited: false,
    },
    PropertyMetadata {
        name: "transform-style",
        initial: "flat",
        inherited: false,
    },
    PropertyMetadata {
        name: "unicode-bidi",
        initial: "normal",
        inherited: false,
    },
    PropertyMetadata {
        name: "vertical-align",
        initial: "baseline",
        inherited: false,
    },
    PropertyMetadata {
        name: "visibility",
        initial: "visible",
        inherited: true,
    },
    PropertyMetadata {
        name: "white-space",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "widows",
        initial: "2",
        inherited: true,
    },
    PropertyMetadata {
        name: "width",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata {
        name: "word-break",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "word-spacing",
        initial: "normal",
        inherited: true,
    },
    PropertyMetadata {
        name: "writing-mode",
        initial: "horizontal-tb",
        inherited: true,
    },
    PropertyMetadata { name: "inline-size", initial: "auto", inherited: false },
    PropertyMetadata { name: "block-size", initial: "auto", inherited: false },
    PropertyMetadata { name: "min-inline-size", initial: "auto", inherited: false },
    PropertyMetadata { name: "min-block-size", initial: "auto", inherited: false },
    PropertyMetadata { name: "max-inline-size", initial: "none", inherited: false },
    PropertyMetadata { name: "max-block-size", initial: "none", inherited: false },
    PropertyMetadata { name: "padding-inline-start", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-inline-end", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-block-start", initial: "0", inherited: false },
    PropertyMetadata { name: "padding-block-end", initial: "0", inherited: false },
    PropertyMetadata { name: "inset-inline-start", initial: "auto", inherited: false },
    PropertyMetadata { name: "inset-inline-end", initial: "auto", inherited: false },
    PropertyMetadata { name: "inset-block-start", initial: "auto", inherited: false },
    PropertyMetadata { name: "inset-block-end", initial: "auto", inherited: false },
    PropertyMetadata { name: "stroke", initial: "none", inherited: true },
    PropertyMetadata { name: "stroke-width", initial: "1px", inherited: true },
    PropertyMetadata { name: "stroke-opacity", initial: "1", inherited: true },
    PropertyMetadata { name: "stroke-linecap", initial: "butt", inherited: true },
    PropertyMetadata { name: "stroke-linejoin", initial: "miter", inherited: true },
    PropertyMetadata { name: "stroke-miterlimit", initial: "4", inherited: true },
    PropertyMetadata { name: "stroke-dasharray", initial: "none", inherited: true },
    PropertyMetadata { name: "stroke-dashoffset", initial: "0", inherited: true },
    PropertyMetadata { name: "fill-rule", initial: "nonzero", inherited: true },
    PropertyMetadata { name: "clip-rule", initial: "nonzero", inherited: true },
    PropertyMetadata { name: "text-anchor", initial: "start", inherited: true },
    PropertyMetadata { name: "dominant-baseline", initial: "auto", inherited: true },
    PropertyMetadata { name: "clip-path", initial: "none", inherited: false },
    PropertyMetadata { name: "mask", initial: "none", inherited: false },
    PropertyMetadata { name: "vector-effect", initial: "none", inherited: false },
    PropertyMetadata { name: "font-size-adjust", initial: "none", inherited: true },
    PropertyMetadata {
        name: "z-index",
        initial: "auto",
        inherited: false,
    },
    PropertyMetadata { name: "zoom", initial: "1", inherited: false },
];

pub(crate) fn known_properties() -> impl Iterator<Item = &'static str> {
    PROPERTIES.iter().map(|property| property.name)
}
pub(crate) fn is_inherited_property(name: &str) -> bool {
    metadata(name).is_some_and(|property| property.inherited)
}

fn metadata(name: &str) -> Option<PropertyMetadata> {
    PROPERTIES
        .iter()
        .copied()
        .find(|property| property.name == name)
}

pub(crate) fn is_supported_property(name: &str) -> bool {
    metadata(&name.to_ascii_lowercase()).is_some()
}

/// Expands shorthands before cascade ranking so each resulting longhand keeps
/// the source declaration's origin, importance, specificity, and order.
pub(crate) fn expand_declaration(property: &str, value: &str) -> Vec<(String, String)> {
    const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];
    let property = if property.starts_with("--") {
        property.to_owned()
    } else {
        property.to_ascii_lowercase()
    };
    if property == "font-synthesis" {
        return crate::font_properties::expand_synthesis(value).unwrap_or_default();
    }
    if property == "list-style" {
        if contains_var_function(value) {
            return ["list-style-type", "list-style-image", "list-style-position"]
                .into_iter().map(|name| (name.to_owned(), deferred(DEFERRED_LIST_STYLE, value))).collect();
        }
        return expand_list_style(value).unwrap_or_default();
    }
    if matches!(property.as_str(), "animation" | "transition") {
        if contains_var_function(value) {
            let (prefix, names): (&str, &[&str]) = if property == "animation" {
                (DEFERRED_ANIMATION, &["animation-name", "animation-duration", "animation-timing-function", "animation-delay", "animation-iteration-count", "animation-direction", "animation-fill-mode", "animation-play-state", "animation-composition", "animation-timeline"])
            } else {
                (DEFERRED_TRANSITION, &["transition-property", "transition-duration", "transition-timing-function", "transition-delay", "transition-behavior"])
            };
            return names.iter().map(|name| ((*name).to_owned(), deferred(prefix, value))).collect();
        }
        return crate::animation::expand_shorthand(&property, value).unwrap_or_default();
    }
    if metadata(&property).is_some()
        && css_wide_keyword(value).is_none()
        && !contains_var_function(value)
        && !contains_attr_function(value)
        && !matches!(property.as_str(), "opacity" | "fill-opacity" | "overflow")
        && values::parse(&property, value).is_none()
    {
        return Vec::new();
    }
    let parts: Vec<&str> = value.split_ascii_whitespace().collect();
    let logical_parts = if matches!(property.as_str(),
        "margin-inline" | "margin-block" | "padding-inline" | "padding-block"
        | "inset-inline" | "inset-block") {
        top_level_components(&tokenize(value))
            .map(|parts| parts.into_iter().map(serialize).collect::<Vec<_>>())
    } else {
        None
    };
    match property.as_str() {
        "all" if contains_var_function(value) => PROPERTIES
            .iter()
            .filter(|property| !matches!(property.name, "direction" | "unicode-bidi"))
            .map(|property| (property.name.to_owned(), deferred(DEFERRED_ALL, value)))
            .collect(),
        "all" if css_wide_keyword(value).is_some() => PROPERTIES
            .iter()
            .filter(|property| !matches!(property.name, "direction" | "unicode-bidi"))
            .map(|property| (property.name.to_owned(), value.to_owned()))
            .collect(),
        "margin" | "padding" | "border-width" | "border-style" | "border-color" => {
            if parts.is_empty() {
                return vec![(property, value.to_owned())];
            }
            SIDES
                .iter()
                .enumerate()
                .map(|(index, side)| {
                    let part = parts[replicate_index(parts.len(), index)];
                    (longhand_name(&property, side), part.to_owned())
                })
                .collect()
        }
        "margin-inline" | "margin-block" | "padding-inline" | "padding-block"
        | "inset-inline" | "inset-block"
            if logical_parts.as_ref().is_some_and(|parts| (1..=2).contains(&parts.len())) => {
            let parts = logical_parts.as_ref().unwrap();
            vec![
                (format!("{property}-start"), parts[0].clone()),
                (format!("{property}-end"), parts[parts.len() - 1].clone()),
            ]
        },
        "inset" if !parts.is_empty() => SIDES
            .iter()
            .enumerate()
            .map(|(index, side)| {
                (
                    (*side).to_owned(),
                    parts[replicate_index(parts.len(), index)].to_owned(),
                )
            })
            .collect(),
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            expand_side_border(&property, value).unwrap_or_default()
        }
        // Rollback/defaulting must reach every border longhand. Treat a
        // var-bearing border as a deferred whole value for the same reason.
        "border" if css_wide_keyword(value).is_some() || contains_var_function(value) => SIDES
            .iter()
            .flat_map(|side| {
                expand_side_border(&format!("border-{side}"), value).unwrap_or_default()
            })
            .collect(),
        "border" if !parts.is_empty() => {
            let mut width = "medium";
            let mut style = "none";
            let mut color = "currentcolor";
            for part in parts {
                if is_border_style(part) {
                    style = part;
                } else if is_border_width(part) {
                    width = part;
                } else {
                    color = part;
                }
            }
            let mut expanded = Vec::with_capacity(12);
            for side in SIDES {
                expanded.push((format!("border-{side}-width"), width.to_owned()));
                expanded.push((format!("border-{side}-style"), style.to_owned()));
                expanded.push((format!("border-{side}-color"), color.to_owned()));
            }
            expanded
        }
        "background" if contains_var_function(value) => PROPERTIES.iter()
            .filter(|p| p.name.starts_with("background-"))
            .map(|p| (p.name.to_owned(), deferred(DEFERRED_BACKGROUND, value))).collect(),
        "background" => expand_basic_background(value).unwrap_or_default(),
        "border-radius" if css_wide_keyword(value).is_some() => [
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), value.to_owned()))
        .collect(),
        "border-radius" if contains_var_function(value) => deferred_border_radius(value),
        "border-radius" => expand_border_radius(value).unwrap_or_default(),
        "background-clip"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_background_clip(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "box-shadow"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_box_shadow(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "transform"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_transform(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "transform-style"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "flat" | "preserve-3d"
                ) =>
        {
            vec![(property, value.to_owned())]
        }
        "opacity" | "fill-opacity"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_opacity(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "z-index"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_z_index(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "overflow" if css_wide_keyword(value).is_some() => vec![
            ("overflow-x".to_owned(), value.to_owned()),
            ("overflow-y".to_owned(), value.to_owned()),
        ],
        "overflow" if contains_var_function(value) => deferred_overflow(value),
        "overflow"
            if (1..=2).contains(&parts.len()) && parts.iter().all(|part| valid_overflow(part)) =>
        {
            vec![
                ("overflow-x".to_owned(), parts[0].to_owned()),
                (
                    "overflow-y".to_owned(),
                    parts.get(1).copied().unwrap_or(parts[0]).to_owned(),
                ),
            ]
        }
        "overflow-x" | "overflow-y"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_overflow(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_radius_pair(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "background-clip"
        | "box-shadow"
        | "transform"
        | "transform-style"
        | "opacity"
        | "fill-opacity"
        | "z-index"
        | "overflow"
        | "overflow-x"
        | "overflow-y"
        | "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => Vec::new(),
        "font" if contains_var_function(value) => FONT_LONGHANDS
            .iter()
            .map(|name| ((*name).to_owned(), deferred(DEFERRED_FONT, value)))
            .collect(),
        "font" => {
            expand_font_shorthand(value).unwrap_or_else(|| vec![(property, value.to_owned())])
        }
        "grid-column-gap" if css_wide_keyword(value).is_some() || valid_gap_value(value) => {
            vec![("column-gap".to_owned(), value.to_owned())]
        }
        "grid-row-gap" if css_wide_keyword(value).is_some() || valid_gap_value(value) => {
            vec![("row-gap".to_owned(), value.to_owned())]
        }
        "grid-gap"
            if !parts.is_empty()
                && parts.len() <= 2
                && (css_wide_keyword(value).is_some()
                    || parts.iter().all(|part| valid_gap_value(part))) =>
        {
            let row = parts[0];
            let column = parts.get(1).copied().unwrap_or(row);
            vec![
                ("row-gap".to_owned(), row.to_owned()),
                ("column-gap".to_owned(), column.to_owned()),
            ]
        }
        "gap"
            if !parts.is_empty()
                && parts.len() <= 2
                && (css_wide_keyword(value).is_some()
                    || parts.iter().all(|part| valid_gap_value(part))) =>
        {
            let row = parts[0];
            let column = parts.get(1).copied().unwrap_or(row);
            vec![
                ("row-gap".to_owned(), row.to_owned()),
                ("column-gap".to_owned(), column.to_owned()),
            ]
        }
        "grid-column-start" | "grid-column-end" | "grid-row-start" | "grid-row-end"
            if css_wide_keyword(value).is_some()
                || contains_var_function(value)
                || valid_grid_line(value) =>
        {
            vec![(property, value.to_owned())]
        }
        "grid-column-start" | "grid-column-end" | "grid-row-start" | "grid-row-end" => {
            Vec::new()
        }
        "grid-column" | "grid-row" if css_wide_keyword(value).is_some() => {
            let prefix = property.as_str();
            vec![
                (format!("{prefix}-start"), value.to_owned()),
                (format!("{prefix}-end"), value.to_owned()),
            ]
        }
        "grid-column" | "grid-row" if !parts.is_empty() => {
            let prefix = property.as_str();
            let (start, explicit_end) = match value.split_once('/') {
                Some((start, end)) if !end.contains('/') => (start.trim(), Some(end.trim())),
                Some(_) => return Vec::new(),
                None => (value.trim(), None),
            };
            if !valid_grid_line(start) || explicit_end.is_some_and(|end| !valid_grid_line(end)) {
                return Vec::new();
            }
            let end = explicit_end.unwrap_or_else(|| {
                if grid_line_custom_ident(start) { start } else { "auto" }
            });
            vec![
                (format!("{prefix}-start"), start.to_owned()),
                (format!("{prefix}-end"), end.to_owned()),
            ]
        }
        "grid-template" if contains_var_function(value) => deferred_grid_template(value),
        "grid-template" => expand_grid_template(value).unwrap_or_default(),
        "grid" if css_wide_keyword(value).is_some() => [
            "grid-template-rows", "grid-template-columns", "grid-auto-rows",
            "grid-auto-columns", "grid-auto-flow",
        ].into_iter().map(|name| (name.to_owned(), value.to_owned())).collect(),
        "grid" if contains_var_function(value) => {
            let mut template = deferred_grid_template(value);
            template.extend([
                ("grid-auto-rows".to_owned(), "auto".to_owned()),
                ("grid-auto-columns".to_owned(), "auto".to_owned()),
                ("grid-auto-flow".to_owned(), "row".to_owned()),
            ]);
            template
        },
        "grid" => expand_grid_template(value).map(|mut template| {
            template.extend([
                ("grid-auto-rows".to_owned(), "auto".to_owned()),
                ("grid-auto-columns".to_owned(), "auto".to_owned()),
                ("grid-auto-flow".to_owned(), "row".to_owned()),
            ]);
            template
        }).unwrap_or_default(),
        "flex-flow" if !parts.is_empty() => {
            if css_wide_keyword(value).is_some() {
                vec![
                    ("flex-direction".to_owned(), value.to_owned()),
                    ("flex-wrap".to_owned(), value.to_owned()),
                ]
            } else if contains_var_function(value) {
                deferred_flex_flow(value)
            } else {
                expand_flex_flow(value).unwrap_or_default()
            }
        }
        "flex" if !parts.is_empty() => {
            if css_wide_keyword(value).is_some() {
                vec![
                    ("flex-grow".to_owned(), value.to_owned()),
                    ("flex-shrink".to_owned(), value.to_owned()),
                    ("flex-basis".to_owned(), value.to_owned()),
                ]
            } else {
                match expand_flex_shorthand(&parts) {
                    Some((grow, shrink, basis)) => vec![
                        ("flex-grow".to_owned(), grow),
                        ("flex-shrink".to_owned(), shrink),
                        ("flex-basis".to_owned(), basis),
                    ],
                    // An invalid `flex` shorthand emits no cascade candidates,
                    // so it cannot override earlier valid longhands (same rule
                    // as flex-flow/grid-template).
                    None => Vec::new(),
                }
            }
        }
        _ => vec![(property, value.to_owned())],
    }
}

// Existing simple background grammar, with proper omitted-longhand resets.
// Layered image/position/size shorthand parsing remains a separate ticket.
fn expand_basic_background(value: &str) -> Option<Vec<(String, String)>> {
    let value = value.trim();
    let wide = css_wide_keyword(value).is_some();
    let none = value.eq_ignore_ascii_case("none");
    if !wide && !none && values::parse("background-color", value).is_none() { return None; }
    Some(PROPERTIES.iter().filter(|p| p.name.starts_with("background-"))
        .map(|p| (p.name.to_owned(), if wide || (p.name == "background-color" && !none) {
            value.to_owned()
        } else { p.initial.to_owned() })).collect())
}

fn expand_list_style(value: &str) -> Option<Vec<(String, String)>> {
    let value = value.trim();
    if css_wide_keyword(value).is_some() {
        return Some(["list-style-type", "list-style-image", "list-style-position"]
            .into_iter().map(|name| (name.to_owned(), value.to_owned())).collect());
    }
    let tokens = tokenize(value);
    let parts = top_level_components(&tokens)?;
    if parts.is_empty() { return None; }
    let mut style_type = None;
    let mut image = None;
    let mut position = None;
    let mut none = 0usize;
    for part in parts {
        let css = serialize(part);
        match part {
            [Token::Ident(name)] if name.eq_ignore_ascii_case("none") => none += 1,
            [Token::Ident(name)] if matches!(name.to_ascii_lowercase().as_str(), "inside" | "outside") => {
                if position.replace(name.to_ascii_lowercase()).is_some() { return None; }
            }
            [Token::Str(_)] | [Token::Ident(_)] => {
                if style_type.replace(css).is_some() { return None; }
            }
            [Token::Url { .. }] => {
                if image.replace(css).is_some() { return None; }
            }
            [Token::Function(name), Token::OpenParen, .., Token::CloseParen]
                if name.eq_ignore_ascii_case("url") || crate::generated::image_function_name(name) => {
                if image.replace(css).is_some() { return None; }
            }
            _ => return None,
        }
    }
    if none > 2 || none > usize::from(style_type.is_none()) + usize::from(image.is_none()) {
        return None;
    }
    if none > 0 {
        if style_type.is_none() { style_type = Some("none".to_owned()); none -= 1; }
        if none > 0 && image.is_none() { image = Some("none".to_owned()); }
        // A single `none` with no other type/image sets both, as required by
        // the shorthand's marker-suppression meaning.
        if style_type.as_deref() == Some("none") && image.is_none() { image = Some("none".to_owned()); }
    }
    Some(vec![
        ("list-style-type".to_owned(), style_type.unwrap_or_else(|| "disc".to_owned())),
        ("list-style-image".to_owned(), image.unwrap_or_else(|| "none".to_owned())),
        ("list-style-position".to_owned(), position.unwrap_or_else(|| "outside".to_owned())),
    ])
}

const DEFERRED_BACKGROUND: &str = "amazing-internal-background";
const DEFERRED_FONT: &str = "amazing-internal-font";
const DEFERRED_LIST_STYLE: &str = "amazing-internal-list-style";
const DEFERRED_ANIMATION: &str = "amazing-internal-animation";
const DEFERRED_TRANSITION: &str = "amazing-internal-transition";
const DEFERRED_BORDER_WIDTH: &str = "amazing-internal-border-side-width";
const DEFERRED_BORDER_STYLE: &str = "amazing-internal-border-side-style";
const DEFERRED_BORDER_COLOR: &str = "amazing-internal-border-side-color";

const DEFERRED_ALL: &str = "amazing-internal-all";
const DEFERRED_FLEX_DIRECTION: &str = "amazing-internal-flex-flow-direction";
const DEFERRED_FLEX_WRAP: &str = "amazing-internal-flex-flow-wrap";
const DEFERRED_GRID_ROWS: &str = "amazing-internal-grid-template-rows";
const DEFERRED_GRID_COLUMNS: &str = "amazing-internal-grid-template-columns";
const DEFERRED_RADIUS_TOP_LEFT: &str = "amazing-internal-border-radius-top-left";
const DEFERRED_RADIUS_TOP_RIGHT: &str = "amazing-internal-border-radius-top-right";
const DEFERRED_RADIUS_BOTTOM_RIGHT: &str = "amazing-internal-border-radius-bottom-right";
const DEFERRED_RADIUS_BOTTOM_LEFT: &str = "amazing-internal-border-radius-bottom-left";
const DEFERRED_OVERFLOW_X: &str = "amazing-internal-overflow-x";
const DEFERRED_OVERFLOW_Y: &str = "amazing-internal-overflow-y";

fn contains_var_function(value: &str) -> bool {
    tokenize(value)
        .iter()
        .any(|token| matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("var")))
}

fn contains_attr_function(value: &str) -> bool {
    tokenize(value).iter().any(|token| {
        matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("attr"))
    })
}

fn deferred(function: &str, value: &str) -> String {
    format!("{function}({value})")
}

fn deferred_flex_flow(value: &str) -> Vec<(String, String)> {
    vec![
        (
            "flex-direction".to_owned(),
            deferred(DEFERRED_FLEX_DIRECTION, value),
        ),
        ("flex-wrap".to_owned(), deferred(DEFERRED_FLEX_WRAP, value)),
    ]
}

fn deferred_grid_template(value: &str) -> Vec<(String, String)> {
    vec![
        (
            "grid-template-rows".to_owned(),
            deferred(DEFERRED_GRID_ROWS, value),
        ),
        (
            "grid-template-columns".to_owned(),
            deferred(DEFERRED_GRID_COLUMNS, value),
        ),
    ]
}

fn deferred_border_radius(value: &str) -> Vec<(String, String)> {
    [
        ("border-top-left-radius", DEFERRED_RADIUS_TOP_LEFT),
        ("border-top-right-radius", DEFERRED_RADIUS_TOP_RIGHT),
        ("border-bottom-right-radius", DEFERRED_RADIUS_BOTTOM_RIGHT),
        ("border-bottom-left-radius", DEFERRED_RADIUS_BOTTOM_LEFT),
    ]
    .into_iter()
    .map(|(property, function)| (property.to_owned(), deferred(function, value)))
    .collect()
}

fn deferred_overflow(value: &str) -> Vec<(String, String)> {
    vec![
        (
            "overflow-x".to_owned(),
            deferred(DEFERRED_OVERFLOW_X, value),
        ),
        (
            "overflow-y".to_owned(),
            deferred(DEFERRED_OVERFLOW_Y, value),
        ),
    ]
}

/// One optional width, style and color, in any order. Validate the complete
/// shorthand atomically so an invalid component cannot reset other longhands.
fn expand_side_border(property: &str, value: &str) -> Option<Vec<(String, String)>> {
    let components = [
        ("width", DEFERRED_BORDER_WIDTH),
        ("style", DEFERRED_BORDER_STYLE),
        ("color", DEFERRED_BORDER_COLOR),
    ];
    if css_wide_keyword(value).is_some() || contains_var_function(value) {
        return Some(components.into_iter().map(|(component, function)| {
            (format!("{property}-{component}"), if contains_var_function(value) {
                deferred(function, value)
            } else {
                value.to_owned()
            })
        }).collect());
    }
    let tokens = tokenize(value);
    let parts = top_level_components(&tokens)?;
    if !(1..=3).contains(&parts.len()) {
        return None;
    }
    let mut slots: [Option<String>; 3] = [None, None, None];
    for part in parts {
        let text = serialize_computed(part);
        let index = if values::parse("border-top-width", &text).is_some() {
            0
        } else if is_border_style(&text) {
            1
        } else {
            // Preserve known unsupported color spaces just as longhands do,
            // but never interpret an unrelated function as a color component.
            if let Some(Token::Function(name)) = part.first() {
                if !matches!(name.to_ascii_lowercase().as_str(),
                    "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch"
                    | "oklab" | "oklch" | "color" | "color-mix" | "light-dark") {
                    return None;
                }
            }
            values::parse("border-top-color", &text)?;
            2
        };
        if slots[index].replace(text).is_some() {
            return None;
        }
    }
    Some(components.into_iter().zip(slots).zip(["medium", "none", "currentcolor"])
        .map(|(((component, _), value), initial)| {
            (format!("{property}-{component}"), value.unwrap_or_else(|| initial.to_owned()))
        }).collect())
}

fn expand_border_radius(value: &str) -> Option<Vec<(String, String)>> {
    let tokens = tokenize(value);
    let mut slash = None;
    let mut stack = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen | Token::OpenSquare | Token::OpenCurly => stack.push(token),
            Token::CloseParen | Token::CloseSquare | Token::CloseCurly => {
                stack.pop()?;
            }
            Token::Delim('/') if stack.is_empty() && slash.replace(index).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let (horizontal, vertical) = slash.map_or((&tokens[..], None), |slash| {
        (&tokens[..slash], Some(&tokens[slash + 1..]))
    });
    let horizontal = expand_radius_axis(horizontal)?;
    let vertical = match vertical {
        Some(vertical) => expand_radius_axis(vertical)?,
        None => horizontal.clone(),
    };
    let names = [
        "border-top-left-radius",
        "border-top-right-radius",
        "border-bottom-right-radius",
        "border-bottom-left-radius",
    ];
    Some(
        names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                let value = if horizontal[index] == vertical[index] {
                    horizontal[index].clone()
                } else {
                    format!("{} {}", horizontal[index], vertical[index])
                };
                (name.to_owned(), value)
            })
            .collect(),
    )
}

fn expand_radius_axis(tokens: &[Token]) -> Option<[String; 4]> {
    let components = top_level_components(tokens)?;
    if components.is_empty()
        || components.len() > 4
        || !components.iter().all(|component| radius_component(component).is_some())
    {
        return None;
    }
    let values = components.into_iter().map(radius_component).collect::<Option<Vec<_>>>()?;
    Some(match values.as_slice() {
        [one] => [one.clone(), one.clone(), one.clone(), one.clone()],
        [top_left, top_right] => [
            top_left.clone(),
            top_right.clone(),
            top_left.clone(),
            top_right.clone(),
        ],
        [top_left, other, bottom_right] => [
            top_left.clone(),
            other.clone(),
            bottom_right.clone(),
            other.clone(),
        ],
        [top_left, top_right, bottom_right, bottom_left] => [
            top_left.clone(),
            top_right.clone(),
            bottom_right.clone(),
            bottom_left.clone(),
        ],
        _ => return None,
    })
}

fn valid_radius_pair(value: &str) -> bool {
    let tokens = tokenize(value);
    top_level_components(&tokens).is_some_and(|components| {
        (1..=2).contains(&components.len())
            && components
                .iter()
                .all(|component| radius_component(component).is_some())
    })
}

fn radius_component(tokens: &[Token]) -> Option<String> {
    match trim_tokens(tokens) {
        [Token::Number(value)] if value.is_finite() && *value == 0.0 => Some("0".into()),
        [Token::Percentage(value)] if value.is_finite() && *value >= 0.0 => Some(serialize(tokens)),
        [Token::Dimension(value, unit)] => {
            (value.is_finite() && *value >= 0.0 && unit.eq_ignore_ascii_case("px"))
                .then(|| serialize(tokens))
        }
        _ => {
            let text = serialize(tokens);
            if crate::math::length_percentage(&text, true) {
                if let Some(inside) = text.trim().strip_prefix("calc(").and_then(|s| s.strip_suffix(')')) {
                    if let Some(pixels) = inside.strip_suffix("px").and_then(|s| s.trim().parse::<f32>().ok()) {
                        if pixels.is_finite() { return Some(format!("{}px", pixels.max(0.0))); }
                    }
                }
                return Some(text);
            }
            None
        },
    }
}

fn valid_background_clip(value: &str) -> bool {
    matches!(
        trim_tokens(&tokenize(value)),
        [Token::Ident(keyword)] if matches!(
            keyword.to_ascii_lowercase().as_str(),
            "border-box" | "padding-box" | "content-box"
        )
    )
}

fn valid_box_shadow(value: &str) -> bool {
    if value.trim().eq_ignore_ascii_case("none") {
        return true;
    }
    let tokens = tokenize(value);
    if tokens.iter().any(|token| matches!(token, Token::Comma)) {
        return false;
    }
    let Some(components) = top_level_components(&tokens) else {
        return false;
    };
    let mut inset = false;
    let mut color = false;
    let mut lengths = Vec::new();
    for component in components {
        match trim_tokens(component) {
            [Token::Ident(keyword)] if keyword.eq_ignore_ascii_case("inset") => {
                if inset {
                    return false;
                }
                inset = true;
            }
            [Token::Ident(keyword)] if valid_supported_color_name(keyword) => {
                if color {
                    return false;
                }
                color = true;
            }
            [Token::Hash(hex)] if valid_hex_color(hex) => {
                if color {
                    return false;
                }
                color = true;
            }
            component
                if function_inner(component, "rgb").is_some()
                    || function_inner(component, "rgba").is_some() =>
            {
                let inner =
                    function_inner(component, "rgb").or_else(|| function_inner(component, "rgba"));
                if color || !inner.is_some_and(valid_rgb_color_tokens) {
                    return false;
                }
                color = true;
            }
            component => {
                let Some(length) = shadow_length(component) else {
                    return false;
                };
                lengths.push(length);
            }
        }
    }
    (2..=4).contains(&lengths.len()) && lengths.get(2).is_none_or(|blur| *blur >= 0.0)
}

fn valid_supported_color_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "currentcolor"
            | "aliceblue"
            | "antiquewhite"
            | "aqua"
            | "aquamarine"
            | "azure"
            | "beige"
            | "black"
            | "blue"
            | "brown"
            | "chocolate"
            | "coral"
            | "crimson"
            | "cyan"
            | "darkgray"
            | "darkgrey"
            | "dimgray"
            | "dimgrey"
            | "fuchsia"
            | "gold"
            | "goldenrod"
            | "gray"
            | "green"
            | "greenyellow"
            | "grey"
            | "honeydew"
            | "hotpink"
            | "indigo"
            | "ivory"
            | "khaki"
            | "lavender"
            | "lime"
            | "magenta"
            | "maroon"
            | "navy"
            | "olive"
            | "orange"
            | "orchid"
            | "pink"
            | "plum"
            | "purple"
            | "rebeccapurple"
            | "red"
            | "salmon"
            | "seagreen"
            | "silver"
            | "skyblue"
            | "slategray"
            | "snow"
            | "steelblue"
            | "tan"
            | "teal"
            | "thistle"
            | "tomato"
            | "transparent"
            | "turquoise"
            | "violet"
            | "wheat"
            | "white"
            | "whitesmoke"
            | "yellow"
    )
}

fn valid_hex_color(hex: &str) -> bool {
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_rgb_color_tokens(tokens: &[Token]) -> bool {
    let value = serialize(tokens);
    if value.contains(',') {
        let parts = value.split(',').map(str::trim).collect::<Vec<_>>();
        matches!(parts.len(), 3 | 4)
            && parts[..3].iter().all(|part| valid_rgb_channel(part))
            && parts.get(3).is_none_or(|alpha| valid_alpha_channel(alpha))
    } else {
        let normalized = value.replace('/', " / ");
        let parts = normalized.split_ascii_whitespace().collect::<Vec<_>>();
        match parts.as_slice() {
            [red, green, blue] => [red, green, blue]
                .into_iter()
                .all(|part| valid_rgb_channel(part)),
            [red, green, blue, slash, alpha] if *slash == "/" => {
                [red, green, blue]
                    .into_iter()
                    .all(|part| valid_rgb_channel(part))
                    && valid_alpha_channel(alpha)
            }
            _ => false,
        }
    }
}

fn valid_rgb_channel(value: &str) -> bool {
    value
        .strip_suffix('%')
        .unwrap_or(value)
        .parse::<f32>()
        .is_ok_and(|value| value.is_finite())
}

fn valid_alpha_channel(value: &str) -> bool {
    valid_rgb_channel(value)
}

fn valid_opacity(value: &str) -> bool {
    matches!(
        trim_tokens(&tokenize(value)),
        [Token::Number(value)] | [Token::Percentage(value)] if value.is_finite()
    )
}

fn valid_overflow(value: &str) -> bool {
    matches!(
        trim_tokens(&tokenize(value)),
        [Token::Ident(keyword)] if matches!(
            keyword.to_ascii_lowercase().as_str(),
            "visible" | "hidden" | "clip" | "scroll" | "auto"
        )
    )
}

fn shadow_length(tokens: &[Token]) -> Option<f64> {
    match trim_tokens(tokens) {
        [Token::Number(value)] if *value == 0.0 => Some(*value),
        [Token::Dimension(value, unit)] if unit.eq_ignore_ascii_case("px") => Some(*value),
        _ => None,
    }
}

fn valid_transform(value: &str) -> bool {
    if value.trim().eq_ignore_ascii_case("none") {
        return true;
    }
    let tokens = tokenize(value);
    let Some(components) = top_level_components(&tokens) else {
        return false;
    };
    !components.is_empty() && components.into_iter().all(valid_transform_function)
}

fn valid_transform_function(tokens: &[Token]) -> bool {
    let tokens = trim_tokens(tokens);
    let Some(Token::Function(name)) = tokens.first() else {
        return false;
    };
    let Some(inner) = function_inner(tokens, name) else {
        return false;
    };
    let Some(arguments) = transform_arguments(inner) else {
        return false;
    };
    match name.to_ascii_lowercase().as_str() {
        "translate" => {
            (1..=2).contains(&arguments.len())
                && arguments.iter().all(|arg| valid_transform_length(arg))
        }
        "translatex" | "translatey" => arguments.len() == 1 && valid_transform_length(arguments[0]),
        "scale" => {
            (1..=2).contains(&arguments.len())
                && arguments.iter().all(|arg| valid_transform_number(arg))
        }
        "scalex" | "scaley" => arguments.len() == 1 && valid_transform_number(arguments[0]),
        "rotate" | "skewx" | "skewy" => arguments.len() == 1 && valid_transform_angle(arguments[0]),
        "matrix" => arguments.len() == 6 && arguments.iter().all(|arg| valid_transform_number(arg)),
        _ => false,
    }
}

pub(crate) fn transform_arguments(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let tokens = trim_tokens(tokens);
    if tokens.is_empty() {
        return None;
    }
    if tokens.iter().any(|token| matches!(token, Token::Comma)) {
        let mut output = Vec::new();
        let mut start = 0usize;
        for (index, token) in tokens.iter().enumerate() {
            if matches!(token, Token::Comma) {
                let argument = trim_tokens(&tokens[start..index]);
                if argument.is_empty() {
                    return None;
                }
                output.push(argument);
                start = index + 1;
            }
        }
        let argument = trim_tokens(&tokens[start..]);
        if argument.is_empty() {
            return None;
        }
        output.push(argument);
        Some(output)
    } else {
        top_level_components(tokens)
    }
}

fn valid_transform_number(tokens: &[Token]) -> bool {
    matches!(trim_tokens(tokens), [Token::Number(value)] if value.is_finite())
}

fn valid_transform_length(tokens: &[Token]) -> bool {
    crate::values::LengthPercentage::parse(&serialize(trim_tokens(tokens))).is_some()
}

fn valid_transform_angle(tokens: &[Token]) -> bool {
    matches!(trim_tokens(tokens), [Token::Number(value)] if *value == 0.0)
        || matches!(
            trim_tokens(tokens),
            [Token::Dimension(value, unit)] if value.is_finite() && matches!(
                unit.to_ascii_lowercase().as_str(),
                "deg" | "grad" | "rad" | "turn"
            )
        )
}

fn expand_flex_flow(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() {
        return Some(vec![
            ("flex-direction".to_owned(), value.trim().to_owned()),
            ("flex-wrap".to_owned(), value.trim().to_owned()),
        ]);
    }
    let tokens = tokenize(value);
    let words: Vec<&str> = tokens
        .iter()
        .filter_map(|token| match token {
            Token::Whitespace => None,
            Token::Ident(word) => Some(word.as_str()),
            _ => Some(""),
        })
        .collect();
    if words.is_empty() || words.len() > 2 || words.iter().any(|word| word.is_empty()) {
        return None;
    }

    let mut direction = None;
    let mut wrap = None;
    for word in words {
        let lower = word.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "row" | "row-reverse" | "column" | "column-reverse"
        ) {
            if direction.replace(lower).is_some() {
                return None;
            }
        } else if matches!(lower.as_str(), "nowrap" | "wrap" | "wrap-reverse") {
            if wrap.replace(lower).is_some() {
                return None;
            }
        } else {
            return None;
        }
    }

    Some(vec![
        (
            "flex-direction".to_owned(),
            direction.unwrap_or_else(|| "row".into()),
        ),
        (
            "flex-wrap".to_owned(),
            wrap.unwrap_or_else(|| "nowrap".into()),
        ),
    ])
}

fn expand_grid_template(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() || value.trim().eq_ignore_ascii_case("none") {
        return Some(vec![
            ("grid-template-rows".to_owned(), value.trim().to_owned()),
            ("grid-template-columns".to_owned(), value.trim().to_owned()),
        ]);
    }

    let tokens = tokenize(value);
    let mut stack = Vec::new();
    let mut slash = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Delim('/') if stack.is_empty() && slash.replace(index).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let slash = slash?;
    let rows = serialize(&tokens[..slash]).trim().to_owned();
    let columns = serialize(&tokens[slash + 1..]).trim().to_owned();
    if !valid_grid_track_list(&rows) || !valid_grid_track_list(&columns) {
        return None;
    }
    Some(vec![
        ("grid-template-rows".to_owned(), rows),
        ("grid-template-columns".to_owned(), columns),
    ])
}

pub(crate) fn top_level_components(tokens: &[Token]) -> Option<Vec<&[Token]>> {
    let mut components = Vec::new();
    let mut stack = Vec::new();
    let mut start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Whitespace if stack.is_empty() => {
                let component = trim_tokens(&tokens[start..index]);
                if !component.is_empty() {
                    components.push(component);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let component = trim_tokens(&tokens[start..]);
    if !component.is_empty() {
        components.push(component);
    }
    Some(components)
}

fn split_top_level_comma(tokens: &[Token]) -> Option<(&[Token], &[Token])> {
    let mut stack = Vec::new();
    let mut comma = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::OpenParen => stack.push(')'),
            Token::OpenSquare => stack.push(']'),
            Token::OpenCurly => stack.push('}'),
            Token::CloseParen if stack.pop() != Some(')') => return None,
            Token::CloseSquare if stack.pop() != Some(']') => return None,
            Token::CloseCurly if stack.pop() != Some('}') => return None,
            Token::Comma if stack.is_empty() && comma.replace(index).is_some() => {
                return None;
            }
            _ => {}
        }
    }
    if !stack.is_empty() {
        return None;
    }
    let comma = comma?;
    Some((
        trim_tokens(&tokens[..comma]),
        trim_tokens(&tokens[comma + 1..]),
    ))
}

pub(crate) fn function_inner<'a>(tokens: &'a [Token], expected: &str) -> Option<&'a [Token]> {
    let tokens = trim_tokens(tokens);
    match (tokens.first(), tokens.get(1), tokens.last()) {
        (Some(Token::Function(name)), Some(Token::OpenParen), Some(Token::CloseParen))
            if name.eq_ignore_ascii_case(expected) =>
        {
            Some(&tokens[2..tokens.len() - 1])
        }
        _ => None,
    }
}

fn valid_grid_track_list(value: &str) -> bool {
    let tokens = tokenize(value);
    let tokens = trim_tokens(&tokens);
    if matches!(tokens, [Token::Ident(value)] if value.eq_ignore_ascii_case("none")) {
        return true;
    }
    if matches!(tokens.first(), Some(Token::Ident(name)) if name.eq_ignore_ascii_case("subgrid")) {
        return parse_subgrid(value).is_some();
    }
    valid_grid_track_tokens(tokens, true)
}

/// Grid Level 2 §7.2: `subgrid <line-name-list>?`. Track sizes cannot
/// participate in this branch of the grid-template grammar.
pub(crate) fn parse_subgrid(value: &str) -> Option<values::Subgrid> {
    use values::{Subgrid, SubgridLineNames, SubgridRepeatCount};
    let tokens = tokenize(value);
    let mut position = 0;
    skip_css_whitespace(&tokens, &mut position);
    match tokens.get(position)? {
        Token::Ident(name) if name.eq_ignore_ascii_case("subgrid") => position += 1,
        _ => return None,
    }
    let mut line_names = Vec::new();
    let mut auto_fill_seen = false;
    loop {
        skip_css_whitespace(&tokens, &mut position);
        let Some(token) = tokens.get(position) else { break };
        if matches!(token, Token::OpenSquare) {
            let (names, end) = parse_subgrid_names(&tokens, position)?;
            line_names.push(SubgridLineNames::Names(names));
            position = end;
            continue;
        }
        if matches!(token, Token::Function(name) if name.eq_ignore_ascii_case("repeat")) {
            let start = position;
            let mut depth = 0usize;
            loop {
                match tokens.get(position)? {
                    Token::OpenParen => depth += 1,
                    Token::CloseParen => {
                        depth = depth.checked_sub(1)?;
                        if depth == 0 { position += 1; break; }
                    }
                    _ => {}
                }
                position += 1;
            }
            let inner = function_inner(&tokens[start..position], "repeat")?;
            let (count, names) = split_top_level_comma(inner)?;
            let count = match trim_tokens(count) {
                [Token::Number(n)] if n.is_finite() && *n >= 1.0 && n.fract() == 0.0 && *n <= u32::MAX as f64 => SubgridRepeatCount::Integer(*n as u32),
                [Token::Ident(name)] if name.eq_ignore_ascii_case("auto-fill") && !auto_fill_seen => {
                    auto_fill_seen = true;
                    SubgridRepeatCount::AutoFill
                }
                _ => return None,
            };
            let mut names_position = 0;
            let mut groups = Vec::new();
            while names_position < names.len() {
                skip_css_whitespace(names, &mut names_position);
                if names_position == names.len() { break; }
                let (group, end) = parse_subgrid_names(names, names_position)?;
                groups.push(group);
                names_position = end;
            }
            if groups.is_empty() { return None; }
            line_names.push(SubgridLineNames::Repeat { count, names: groups });
            continue;
        }
        return None;
    }
    Some(Subgrid { line_names })
}

fn parse_subgrid_names(tokens: &[Token], start: usize) -> Option<(Vec<String>, usize)> {
    if !matches!(tokens.get(start), Some(Token::OpenSquare)) { return None; }
    let end = tokens[start + 1..].iter().position(|token| matches!(token, Token::CloseSquare))? + start + 1;
    let mut names = Vec::new();
    for token in &tokens[start + 1..end] {
        match token {
            Token::Whitespace => {}
            Token::Ident(name) if valid_grid_line_name(name) => names.push(name.clone()),
            _ => return None,
        }
    }
    Some((names, end + 1))
}

fn valid_grid_track_tokens(tokens: &[Token], allow_repeat: bool) -> bool {
    let Some(components) = top_level_components(tokens) else {
        return false;
    };
    if components.is_empty() {
        return false;
    }
    let mut auto_repeat_seen = false;
    for component in components {
        if function_inner(component, "repeat").is_some() {
            if !allow_repeat {
                return false;
            }
            let Some((auto_repeat, tracks)) = valid_repeat(component) else {
                return false;
            };
            if auto_repeat {
                if auto_repeat_seen || !valid_fixed_track_tokens(tracks) {
                    return false;
                }
                auto_repeat_seen = true;
            } else if !valid_grid_track_tokens(tracks, false) {
                return false;
            }
        } else if !valid_grid_track_component(component) {
            return false;
        }
    }
    true
}

fn valid_grid_track_component(tokens: &[Token]) -> bool {
    let tokens = trim_tokens(tokens);
    if valid_grid_breadth(tokens, true) {
        return true;
    }
    if matches!(
        (tokens.first(), tokens.last()),
        (Some(Token::OpenSquare), Some(Token::CloseSquare))
    ) {
        // One or more adjacent `[name]` blocks (`[a][b]` is valid without
        // intervening whitespace); each block validates independently.
        let mut rest: &[Token] = tokens;
        loop {
            let Some((first, tail)) = rest.split_first() else {
                return false;
            };
            if !matches!(first, Token::OpenSquare) {
                return false;
            }
            let mut depth = 1usize;
            let mut close = None;
            for (index, token) in tail.iter().enumerate() {
                match token {
                    Token::OpenSquare => depth += 1,
                    Token::CloseSquare => {
                        depth -= 1;
                        if depth == 0 {
                            close = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(close) = close else {
                return false;
            };
            let inner = trim_tokens(&tail[..close]);
            if inner.is_empty() {
                return false;
            }
            if !inner.iter().all(|token| match token {
                Token::Ident(name) => valid_grid_line_name(name),
                Token::Whitespace => true,
                _ => false,
            }) {
                return false;
            }
            rest = trim_tokens(&tail[close + 1..]);
            if rest.is_empty() {
                return true;
            }
        }
    }
    if let Some(inner) = function_inner(tokens, "minmax") {
        return split_top_level_comma(inner).is_some_and(|(minimum, maximum)| {
            valid_grid_breadth(minimum, false) && valid_grid_breadth(maximum, true)
        });
    }
    false
}

fn valid_grid_line_name(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "auto" | "span" | "default" | "initial" | "inherit" | "unset" | "revert" | "revert-layer"
    )
}

fn grid_line_custom_ident(value: &str) -> bool {
    matches!(trim_tokens(&tokenize(value)), [Token::Ident(name)] if valid_grid_line_name(name))
}

fn valid_grid_line(value: &str) -> bool {
    // A variable can supply the complete grid-line grammar after substitution.
    if contains_var_function(value) {
        return true;
    }
    let tokens = tokenize(value);
    let components: Vec<&Token> = tokens.iter().filter(|token| !matches!(token, Token::Whitespace)).collect();
    if matches!(components.as_slice(), [Token::Ident(name)] if name.eq_ignore_ascii_case("auto")) {
        return true;
    }
    let mut span = false;
    let mut integer = None;
    let mut name = false;
    for token in &components {
        match token {
            Token::Ident(ident) if ident.eq_ignore_ascii_case("span") && !span => span = true,
            Token::Ident(ident) if valid_grid_line_name(ident) && !name => name = true,
            Token::Number(number) if number.is_finite() && number.fract() == 0.0 && integer.is_none() => integer = Some(*number),
            _ => return false,
        }
    }
    if span {
        (integer.is_some_and(|number| number > 0.0) || name)
            && integer.is_none_or(|number| number > 0.0)
    } else {
        (integer.is_some() || name) && integer != Some(0.0)
    }
}

/// Returns whether this is an auto-repeat plus its track-list body.
fn valid_repeat(tokens: &[Token]) -> Option<(bool, &[Token])> {
    let inner = function_inner(tokens, "repeat")?;
    let (count, tracks) = split_top_level_comma(inner)?;
    let auto_repeat = match count {
        [Token::Number(value)] if value.is_finite() && *value >= 1.0 && value.fract() == 0.0 => {
            false
        }
        [Token::Ident(value)]
            if value.eq_ignore_ascii_case("auto-fill")
                || value.eq_ignore_ascii_case("auto-fit") =>
        {
            true
        }
        _ => return None,
    };
    Some((auto_repeat, tracks))
}

fn valid_fixed_track_tokens(tokens: &[Token]) -> bool {
    let Some(components) = top_level_components(tokens) else {
        return false;
    };
    let mut saw_track = false;
    for component in components {
        let component = trim_tokens(component);
        if matches!(
            (component.first(), component.last()),
            (Some(Token::OpenSquare), Some(Token::CloseSquare))
        ) {
            if !valid_grid_track_component(component) {
                return false;
            }
            continue;
        }
        saw_track = true;
        if !valid_fixed_size(component) {
            return false;
        }
    }
    saw_track
}

fn valid_fixed_size(tokens: &[Token]) -> bool {
    if valid_fixed_breadth(tokens) {
        return true;
    }
    function_inner(tokens, "minmax")
        .and_then(split_top_level_comma)
        .is_some_and(|(minimum, maximum)| {
            (valid_fixed_breadth(minimum) && valid_grid_breadth(maximum, true))
                || (valid_grid_breadth(minimum, false) && valid_fixed_breadth(maximum))
        })
}

fn valid_fixed_breadth(tokens: &[Token]) -> bool {
    match trim_tokens(tokens) {
        [Token::Number(value)] => value.is_finite() && *value == 0.0,
        [Token::Percentage(value)] => value.is_finite() && *value >= 0.0,
        [Token::Dimension(value, unit)] => {
            value.is_finite()
                && *value >= 0.0
                && matches!(
                    unit.to_ascii_lowercase().as_str(),
                    "px" | "em"
                        | "rem"
                        | "ex"
                        | "ch"
                        | "vw"
                        | "vh"
                        | "vmin"
                        | "vmax"
                        | "cm"
                        | "mm"
                        | "q"
                        | "in"
                        | "pc"
                        | "pt"
                )
        }
        _ => false,
    }
}

fn valid_grid_breadth(tokens: &[Token], allow_fr: bool) -> bool {
    match trim_tokens(tokens) {
        [Token::Ident(value)] => matches!(
            value.to_ascii_lowercase().as_str(),
            "auto" | "min-content" | "max-content"
        ),
        [Token::Number(value)] => value.is_finite() && *value == 0.0,
        [Token::Percentage(value)] => value.is_finite() && *value >= 0.0,
        [Token::Dimension(value, unit)] => {
            value.is_finite()
                && *value >= 0.0
                && match unit.to_ascii_lowercase().as_str() {
                    "fr" => allow_fr,
                    "px" | "em" | "rem" | "ex" | "ch" | "vw" | "vh" | "vmin" | "vmax" | "cm"
                    | "mm" | "q" | "in" | "pc" | "pt" => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Non-negative `<number>` (the `flex-grow`/`flex-shrink` grammar),
/// including the special calc() forms the layout solver evaluates.
fn valid_flex_number(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("calc(infinity)")
        || trimmed.eq_ignore_ascii_case("calc(-infinity)")
        || trimmed.eq_ignore_ascii_case("calc(nan)")
    {
        return true;
    }
    matches!(tokenize(trimmed).as_slice(), [Token::Number(n)] if n.is_finite() && *n >= 0.0)
}

/// `<'flex-basis'>` within the supported subset: `auto` or a fixed breadth.
fn valid_flex_basis(value: &str) -> bool {
    let owned = tokenize(value);
    let tokens = trim_tokens(&owned);
    matches!(tokens, [Token::Ident(name)] if name.eq_ignore_ascii_case("auto"))
        || valid_fixed_breadth(tokens)
}

/// Legacy gap aliases accept one non-negative length-percentage per side
/// (`row-gap`/`column-gap` also allow the `normal` keyword).
fn valid_gap_value(value: &str) -> bool {
    let owned = tokenize(value);
    let tokens = trim_tokens(&owned);
    matches!(tokens, [Token::Ident(name)] if name.eq_ignore_ascii_case("normal"))
        || valid_fixed_breadth(tokens)
}

/// `flex` shorthand per §7.1.10: `none`, `auto`, or up to three components
/// `<'flex-grow'> <'flex-shrink'>? <'flex-basis'>?`, where an omitted basis is
/// `0%` after a second number and `auto` otherwise. Invalid combinations
/// return `None` so the declaration emits no cascade candidates.
fn expand_flex_shorthand(parts: &[&str]) -> Option<(String, String, String)> {
    let text = |component: &str| component.to_owned();
    let (grow, shrink, basis) = match parts {
        ["none"] => ("0".to_owned(), "0".to_owned(), "auto".to_owned()),
        ["auto"] => ("1".to_owned(), "1".to_owned(), "auto".to_owned()),
        [grow] if valid_flex_number(grow) => (text(grow), "1".to_owned(), "0%".to_owned()),
        [basis] if valid_flex_basis(basis) => ("1".to_owned(), "1".to_owned(), text(basis)),
        [grow, shrink] if valid_flex_number(grow) && valid_flex_number(shrink) => {
            (text(grow), text(shrink), "0%".to_owned())
        }
        [grow, basis] if valid_flex_number(grow) && valid_flex_basis(basis) => {
            (text(grow), "1".to_owned(), text(basis))
        }
        [grow, shrink, basis]
            if valid_flex_number(grow) && valid_flex_number(shrink) && valid_flex_basis(basis) =>
        {
            (text(grow), text(shrink), text(basis))
        }
        // §7.1.10's || combinator also permits the basis first.
        [basis, grow] if valid_flex_basis(basis) && valid_flex_number(grow) => {
            (text(grow), "1".to_owned(), text(basis))
        }
        [basis, grow, shrink]
            if valid_flex_basis(basis) && valid_flex_number(grow) && valid_flex_number(shrink) =>
        {
            (text(grow), text(shrink), text(basis))
        }
        _ => return None,
    };
    Some((grow, shrink, basis))
}

const FONT_LONGHANDS: &[&str] = &[
    "font-family", "font-feature-settings", "font-size", "font-size-adjust", "font-stretch",
    "font-style", "font-variant", "font-variant-ligatures", "font-weight", "font-kerning",
    "font-language-override", "font-optical-sizing", "font-variant-alternates", "font-variant-caps",
    "font-variant-east-asian", "font-variant-emoji", "font-variant-numeric",
    "font-variant-position", "font-variation-settings",
    "line-height",
];

fn expand_font_shorthand(value: &str) -> Option<Vec<(String, String)>> {
    if css_wide_keyword(value).is_some() {
        return Some(
            FONT_LONGHANDS.iter().copied()
            .map(|property| (property.to_owned(), value.to_owned()))
            .collect(),
        );
    }
    let tokens = tokenize(value);
    let mut position = 0usize;
    let mut style = "normal".to_owned();
    let mut variant = "normal".to_owned();
    let mut weight = "normal".to_owned();
    let mut stretch = "normal".to_owned();
    let mut style_seen = false;
    let mut variant_seen = false;
    let mut weight_seen = false;
    let mut stretch_seen = false;
    let mut optional_count = 0usize;
    let size = loop {
        skip_css_whitespace(&tokens, &mut position);
        let token = tokens.get(position)?;
        if is_font_size_token(token) {
            let size = serialize(std::slice::from_ref(token));
            position += 1;
            break size;
        }
        optional_count += 1;
        if optional_count > 4 {
            return None;
        }
        match token {
            Token::Ident(value) if value.eq_ignore_ascii_case("normal") => {}
            Token::Ident(value)
                if value.eq_ignore_ascii_case("italic")
                    || value.eq_ignore_ascii_case("oblique") =>
            {
                if style_seen {
                    return None;
                }
                style_seen = true;
                style = value.clone();
            }
            Token::Ident(value) if value.eq_ignore_ascii_case("small-caps") => {
                if variant_seen {
                    return None;
                }
                variant_seen = true;
                variant = value.clone();
            }
            Token::Ident(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "bold" | "bolder" | "lighter"
                ) =>
            {
                if weight_seen {
                    return None;
                }
                weight_seen = true;
                weight = value.clone();
            }
            Token::Number(value) if (1.0..=1000.0).contains(value) && value.fract() == 0.0 => {
                if weight_seen {
                    return None;
                }
                weight_seen = true;
                weight = serialize(std::slice::from_ref(token));
            }
            Token::Ident(value)
                if value.to_ascii_lowercase().ends_with("condensed")
                    || value.to_ascii_lowercase().ends_with("expanded") =>
            {
                if stretch_seen {
                    return None;
                }
                stretch_seen = true;
                stretch = value.clone();
            }
            _ => return None,
        }
        position += 1;
    };
    skip_css_whitespace(&tokens, &mut position);
    let line_height = if tokens.get(position) == Some(&Token::Delim('/')) {
        position += 1;
        skip_css_whitespace(&tokens, &mut position);
        let token = tokens.get(position)?;
        if !is_line_height_token(token) {
            return None;
        }
        position += 1;
        serialize(std::slice::from_ref(token))
    } else {
        "normal".to_owned()
    };
    skip_css_whitespace(&tokens, &mut position);
    let family_tokens = tokens.get(position..)?;
    if !valid_font_family_tokens(family_tokens) {
        return None;
    }
    let family = serialize(family_tokens);
    Some(vec![
        ("font-style".into(), style),
        ("font-variant".into(), variant),
        ("font-variant-caps".into(), if variant_seen { "small-caps" } else { "normal" }.into()),
        ("font-variant-alternates".into(), "normal".into()),
        ("font-variant-east-asian".into(), "normal".into()),
        ("font-variant-emoji".into(), "normal".into()),
        ("font-variant-numeric".into(), "normal".into()),
        ("font-variant-position".into(), "normal".into()),
        ("font-variant-ligatures".into(), "normal".into()),
        ("font-feature-settings".into(), "normal".into()),
        ("font-kerning".into(), "auto".into()),
        ("font-language-override".into(), "normal".into()),
        ("font-optical-sizing".into(), "auto".into()),
        ("font-variation-settings".into(), "normal".into()),
        ("font-weight".into(), weight),
        ("font-stretch".into(), stretch),
        ("font-size-adjust".into(), "none".into()),
        ("font-size".into(), size),
        ("line-height".into(), line_height),
        ("font-family".into(), family),
    ])
}

/// Validate the two Fonts 4 feature grammars before a declaration enters the
/// cascade, and serialize their computed values for public style consumers.
pub(crate) fn canonical_font_feature_value(property: &str, value: &str) -> Option<String> {
    let tokens = tokenize(value);
    let significant: Vec<&Token> = tokens.iter().filter(|t| !matches!(t, Token::Whitespace)).collect();
    if significant.len() == 1 {
        if let Token::Ident(name) = significant[0] {
            if name.eq_ignore_ascii_case("normal") ||
                (property == "font-variant-ligatures" && name.eq_ignore_ascii_case("none")) {
                return Some(name.to_ascii_lowercase());
            }
        }
    }
    if property == "font-variant-ligatures" {
        let mut seen = [false; 4];
        let mut result = Vec::new();
        for token in significant {
            let Token::Ident(name) = token else { return None };
            let name = name.to_ascii_lowercase();
            let category = match name.as_str() {
                "common-ligatures" | "no-common-ligatures" => 0,
                "discretionary-ligatures" | "no-discretionary-ligatures" => 1,
                "historical-ligatures" | "no-historical-ligatures" => 2,
                "contextual" | "no-contextual" => 3,
                _ => return None,
            };
            if seen[category] { return None; }
            seen[category] = true;
            result.push(name);
        }
        return (!result.is_empty()).then(|| result.join(" "));
    }
    if property != "font-feature-settings" { return None; }
    let mut result = Vec::new();
    let mut position = 0;
    while position < significant.len() {
        let Token::Str(tag) = significant[position] else { return None };
        if tag.len() != 4 || !tag.bytes().all(|c| (0x20..=0x7e).contains(&c)) {
            return None;
        }
        position += 1;
        let mut item = format!("\"{}\"", tag.replace('\\', "\\\\").replace('"', "\\\""));
        if let Some(token) = significant.get(position) {
            match token {
                Token::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => {
                    item.push_str(&format!(" {n}"));
                    position += 1;
                }
                Token::Ident(keyword) if keyword.eq_ignore_ascii_case("on") || keyword.eq_ignore_ascii_case("off") => {
                    item.push_str(if keyword.eq_ignore_ascii_case("on") { " on" } else { " off" });
                    position += 1;
                }
                _ => {}
            }
        }
        result.push(item);
        if position == significant.len() { break; }
        if !matches!(significant[position], Token::Comma) { return None; }
        position += 1;
        if position == significant.len() { return None; }
    }
    (!result.is_empty()).then(|| result.join(", "))
}

fn skip_css_whitespace(tokens: &[Token], position: &mut usize) {
    while matches!(tokens.get(*position), Some(Token::Whitespace)) {
        *position += 1;
    }
}

fn is_font_size_token(token: &Token) -> bool {
    matches!(token, Token::Dimension(value, _) | Token::Percentage(value) if *value >= 0.0)
        || matches!(token, Token::Number(value) if *value == 0.0)
        || matches!(token, Token::Ident(value) if matches!(
            value.to_ascii_lowercase().as_str(),
            "xx-small" | "x-small" | "small" | "medium" | "large" | "x-large" | "xx-large" | "xxx-large" | "smaller" | "larger"
        ))
}

fn is_line_height_token(token: &Token) -> bool {
    matches!(token, Token::Dimension(value, _) | Token::Percentage(value) | Token::Number(value) if *value >= 0.0)
        || matches!(token, Token::Ident(value) if value.eq_ignore_ascii_case("normal"))
}

fn valid_font_family_tokens(tokens: &[Token]) -> bool {
    #[derive(Clone, Copy)]
    enum FamilyKind {
        None,
        String,
        Ident,
    }
    let mut kind = FamilyKind::None;
    for token in tokens {
        match token {
            Token::Whitespace => {}
            Token::Comma if !matches!(kind, FamilyKind::None) => {
                kind = FamilyKind::None;
            }
            Token::Ident(_) if !matches!(kind, FamilyKind::String) => kind = FamilyKind::Ident,
            Token::Str(_) if matches!(kind, FamilyKind::None) => kind = FamilyKind::String,
            _ => return false,
        }
    }
    !matches!(kind, FamilyKind::None)
}

fn replicate_index(count: usize, side: usize) -> usize {
    match count {
        0 | 1 => 0,
        2 => side % 2,
        3 if side == 3 => 1,
        3 => side.min(2),
        _ => side.min(3),
    }
}

fn longhand_name(shorthand: &str, side: &str) -> String {
    match shorthand.rsplit_once('-') {
        Some((head, tail)) => format!("{head}-{side}-{tail}"),
        None => format!("{shorthand}-{side}"),
    }
}

fn is_border_style(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "none"
            | "hidden"
            | "dotted"
            | "dashed"
            | "solid"
            | "double"
            | "groove"
            | "ridge"
            | "inset"
            | "outset"
    )
}

fn is_border_width(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "thin" | "medium" | "thick"
    ) || matches!(
        value.chars().next(),
        Some('0'..='9') | Some('.') | Some('-') | Some('+')
    )
}

#[derive(Clone, Copy)]
enum CssWide {
    Initial,
    Inherit,
    Unset,
    Revert,
    RevertLayer,
}

fn css_wide_keyword(value: &str) -> Option<CssWide> {
    match value.trim().to_ascii_lowercase().as_str() {
        "initial" => Some(CssWide::Initial),
        "inherit" => Some(CssWide::Inherit),
        "unset" => Some(CssWide::Unset),
        "revert" => Some(CssWide::Revert),
        "revert-layer" => Some(CssWide::RevertLayer),
        _ => None,
    }
}

/// Turns one cascaded winner per property into computed values.
#[cfg(test)]
pub(crate) fn resolve(
    parent: Option<&ComputedStyle>,
    cascaded: BTreeMap<String, String>,
) -> ComputedStyle {
    resolve_with_root_attrs_environment(parent, cascaded, 16.0, None, &MediaEnvironment::default())
}

#[cfg(test)]
pub(crate) fn resolve_with_root_and_attrs(
    parent: Option<&ComputedStyle>,
    cascaded: BTreeMap<String, String>,
    root_font_px: f32,
    attrs: Option<&[(String, String)]>,
) -> ComputedStyle {
    resolve_with_root_attrs_environment(parent, cascaded, root_font_px, attrs, &MediaEnvironment::default())
}

pub(crate) fn resolve_with_root_attrs_environment(
    parent: Option<&ComputedStyle>,
    mut cascaded: BTreeMap<String, String>,
    root_font_px: f32,
    attrs: Option<&[(String, String)]>,
    environment: &MediaEnvironment,
) -> ComputedStyle {
    let mut inherited_custom = parent.map(|style| style.custom.clone()).unwrap_or_default();
    let custom_names: Vec<String> = cascaded
        .keys()
        .filter(|name| name.starts_with("--"))
        .cloned()
        .collect();
    let mut local_raw = BTreeMap::new();
    for name in custom_names {
        let value = cascaded.remove(&name).expect("collected custom property");
        match css_wide_keyword(&value) {
            Some(CssWide::Initial) => {
                inherited_custom.insert(name, CustomValue::Invalid);
            }
            Some(CssWide::Inherit | CssWide::Unset) => {
                let inherited = parent
                    .and_then(|style| style.custom.get(&name))
                    .cloned()
                    .unwrap_or(CustomValue::Invalid);
                inherited_custom.insert(name, inherited);
            }
            _ => {
                local_raw.insert(name, tokenize(&value));
            }
        }
    }
    let cycles = cycle_members(&local_raw);
    let mut resolver = CustomResolver {
        local: &local_raw,
        inherited: &inherited_custom,
        cycles: &cycles,
        memo: BTreeMap::new(),
        resolving: BTreeSet::new(),
    };
    let mut resolved_local = Vec::with_capacity(local_raw.len());
    for name in local_raw.keys() {
        let value = resolver.resolve_name(name);
        resolved_local.push((name.clone(), value));
    }

    let mut properties = BTreeMap::new();
    let mut inherited_typed = BTreeMap::new();
    let mut relative_colors = BTreeMap::new();
    for property in PROPERTIES {
        let parent_value = parent.and_then(|style| style.get(property.name));
        let specified = cascaded.remove(property.name);
        let (mut value, inherits) = match specified.as_deref() {
            None => (default_value(*property, parent_value), property.inherited),
            Some(specified) => compute_normal_value(
                *property,
                specified,
                parent_value,
                root_font_px,
                &mut resolver,
                attrs,
                environment,
            ),
        };
        if relative_currentcolor_identity(&value) || color_mix_currentcolor(&value) {
            relative_colors.insert(property.name.to_owned(), value.clone());
        } else if (specified.as_deref().is_some_and(|value| matches!(css_wide_keyword(value), Some(CssWide::Inherit | CssWide::Unset))) ||
            (specified.is_none() && property.inherited)) &&
            let Some(source) = parent.and_then(|parent| parent.relative_colors.get(property.name)) {
            value = source.clone();
            relative_colors.insert(property.name.to_owned(), source.clone());
        }
        if inherits {
            if let Some(typed) = parent.and_then(|style| style.typed(property.name)) {
                inherited_typed.insert(property.name.to_owned(), typed.clone());
            }
        }
        properties.insert(property.name.to_owned(), value);
    }
    // Initial font-size is typed even when no declaration supplied it.
    if let Some(font) = properties.get_mut("font-size") {
        if let Some(px) =
            computed_font_size(font, parent.and_then(|p| p.get("font-size")), root_font_px)
        {
            *font = px;
        }
    }
    couple_overflow_axes(&mut properties);
    // Preserve declared unknown properties for diagnostics and forward
    // compatibility. They still receive variable substitution, but without
    // metadata an invalid or CSS-wide value cannot manufacture an initial.
    for (name, specified) in cascaded {
        let tokens = tokenize(&specified);
        if let Some(substituted) = resolver.substitute(&tokens) {
            let value = serialize_computed(&substituted);
            if css_wide_keyword(&value).is_none() {
                properties.insert(name, value);
            }
        }
    }
    drop(resolver);
    inherited_custom.extend(resolved_local);
    let mut style = ComputedStyle {
        props: properties,
        typed: BTreeMap::new(),
        custom: inherited_custom,
        relative_colors,
        root_font_px: f64::from(root_font_px),
        effective_zoom: 1.0,
        author_specified: BTreeSet::new(),
        url_bases: parent.map(|parent| parent.url_bases.clone()).unwrap_or_default(),
    };
    style.resolve_relative_colors();
    let local_zoom = style.props.get("zoom").and_then(|value| used_zoom_factor(value)).unwrap_or(1.0);
    style.effective_zoom = parent.map_or(1.0, |parent| parent.effective_zoom) * local_zoom;
    style.rebuild_typed();
    style.typed.extend(inherited_typed);
    // Axis coupling happens after inheritance and can change an inherited axis.
    for axis in ["overflow-x", "overflow-y"] {
        if let Some(value) = style.get(axis).and_then(|css| values::parse(axis, css)) {
            style.typed.insert(axis.to_owned(), value);
        }
    }
    if parent.is_none() {
        if let Some(ComputedValue::LengthPercentage(values::LengthPercentage::Px(px))) =
            style.typed("font-size")
        {
            style.root_font_px = *px;
        }
    }
    style.resolve_typed_units();
    style
}

fn used_zoom_factor(value: &str) -> Option<f64> {
    match values::parse("zoom", value) {
        Some(ComputedValue::Number(n)) => Some(if n == 0.0 { 1.0 } else { n }),
        _ => None,
    }
}

/// Determine rollback after variable substitution without reviving invalid values.
/// Custom properties have already cascaded; CSS-wide tokens produced inside a
/// custom property's substitution remain tokens until consumed by a longhand.
pub(crate) fn substituted_rollback(
    style: &ComputedStyle,
    property: &str,
    value: &str,
) -> Option<Rollback> {
    if property.starts_with("--") || !contains_var_function(value) {
        return None;
    }
    let local = BTreeMap::new();
    let cycles = BTreeSet::new();
    let mut resolver = CustomResolver {
        local: &local,
        inherited: &style.custom,
        cycles: &cycles,
        memo: BTreeMap::new(),
        resolving: BTreeSet::new(),
    };
    let tokens = resolver.substitute(&tokenize(value))?;
    let value = serialize_computed(&tokens);
    let value = resolve_deferred_shorthand(property, &value).ok()?;
    match css_wide_keyword(&value) {
        Some(CssWide::Revert) => Some(Rollback::Origin),
        Some(CssWide::RevertLayer) => Some(Rollback::Layer),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rollback {
    Origin,
    Layer,
}

fn couple_overflow_axes(properties: &mut BTreeMap<String, String>) {
    let x = properties
        .get("overflow-x")
        .cloned()
        .unwrap_or_else(|| "visible".into());
    let y = properties
        .get("overflow-y")
        .cloned()
        .unwrap_or_else(|| "visible".into());
    let coupled = |value: &str, other: &str| {
        let other_scrolls = !matches!(other.to_ascii_lowercase().as_str(), "visible" | "clip");
        if other_scrolls && value.eq_ignore_ascii_case("visible") {
            "auto".to_owned()
        } else if other_scrolls && value.eq_ignore_ascii_case("clip") {
            "hidden".to_owned()
        } else {
            value.to_owned()
        }
    };
    properties.insert("overflow-x".into(), coupled(&x, &y));
    properties.insert("overflow-y".into(), coupled(&y, &x));
}

fn default_value(property: PropertyMetadata, parent: Option<&str>) -> String {
    if property.inherited {
        parent.unwrap_or(property.initial).to_owned()
    } else {
        property.initial.to_owned()
    }
}

fn compute_normal_value(
    property: PropertyMetadata,
    specified: &str,
    parent: Option<&str>,
    root_font_px: f32,
    resolver: &mut CustomResolver<'_>,
    attrs: Option<&[(String, String)]>,
    environment: &MediaEnvironment,
) -> (String, bool) {
    let Some(tokens) = resolver.substitute(&tokenize(specified))
        .and_then(|tokens| attr_values::substitute_attributes(&tokens, attrs))
        .and_then(|tokens| crate::viewport_units::resolve(&tokens, environment)) else {
        // Invalid at computed-value time behaves as `unset`.
        return (default_value(property, parent), property.inherited);
    };
    if trim_tokens(&tokens).is_empty() {
        return (default_value(property, parent), property.inherited);
    }
    let value = serialize_computed(&tokens);
    let Ok(value) = resolve_deferred_shorthand(property.name, &value) else {
        // A shorthand containing var() is parsed only after substitution. If
        // its substituted grammar is invalid, the winning declaration is
        // invalid at computed-value time and behaves as unset; lower cascade
        // candidates do not revive.
        return (default_value(property, parent), property.inherited);
    };
    if matches!(
        property.name,
        "grid-template-rows" | "grid-template-columns"
    ) && css_wide_keyword(&value).is_none()
        && !valid_grid_track_list(&value)
    {
        // Longhand track lists enforce the same grammar as the
        // `grid-template` shorthand; an invalid winner behaves as unset and
        // lower cascade candidates do not revive.
        return (default_value(property, parent), property.inherited);
    }
    if css_wide_keyword(&value).is_none() && !valid_new_paint_property(property.name, &value) {
        return (default_value(property, parent), property.inherited);
    }
    if css_wide_keyword(&value).is_none()
        && values::parse(property.name, &value).is_none()
        && !matches!(property.name, "opacity" | "fill-opacity")
    {
        return (default_value(property, parent), property.inherited);
    }
    let inherits = match css_wide_keyword(&value) {
        Some(CssWide::Inherit) => true,
        Some(CssWide::Unset) => property.inherited,
        _ => property.name == "color" && value.eq_ignore_ascii_case("currentcolor"),
    };
    let computed = if property.name == "color" && value.eq_ignore_ascii_case("currentcolor") {
        parent.unwrap_or(property.initial).to_owned()
    } else {
        match css_wide_keyword(&value) {
            Some(CssWide::Initial) => property.initial.to_owned(),
            Some(CssWide::Inherit) => parent.unwrap_or(property.initial).to_owned(),
            Some(CssWide::Unset) => default_value(property, parent),
            // Cascade handles rollback (including substituted keywords) before
            // the final computed pass. Standalone computation defaults safely.
            Some(CssWide::Revert | CssWide::RevertLayer) => default_value(property, parent),
            None => value,
        }
    };
    let computed = if property.name == "color" && color_mix_currentcolor(&computed) {
        resolve_currentcolor_mix(&computed, parent.unwrap_or("black")).unwrap_or(computed)
    } else { computed };
    let computed = if property.name.starts_with("animation-") || property.name.starts_with("transition-") {
        crate::animation::canonical(property.name, &computed).unwrap_or_else(|| property.initial.to_owned())
    } else if property.name == "font-size" {
        computed_font_size(&computed, parent, root_font_px).unwrap_or(computed)
    } else if crate::font_properties::is_audited_longhand(property.name) {
        crate::font_properties::canonical(property.name, &computed).unwrap_or(computed)
    } else if matches!(property.name, "font-feature-settings" | "font-variant-ligatures") {
        canonical_font_feature_value(property.name, &computed).unwrap_or(computed)
    } else if property.name == "display" {
        values::display_canonical(&computed).unwrap_or(&computed).to_owned()
    } else if matches!(property.name, "ruby-align" | "ruby-position" | "ruby-overhang" | "ruby-merge") {
        values::ruby_canonical(property.name, &computed).unwrap_or(property.initial).to_owned()
    } else if matches!(property.name, "opacity" | "fill-opacity") {
        computed_opacity(&computed).unwrap_or_else(|| property.initial.to_owned())
    } else if property.name == "zoom" {
        match values::parse("zoom", &computed) {
            Some(ComputedValue::Number(n)) => n.to_string(),
            _ => property.initial.to_owned(),
        }
    } else if matches!(property.name, "color" | "background-color"
        | "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color") {
        normalized_extended_color(&computed).unwrap_or(computed)
    } else {
        computed
    };
    (computed, inherits)
}

// A relative color that copies all channels from currentColor into the same
// predefined RGB space has the same displayed color. Retain its source across
// explicit inheritance so each element resolves currentColor against itself.
fn relative_currentcolor_identity(value: &str) -> bool {
    let lower = value.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "hsl(from currentcolor h s l)" | "hsla(from currentcolor h s l)") {
        return true;
    }
    let Some(inner) = lower.strip_prefix("color(").and_then(|value| value.strip_suffix(')')) else {
        return false;
    };
    let words: Vec<_> = inner.split_ascii_whitespace().collect();
    matches!(words.as_slice(), ["from", "currentcolor", "srgb" | "srgb-linear" | "display-p3" |
        "display-p3-linear" | "a98-rgb" | "prophoto-rgb" | "rec2020", "r", "g", "b"])
}

fn color_mix_currentcolor(value: &str) -> bool {
    let tokens = tokenize(value);
    matches!(tokens.first(), Some(Token::Function(name)) if name.eq_ignore_ascii_case("color-mix"))
        && tokens.iter().any(|token| matches!(token, Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor")))
}

fn resolve_currentcolor_mix(value: &str, color: &str) -> Option<String> {
    if !color_mix_currentcolor(value) { return None; }
    let [r, g, b, a] = crate::color::parse_css_color(color)?;
    let mut tokens = tokenize(value);
    for token in &mut tokens {
        if matches!(token, Token::Ident(name) if name.eq_ignore_ascii_case("currentcolor")) {
            *token = Token::Hash(format!("{r:02x}{g:02x}{b:02x}{a:02x}"));
        }
    }
    let [r, g, b, a] = crate::color::parse_css_color(&serialize(&tokens))?;
    Some(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
}

fn normalized_extended_color(value: &str) -> Option<String> {
    let lower = value.trim().to_ascii_lowercase();
    if !["hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch(", "color(", "color-mix(", "contrast-color("].iter()
        .any(|prefix| lower.starts_with(prefix))
        && crate::color::parse_system_color(value).is_none()
    {
        return None;
    }
    let [r, g, b, a] = crate::color::parse_css_color(value)?;
    Some(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
}

fn computed_opacity(value: &str) -> Option<String> {
    let value = value.trim();
    let number = value
        .strip_suffix('%')
        .map(|percent| percent.parse::<f32>().ok().map(|value| value / 100.0))
        .unwrap_or_else(|| value.parse::<f32>().ok())?;
    number
        .is_finite()
        .then(|| number.clamp(0.0, 1.0).to_string())
}

fn valid_new_paint_property(property: &str, value: &str) -> bool {
    match property {
        "background-clip" => valid_background_clip(value),
        "box-shadow" => valid_box_shadow(value),
        "transform" => valid_transform(value),
        "transform-style" => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "flat" | "preserve-3d"
        ),
        "opacity" | "fill-opacity" => valid_opacity(value),
        "z-index" => valid_z_index(value),
        "overflow" | "overflow-x" | "overflow-y" => valid_overflow(value),
        "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-right-radius"
        | "border-bottom-left-radius" => valid_radius_pair(value),
        _ => true,
    }
}

fn valid_z_index(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("auto") || value.parse::<i32>().is_ok()
}

fn deferred_inner<'a>(value: &'a str, function: &str) -> Option<&'a str> {
    value
        .strip_prefix(function)?
        .strip_prefix('(')?
        .strip_suffix(')')
}

fn resolve_deferred_shorthand(property: &str, value: &str) -> Result<String, ()> {
    if let Some(inner) = deferred_inner(value, DEFERRED_LIST_STYLE) {
        return expand_list_style(inner).ok_or(())?.into_iter()
            .find_map(|(name, value)| (name == property).then_some(value)).ok_or(());
    }
    for (function, shorthand) in [(DEFERRED_ANIMATION, "animation"), (DEFERRED_TRANSITION, "transition")] {
        if let Some(inner) = deferred_inner(value, function) {
            return crate::animation::expand_shorthand(shorthand, inner).ok_or(())?.into_iter()
                .find_map(|(name, value)| (name == property).then_some(value)).ok_or(());
        }
    }
    if let Some(inner) = deferred_inner(value, DEFERRED_FONT) {
        return expand_font_shorthand(inner).ok_or(())?.into_iter()
            .find_map(|(name, value)| (name == property).then_some(value)).ok_or(());
    }
    if let Some(inner) = deferred_inner(value, DEFERRED_BACKGROUND) {
        return expand_basic_background(inner).ok_or(())?.into_iter()
            .find_map(|(name, value)| (name == property).then_some(value)).ok_or(());
    }
    if let Some(inner) = deferred_inner(value, DEFERRED_ALL) {
        return css_wide_keyword(inner).map(|_| inner.to_owned()).ok_or(());
    }

    // Every shorthand accepts CSS-wide values after variable substitution.
    for function in [
        DEFERRED_BORDER_WIDTH,
        DEFERRED_BORDER_STYLE,
        DEFERRED_BORDER_COLOR,
        DEFERRED_FLEX_DIRECTION,
        DEFERRED_FLEX_WRAP,
        DEFERRED_GRID_ROWS,
        DEFERRED_GRID_COLUMNS,
        DEFERRED_RADIUS_TOP_LEFT,
        DEFERRED_RADIUS_TOP_RIGHT,
        DEFERRED_RADIUS_BOTTOM_RIGHT,
        DEFERRED_RADIUS_BOTTOM_LEFT,
        DEFERRED_OVERFLOW_X,
        DEFERRED_OVERFLOW_Y,
    ] {
        if let Some(inner) = deferred_inner(value, function) {
            if css_wide_keyword(inner).is_some() {
                return Ok(inner.to_owned());
            }
        }
    }

    for (suffix, function) in [
        ("width", DEFERRED_BORDER_WIDTH),
        ("style", DEFERRED_BORDER_STYLE),
        ("color", DEFERRED_BORDER_COLOR),
    ] {
        if let Some(inner) = deferred_inner(value, function) {
            let side = property.strip_suffix(&format!("-{suffix}")).ok_or(())?;
            return expand_side_border(side, inner).ok_or(())?.into_iter()
                .find_map(|(name, value)| (name == property).then_some(value)).ok_or(());
        }
    }
    let overflow_function = match property {
        "overflow-x" => Some((DEFERRED_OVERFLOW_X, 0usize)),
        "overflow-y" => Some((DEFERRED_OVERFLOW_Y, 1usize)),
        _ => None,
    };
    if let Some((function, index)) = overflow_function
        && let Some(inner) = deferred_inner(value, function)
    {
        let parts = inner.split_ascii_whitespace().collect::<Vec<_>>();
        if !(1..=2).contains(&parts.len()) || !parts.iter().all(|part| valid_overflow(part)) {
            return Err(());
        }
        return Ok(parts.get(index).copied().unwrap_or(parts[0]).to_owned());
    }
    let radius_function = match property {
        "border-top-left-radius" => Some(DEFERRED_RADIUS_TOP_LEFT),
        "border-top-right-radius" => Some(DEFERRED_RADIUS_TOP_RIGHT),
        "border-bottom-right-radius" => Some(DEFERRED_RADIUS_BOTTOM_RIGHT),
        "border-bottom-left-radius" => Some(DEFERRED_RADIUS_BOTTOM_LEFT),
        _ => None,
    };
    if let Some(function) = radius_function
        && let Some(inner) = deferred_inner(value, function)
    {
        return expand_border_radius(inner)
            .ok_or(())?
            .into_iter()
            .find_map(|(name, value)| (name == property).then_some(value))
            .ok_or(());
    }
    let deferred = match property {
        "flex-direction" => deferred_inner(value, DEFERRED_FLEX_DIRECTION)
            .map(|inner| (inner, "flex-direction", true)),
        "flex-wrap" => {
            deferred_inner(value, DEFERRED_FLEX_WRAP).map(|inner| (inner, "flex-wrap", true))
        }
        "grid-template-rows" => deferred_inner(value, DEFERRED_GRID_ROWS)
            .map(|inner| (inner, "grid-template-rows", false)),
        "grid-template-columns" => deferred_inner(value, DEFERRED_GRID_COLUMNS)
            .map(|inner| (inner, "grid-template-columns", false)),
        _ => None,
    };
    let Some((inner, wanted, flex)) = deferred else {
        return Ok(value.to_owned());
    };
    let expanded = if flex {
        expand_flex_flow(inner)
    } else {
        expand_grid_template(inner)
    }
    .ok_or(())?;
    expanded
        .into_iter()
        .find_map(|(property, value)| (property == wanted).then_some(value))
        .ok_or(())
}

fn computed_font_size(value: &str, parent: Option<&str>, root_font_px: f32) -> Option<String> {
    let parent_px = parent.and_then(font_size_px).unwrap_or(16.0);
    let lower = value.trim().to_ascii_lowercase();
    let px = match lower.as_str() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        "larger" => relative_font_size(parent_px, true),
        "smaller" => relative_font_size(parent_px, false),
        _ if lower.ends_with('%') => {
            parent_px * lower[..lower.len() - 1].parse::<f32>().ok()? / 100.0
        }
        _ if lower.ends_with("rem") => {
            root_font_px * lower[..lower.len() - 3].parse::<f32>().ok()?
        }
        _ if lower.ends_with("em") => parent_px * lower[..lower.len() - 2].parse::<f32>().ok()?,
        _ if lower.ends_with("px") => lower[..lower.len() - 2].parse::<f32>().ok()?,
        _ => match values::LengthPercentage::parse(&lower) {
            Some(values::LengthPercentage::Px(px)) => px as f32,
            _ => crate::math::font_size_px(&lower, f64::from(parent_px), f64::from(root_font_px))? as f32,
        },
    };
    if !px.is_finite() || !(0.0..=4096.0).contains(&px) {
        return None;
    }
    Some(format_css_px(px))
}

fn relative_font_size(parent_px: f32, larger: bool) -> f32 {
    const LADDER: [f32; 8] = [9.0, 10.0, 13.0, 16.0, 18.0, 24.0, 32.0, 48.0];
    if larger {
        LADDER
            .into_iter()
            .find(|candidate| *candidate > parent_px)
            .unwrap_or(parent_px * 1.5)
    } else {
        LADDER
            .into_iter()
            .rev()
            .find(|candidate| *candidate < parent_px)
            .unwrap_or(parent_px / 1.5)
    }
}

fn font_size_px(value: &str) -> Option<f32> {
    value
        .trim()
        .strip_suffix("px")?
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

fn format_css_px(px: f32) -> String {
    if px.fract().abs() < f32::EPSILON {
        format!("{px:.0}px")
    } else {
        let mut value = format!("{px:.4}");
        while value.ends_with('0') {
            value.pop();
        }
        format!("{value}px")
    }
}

/// Serializes substituted tokens without merging adjacent component values
/// into a different token on the layout/paint side of the string interface.
/// For example, `var(--n)px` with `--n: 1` remains `1 px` (invalid length),
/// rather than being accidentally retokenized as the valid dimension `1px`.
fn serialize_computed(tokens: &[Token]) -> String {
    fn word_like(token: &Token) -> bool {
        matches!(
            token,
            Token::Ident(_)
                | Token::Hash(_)
                | Token::Number(_)
                | Token::Percentage(_)
                | Token::Dimension(_, _)
        )
    }

    let mut output = String::new();
    let mut previous: Option<&Token> = None;
    for token in tokens {
        if matches!(token, Token::Whitespace) {
            if !output.is_empty() && !output.ends_with(' ') {
                output.push(' ');
            }
            previous = Some(token);
            continue;
        }
        if previous.is_some_and(word_like)
            && (word_like(token) || matches!(token, Token::Function(_) | Token::OpenParen))
        {
            output.push_str("/**/");
        }
        output.push_str(&serialize(std::slice::from_ref(token)));
        previous = Some(token);
    }
    output
}

struct VarCall<'a> {
    name: &'a str,
    fallback: Option<&'a [Token]>,
    end: usize,
}

fn parse_var_call(tokens: &[Token], function_index: usize) -> Option<VarCall<'_>> {
    if !matches!(tokens.get(function_index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
        || tokens.get(function_index + 1) != Some(&Token::OpenParen)
    {
        return None;
    }
    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    let mut comma = None;
    let mut close = None;
    for (offset, token) in tokens[function_index + 2..].iter().enumerate() {
        let index = function_index + 2 + offset;
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth == 0 => {
                close = Some(index);
                break;
            }
            Token::CloseParen => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Comma if paren_depth == 0 && square_depth == 0 && comma.is_none() => {
                comma = Some(index);
            }
            _ => {}
        }
    }
    let close = close?;
    let name_end = comma.unwrap_or(close);
    let name_tokens = trim_tokens(&tokens[function_index + 2..name_end]);
    let [Token::Ident(name)] = name_tokens else {
        return None;
    };
    if !name.starts_with("--") {
        return None;
    }
    let fallback = comma.map(|comma| &tokens[comma + 1..close]);
    Some(VarCall {
        name,
        fallback,
        end: close + 1,
    })
}

fn trim_tokens(mut tokens: &[Token]) -> &[Token] {
    while matches!(tokens.first(), Some(Token::Whitespace)) {
        tokens = &tokens[1..];
    }
    while matches!(tokens.last(), Some(Token::Whitespace)) {
        tokens = &tokens[..tokens.len() - 1];
    }
    tokens
}

/// Parser-facing validation for `var()` grammar and forbidden top-level
/// declaration remnants inside its fallback.
pub(crate) fn valid_variable_syntax(tokens: &[Token]) -> bool {
    let mut index = 0usize;
    while index < tokens.len() {
        if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
        {
            let Some(call) = parse_var_call(tokens, index) else {
                return false;
            };
            if let Some(fallback) = call.fallback
                && (contains_bad_top_level_fallback_token(fallback)
                    || !valid_variable_syntax(fallback))
            {
                return false;
            }
            index = call.end;
        } else {
            index += 1;
        }
    }
    true
}

pub(crate) fn valid_custom_property_value(tokens: &[Token]) -> bool {
    !contains_bad_top_level_fallback_token(tokens) && valid_variable_syntax(tokens)
}

fn contains_bad_top_level_fallback_token(tokens: &[Token]) -> bool {
    let mut paren_depth = 0usize;
    let mut square_depth = 0usize;
    for token in tokens {
        match token {
            Token::OpenParen => paren_depth += 1,
            Token::CloseParen if paren_depth > 0 => paren_depth -= 1,
            Token::OpenSquare => square_depth += 1,
            Token::CloseSquare if square_depth > 0 => square_depth -= 1,
            Token::Semicolon | Token::Delim('!') if paren_depth == 0 && square_depth == 0 => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn collect_dependencies(tokens: &[Token], dependencies: &mut BTreeSet<String>) {
    let mut index = 0usize;
    while index < tokens.len() {
        if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
            && let Some(call) = parse_var_call(tokens, index)
        {
            dependencies.insert(call.name.to_owned());
            if let Some(fallback) = call.fallback {
                collect_dependencies(fallback, dependencies);
            }
            index = call.end;
            continue;
        }
        index += 1;
    }
}

fn cycle_members(local: &BTreeMap<String, Vec<Token>>) -> BTreeSet<String> {
    fn visit(
        name: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        states: &mut HashMap<String, u8>,
        stack: &mut Vec<String>,
        cycles: &mut BTreeSet<String>,
    ) {
        match states.get(name).copied() {
            Some(1) => {
                if let Some(position) = stack.iter().position(|entry| entry == name) {
                    cycles.extend(stack[position..].iter().cloned());
                }
                return;
            }
            Some(2) => return,
            _ => {}
        }
        states.insert(name.to_owned(), 1);
        stack.push(name.to_owned());
        if let Some(dependencies) = graph.get(name) {
            for dependency in dependencies {
                if graph.contains_key(dependency) {
                    visit(dependency, graph, states, stack, cycles);
                }
            }
        }
        stack.pop();
        states.insert(name.to_owned(), 2);
    }

    let graph: BTreeMap<String, BTreeSet<String>> = local
        .iter()
        .map(|(name, tokens)| {
            let mut dependencies = BTreeSet::new();
            collect_dependencies(tokens, &mut dependencies);
            (name.clone(), dependencies)
        })
        .collect();
    let mut states = HashMap::new();
    let mut cycles = BTreeSet::new();
    for name in graph.keys() {
        visit(name, &graph, &mut states, &mut Vec::new(), &mut cycles);
    }
    cycles
}

struct CustomResolver<'a> {
    local: &'a BTreeMap<String, Vec<Token>>,
    inherited: &'a BTreeMap<String, CustomValue>,
    cycles: &'a BTreeSet<String>,
    memo: BTreeMap<String, CustomValue>,
    resolving: BTreeSet<String>,
}

impl CustomResolver<'_> {
    fn resolve_name(&mut self, name: &str) -> CustomValue {
        if let Some(value) = self.memo.get(name) {
            return value.clone();
        }
        if self.cycles.contains(name) || !self.resolving.insert(name.to_owned()) {
            return CustomValue::Invalid;
        }
        let value = match self.local.get(name).cloned() {
            Some(tokens) => self
                .substitute(&tokens)
                .map(CustomValue::Valid)
                .unwrap_or(CustomValue::Invalid),
            None => self
                .inherited
                .get(name)
                .cloned()
                .unwrap_or(CustomValue::Invalid),
        };
        self.resolving.remove(name);
        self.memo.insert(name.to_owned(), value.clone());
        value
    }

    fn substitute(&mut self, tokens: &[Token]) -> Option<Vec<Token>> {
        let mut output = Vec::new();
        let mut index = 0usize;
        while index < tokens.len() {
            if matches!(tokens.get(index), Some(Token::Function(name)) if name.eq_ignore_ascii_case("var"))
            {
                let call = parse_var_call(tokens, index)?;
                let replacement = match self.resolve_name(call.name) {
                    CustomValue::Valid(tokens) => Some(tokens),
                    CustomValue::Invalid => {
                        call.fallback.and_then(|fallback| self.substitute(fallback))
                    }
                }?;
                output.extend(replacement);
                index = call.end;
            } else {
                output.push(tokens[index].clone());
                index += 1;
            }
        }
        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_attributes_substitute_after_variables_and_validate_casts() {
        let attrs = vec![("data-WIDTH".to_owned(), "200px".to_owned()),
            ("data-bad".to_owned(), "red".to_owned())];
        let style = resolve_with_root_and_attrs(None, cascaded(&[
            ("--size", "attr(DATA-width type(<length>))"),
            ("width", "var(--size)"),
            ("height", "attr(data-bad type(<length>), 40px)"),
            ("min-width", "attr(data-bad type(<length>))"),
        ]), 16.0, Some(&attrs));
        assert_eq!(style.get("width"), Some("200px"));
        assert_eq!(style.get("height"), Some("40px"));
        assert_eq!(style.get("min-width"), Some("auto"));
    }

    #[test]
    fn untyped_attribute_fallback_requires_a_string() {
        let empty = resolve_with_root_and_attrs(None,
            cascaded(&[("content", "attr(missing, invalid)")]), 16.0, Some(&[]));
        assert_eq!(empty.get("content"), Some("\"\""));
        let fallback = resolve_with_root_and_attrs(None,
            cascaded(&[("content", "attr(missing, 'fallback')")]), 16.0, Some(&[]));
        assert_eq!(fallback.get("content"), Some("\"fallback\""));
    }

    #[test]
    fn deprecated_system_colors_resolve_before_paint_handoff() {
        let style = resolve(None, cascaded(&[
            ("background-color", "ButtonShadow"),
            ("border-top-color", "ThreeDShadow"),
            ("color", "silver"),
        ]));
        assert_eq!(style.get("background-color"), Some("#f0f0f0ff"));
        assert_eq!(style.get("border-top-color"), Some("#808080ff"));
        assert_eq!(style.get("color"), Some("silver"));
    }

    fn cascaded(values: &[(&str, &str)]) -> BTreeMap<String, String> {
        values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn case_sensitive_custom_properties_inherit_and_substitute() {
        let parent = resolve(None, cascaded(&[("--A", "green"), ("--a", "crimson")]));
        let child = resolve(
            Some(&parent),
            cascaded(&[("color", "var(--A)"), ("background-color", "var(--a)")]),
        );
        assert_eq!(child.get("color"), Some("green"));
        assert_eq!(child.get("background-color"), Some("crimson"));
        assert_eq!(child.custom_serialized("--A"), Some(Some("green".into())));
        assert_eq!(child.custom_serialized("--a"), Some(Some("crimson".into())));
    }

    #[test]
    fn nested_fallback_and_fallback_commas_survive_substitution() {
        let style = resolve(
            None,
            cascaded(&[
                ("color", "var(--missing,var(--also-missing,green))"),
                ("font-family", "var(--family,serif,sans-serif)"),
            ]),
        );
        assert_eq!(style.get("color"), Some("green"));
        assert_eq!(style.get("font-family"), Some("serif,sans-serif"));
    }

    #[test]
    fn cycle_members_are_invalid_before_consumer_fallback_runs() {
        let style = resolve(
            None,
            cascaded(&[
                ("--a", "var(--b)"),
                ("--b", "var(--a,red)"),
                ("color", "var(--a,green)"),
            ]),
        );
        assert_eq!(style.custom_serialized("--a"), Some(None));
        assert_eq!(style.custom_serialized("--b"), Some(None));
        assert_eq!(style.get("color"), Some("green"));
    }

    #[test]
    fn missing_var_is_invalid_at_computed_value_time_not_previous_cascade_value() {
        let style = resolve(None, cascaded(&[("color", "var(--missing)")]));
        assert_eq!(style.get("color"), Some("black"));
    }

    #[test]
    fn css_wide_keywords_use_property_metadata() {
        let parent = resolve(None, cascaded(&[("color", "blue"), ("display", "block")]));
        let child = resolve(
            Some(&parent),
            cascaded(&[
                ("color", "unset"),
                ("display", "inherit"),
                ("background-color", "initial"),
            ]),
        );
        assert_eq!(child.get("color"), Some("blue"));
        assert_eq!(child.get("display"), Some("block"));
        assert_eq!(child.get("background-color"), Some("transparent"));
    }

    #[test]
    fn all_expands_supported_properties_but_excludes_direction_and_unicode_bidi() {
        let expanded = expand_declaration("all", "initial");
        assert!(expanded.iter().any(|(name, _)| name == "color"));
        assert!(expanded.iter().any(|(name, _)| name == "margin-left"));
        assert!(!expanded.iter().any(|(name, _)| name == "direction"));
        assert!(!expanded.iter().any(|(name, _)| name == "unicode-bidi"));
    }

    #[test]
    fn variable_grammar_distinguishes_nested_from_top_level_remnants() {
        assert!(valid_custom_property_value(&tokenize("var(--a,(;))")));
        assert!(valid_custom_property_value(&tokenize("var(--a,)")));
        assert!(!valid_custom_property_value(&tokenize("var(--a,;)")));
        assert!(!valid_custom_property_value(&tokenize("var(--a,!)")));
    }

    #[test]
    fn substitution_preserves_token_boundaries() {
        let style = resolve(None, cascaded(&[("--n", "1"), ("width", "var(--n)px")]));
        assert_eq!(style.get("width"), Some("auto"));
        assert_eq!(
            style.typed("width"),
            Some(&ComputedValue::Keyword(values::Keyword::Auto))
        );
        assert_eq!(
            serialize_computed(&[Token::Number(1.0), Token::Ident("px".into())]),
            "1/**/px"
        );
    }

    #[test]
    fn public_set_uses_the_same_shorthand_expansion_seam() {
        let mut style = ComputedStyle::default();
        style.set("gap", "10px 20px");
        style.set("grid-column", "1 / -1");
        assert_eq!(style.get("row-gap"), Some("10px"));
        assert_eq!(style.get("column-gap"), Some("20px"));
        assert_eq!(style.get("grid-column-start"), Some("1"));
        assert_eq!(style.get("grid-column-end"), Some("-1"));
        assert_eq!(style.get("gap"), None);
        assert_eq!(style.get("grid-column"), None);
    }

    #[test]
    fn grid_lines_validate_both_longhands_and_shorthand_halves() {
        for property in ["grid-column-start", "grid-column-end", "grid-row-start", "grid-row-end"] {
            for invalid in ["default", "0", "span 0", "span -1", "span", "auto foo", "1 2"] {
                assert!(expand_declaration(property, invalid).is_empty(), "{property}: {invalid}");
            }
            for valid in ["auto", "foo", "-1 foo", "span foo 2", "foo span", "span 2"] {
                assert_eq!(expand_declaration(property, valid), vec![(property.to_owned(), valid.to_owned())]);
            }
        }
        for property in ["grid-column", "grid-row"] {
            for invalid in ["default", "1 / default", "default / 1", "span 0", "0 / 1", "1 / 0", "1 / 2 / 3"] {
                assert!(expand_declaration(property, invalid).is_empty(), "{property}: {invalid}");
            }
            assert_eq!(expand_declaration(property, "-1 foo / span foo 2"), vec![
                (format!("{property}-start"), "-1 foo".into()),
                (format!("{property}-end"), "span foo 2".into()),
            ]);
            assert_eq!(expand_declaration(property, "foo"), vec![
                (format!("{property}-start"), "foo".into()),
                (format!("{property}-end"), "foo".into()),
            ]);
            assert_eq!(expand_declaration(property, "span foo 2"), vec![
                (format!("{property}-start"), "span foo 2".into()),
                (format!("{property}-end"), "auto".into()),
            ]);
            assert_eq!(expand_declaration(property, "var(--line)").len(), 2);
        }
        let declarations = [("grid-column", "default"), ("grid-row-start", "default")];
        let style = resolve(None, declarations.into_iter()
            .flat_map(|(property, value)| expand_declaration(property, value))
            .collect());
        assert_eq!(style.get("grid-column-start"), Some("auto"));
        assert_eq!(style.get("grid-column-end"), Some("auto"));
        assert_eq!(style.get("grid-row-start"), Some("auto"));
    }

    #[test]
    fn flex_flow_rejects_unknown_and_duplicate_components() {
        assert_eq!(
            expand_declaration("flex-flow", "wrap column"),
            vec![
                ("flex-direction".into(), "column".into()),
                ("flex-wrap".into(), "wrap".into()),
            ]
        );
        assert_eq!(
            expand_declaration("flex-flow", "row column"),
            Vec::<(String, String)>::new()
        );
        assert_eq!(
            expand_declaration("flex-flow", "row banana"),
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn grid_template_only_splits_on_a_top_level_solidus() {
        assert!(
            expand_declaration(
                "grid-template",
                "minmax(10px,calc(20px / 2)) / repeat(2,1fr)"
            )
            .is_empty()
        );
        assert_eq!(
            expand_declaration("grid-template", "10px / 20px / 30px"),
            Vec::<(String, String)>::new()
        );
        assert!(expand_declaration("grid-template", "banana / banana").is_empty());
        assert!(expand_declaration("grid-template", "fit-content(auto) / 1fr").is_empty());
        assert!(expand_declaration("grid-template", "[span] 10px / 1fr").is_empty());
        assert!(expand_declaration("grid-template", "repeat(auto-fill,1fr) / 30px").is_empty());
        assert!(
            expand_declaration(
                "grid-template",
                "repeat(auto-fill,10px) repeat(auto-fit,20px) / 30px"
            )
            .is_empty()
        );
    }

    #[test]
    fn paint_properties_use_validated_computed_value_grammars() {
        for property in [
            "background-clip",
            "box-shadow",
            "transform",
            "transform-style",
            "border-top-left-radius",
        ] {
            assert!(
                is_supported_property(property),
                "missing metadata for {property}"
            );
        }
        assert!(expand_declaration("transform", "translate(10px,20px)").len() == 1);
        assert!(expand_declaration("transform", "translate(10px,)").is_empty());
        assert!(expand_declaration("transform", "scale(1,2,3)").is_empty());
        assert!(expand_declaration("box-shadow", "5px 5px black").len() == 1);
        assert!(expand_declaration("box-shadow", "5px 5px banana").is_empty());
        assert!(expand_declaration("box-shadow", "5px 5px rgb(banana)").is_empty());
        assert!(expand_declaration("box-shadow", "inset inset 5px 5px black").is_empty());
        assert!(expand_declaration("box-shadow", "5px 5px red blue").is_empty());
        let radius = expand_declaration("border-radius", "10px 20% / 5px");
        assert_eq!(radius.len(), 4);
        assert_eq!(
            radius[0],
            ("border-top-left-radius".into(), "10px 5px".into())
        );
        assert_eq!(
            radius[1],
            ("border-top-right-radius".into(), "20% 5px".into())
        );
        assert!(expand_declaration("opacity", "banana").is_empty());
        assert_eq!(
            expand_declaration("z-index", "-2"),
            vec![("z-index".to_owned(), "-2".to_owned())]
        );
        assert!(expand_declaration("z-index", "2.5").is_empty());
        assert!(expand_declaration("z-index", "banana").is_empty());
        assert!(expand_declaration("overflow-x", "banana").is_empty());
    }

    #[test]
    fn font_shorthand_expands_size_line_height_family_and_resets_optionals() {
        let expanded =
            expand_declaration("font", "italic 700 120px / 2 \"Arabic Test\",sans-serif");
        let values: BTreeMap<_, _> = expanded.into_iter().collect();

        assert_eq!(values.get("font-size").map(String::as_str), Some("120px"));
        assert_eq!(values.get("line-height").map(String::as_str), Some("2"));
        assert_eq!(
            values.get("font-family").map(String::as_str),
            Some("\"Arabic Test\",sans-serif")
        );
        assert_eq!(values.get("font-style").map(String::as_str), Some("italic"));
        assert_eq!(
            values.get("font-variant").map(String::as_str),
            Some("normal")
        );
        assert_eq!(values.get("font-weight").map(String::as_str), Some("700"));

        assert_eq!(
            expand_declaration("font", "italic oblique 16px serif"),
            vec![("font".into(), "italic oblique 16px serif".into())]
        );
        assert_eq!(
            expand_declaration("font", "16px \"Quoted\" adjacent"),
            vec![("font".into(), "16px \"Quoted\" adjacent".into())]
        );
        assert_eq!(
            expand_declaration("font", "normal normal normal normal italic 16px serif"),
            vec![(
                "font".into(),
                "normal normal normal normal italic 16px serif".into()
            )]
        );
    }
}
