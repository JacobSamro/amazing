//! CSS value adoption at setup time. No parsing or length resolution is needed
//! while a retained animation ticks. Reference spaces and resources belong to
//! the producer; unresolved input must select its full-rendering fallback.
use super::*;
use amazing_css::{effects as css, values::LengthPercentage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CssEffectError {
    InvalidSyntax,
    UnresolvedValue,
    UnsupportedProperty,
    InvalidEffect(EffectError),
}
impl From<EffectError> for CssEffectError {
    fn from(value: EffectError) -> Self { Self::InvalidEffect(value) }
}

/// Resolve an entire CSS filter list atomically. `resolve_length` supplies
/// producer-resolved relative lengths in CSS/raster pixels in the same space
/// as the group's source. Absolute lengths are already canonical CSS pixels.
/// Percentages are not filter lengths. URLs and calc expressions require the
/// producer's resource/computed-value pipeline and return `UnresolvedValue`.
/// `current_color` is the element's resolved color, including alpha.
pub fn resolve_css_filters(
    list: &css::FilterList,
    current_color: [u8; 4],
    mut resolve_length: impl FnMut(LengthPercentage) -> Option<f32>,
) -> Result<Vec<FilterOperation>, CssEffectError> {
    use css::FilterFunction as C;
    use FilterOperation as F;
    let css::FilterList::Functions(functions) = list else { return Ok(Vec::new()); };
    if functions.len() > MAX_EFFECT_FILTERS { return Err(EffectError::LimitExceeded.into()); }
    let mut length = |value| {
        let value = match value {
            LengthPercentage::Px(px) => Some(px as f32),
            LengthPercentage::Relative(..) => resolve_length(value),
            LengthPercentage::Percentage(_) => None,
        };
        value.filter(|v| v.is_finite()).ok_or(CssEffectError::UnresolvedValue)
    };
    let mut out = Vec::with_capacity(functions.len());
    for function in functions {
        out.push(match function {
            C::Blur(v) => F::Blur(length(*v)?),
            C::Brightness(v) => F::Brightness(*v as f32),
            C::Contrast(v) => F::Contrast(*v as f32),
            C::Grayscale(v) => F::Grayscale(*v as f32),
            // CSS stores radians; the retained filter kernel takes degrees.
            C::HueRotate(v) => F::HueRotate(v.to_degrees() as f32),
            C::Invert(v) => F::Invert(*v as f32),
            C::Opacity(v) => F::Opacity(*v as f32),
            C::Saturate(v) => F::Saturate(*v as f32),
            C::Sepia(v) => F::Sepia(*v as f32),
            C::DropShadow(shadow) => F::DropShadow {
                offset: (length(shadow.offset_x)?, length(shadow.offset_y)?),
                sigma: shadow.blur.map(&mut length).transpose()?.unwrap_or(0.),
                color: match shadow.color.as_deref() {
                    None => current_color,
                    Some(color) if color.eq_ignore_ascii_case("currentcolor") => current_color,
                    Some(color) => amazing_css::color::parse_css_color(color)
                        .ok_or(CssEffectError::UnresolvedValue)?,
                },
            },
            C::Url(_) | C::Expression { .. } => return Err(CssEffectError::UnresolvedValue),
        });
    }
    EffectOperations { filters: out.clone(), ..Default::default() }.validate()?;
    Ok(out)
}

/// Convert CSS's blend enum exhaustively, including additive compositing.
pub fn resolve_css_blend(value: css::BlendMode) -> Result<EffectBlend, CssEffectError> {
    use css::BlendMode as C;
    Ok(match value {
        C::Normal => EffectBlend::Normal,
        C::Multiply => EffectBlend::Multiply,
        C::Screen => EffectBlend::Screen,
        C::Overlay => EffectBlend::Overlay,
        C::Darken => EffectBlend::Darken,
        C::Lighten => EffectBlend::Lighten,
        C::ColorDodge => EffectBlend::ColorDodge,
        C::ColorBurn => EffectBlend::ColorBurn,
        C::HardLight => EffectBlend::HardLight,
        C::SoftLight => EffectBlend::SoftLight,
        C::Difference => EffectBlend::Difference,
        C::Exclusion => EffectBlend::Exclusion,
        C::Hue => EffectBlend::Hue,
        C::Saturation => EffectBlend::Saturation,
        C::Color => EffectBlend::Color,
        C::Luminosity => EffectBlend::Luminosity,
        C::PlusLighter => EffectBlend::PlusLighter,
    })
}

/// Resolve CSS-13's whole effect delta without mutating the retained scene.
/// This is a value converter, not permission to bypass a stacking-context or
/// reference-space rebuild. The caller must establish those invariants first.
/// `backdrop_bounds` is the producer's resolved raster-space border box.
/// Shape/mask deltas need geometry/decoded resources and fail explicitly;
/// unchanged clips/masks are preserved. No partial update escapes on failure.
pub fn resolve_css_effect_delta(
    current: &EffectOperations,
    changes: &[(String, String)],
    backdrop_bounds: Option<Rect>,
    current_color: [u8; 4],
    mut resolve_length: impl FnMut(LengthPercentage) -> Option<f32>,
) -> Result<EffectOperations, CssEffectError> {
    let mut out = current.clone();
    for (property, value) in changes {
        match property.as_str() {
            "filter" | "backdrop-filter" => {
                let list = css::parse_filter_list(value).ok_or(CssEffectError::InvalidSyntax)?;
                let filters = resolve_css_filters(&list, current_color, &mut resolve_length)?;
                if property == "filter" {
                    out.filters = filters;
                } else {
                    out.backdrop_bounds = if filters.is_empty() { None } else {
                        Some(backdrop_bounds.ok_or(CssEffectError::UnresolvedValue)?)
                    };
                    out.backdrop_filters = filters;
                }
            }
            "mix-blend-mode" => out.blend = resolve_css_blend(
                css::parse_blend_mode(value).ok_or(CssEffectError::InvalidSyntax)?)?,
            "isolation" => out.isolate = css::parse_isolation(value)
                .ok_or(CssEffectError::InvalidSyntax)?,
            _ => return Err(CssEffectError::UnsupportedProperty),
        }
    }
    out.validate()?;
    Ok(out)
}
