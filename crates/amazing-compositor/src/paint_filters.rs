//! Painter filter metadata adoption and the resolved producer boundary.
use super::*;
use amazing_paint::display_list_view::DisplayListView;

/// Filters currently execute on the post-transform render-pass raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintFilterSpace {
    /// Parameters and bounds already resolved into the compositing raster.
    Raster,
    /// Local CSS coordinates require transform/reference-space support first.
    Local,
}

/// An existing non-root paint effect group and its resolved ordered filters.
/// The producer must create a group even for filter-only elements. Raster
/// items must exclude these effects; baked filter pixels would apply twice.
#[derive(Debug, Clone, PartialEq)]
pub struct PaintFilterInput {
    pub effect: EffectId,
    pub space: PaintFilterSpace,
    pub filters: Vec<FilterOperation>,
    pub backdrop_filters: Vec<FilterOperation>,
    pub backdrop_bounds: Option<Rect>,
    /// RC-27's resolved `clip-path` basic-shape mask (`inset`/`circle`/
    /// `ellipse`), converted to the retained `EffectClip` representation.
    pub clip_path: Option<EffectClip>,
}

/// Implemented by the producer adapter once computed lengths, resources,
/// reference spaces and border bounds are known. No style lookup during frames.
pub trait PaintFilterSource {
    fn filter_inputs(&self) -> &[PaintFilterInput];
}
impl PaintFilterSource for Vec<PaintFilterInput> {
    fn filter_inputs(&self) -> &[PaintFilterInput] { self }
}

/// Import all filters atomically, preserving the painter's blend, isolation,
/// opacity and ancestry. Missing, duplicate, root and unresolved inputs fail.
/// Partial property trees never escape an invalid batch.
pub fn paint_view_metadata_with_filters(
    view: &DisplayListView<'_>,
    inputs: &[PaintFilterInput],
) -> Result<(PropertyTrees, Vec<PaintChunk>), SceneError> {
    if inputs.len() >= view.effects().len() && !inputs.is_empty() {
        return Err(SceneError::InvalidState);
    }
    let (mut trees, chunks) = super::paint_view::paint_view_metadata_base(view)?;
    let mut seen = std::collections::HashSet::new();
    for input in inputs {
        if input.effect.0 == 0 || input.effect.0 >= view.effects().len()
            || !seen.insert(input.effect) || input.space != PaintFilterSpace::Raster
            || input.backdrop_bounds.is_some_and(|r|
                [r.x, r.y, r.w, r.h].iter().any(|v| !v.is_finite()) || r.empty())
        {
            return Err(SceneError::InvalidState);
        }
        let mut operations = trees.effect_operations(input.effect).cloned().unwrap_or_default();
        operations.filters = input.filters.clone();
        operations.backdrop_filters = input.backdrop_filters.clone();
        operations.backdrop_bounds = input.backdrop_bounds;
        operations.clip_path = input.clip_path.clone();
        let node = &view.effects()[input.effect.0];
        if node.filter.ops().map(import_filter).collect::<Vec<_>>() == input.filters
            && node.backdrop_filter.ops().map(import_filter).collect::<Vec<_>>() == input.backdrop_filters
            && (!node.filter.is_empty() || !node.backdrop_filter.is_empty())
        {
            operations.painter_filters = Some((node.filter, node.backdrop_filter));
        }
        trees.set_effect_operations(input.effect, operations)
            .map_err(|_| SceneError::InvalidState)?;
    }
    Ok((trees, chunks))
}

/// Resolved RC-25/26 plans indexed by their stable per-build paint effect IDs.
pub struct PainterFilterSource(Vec<PaintFilterInput>);
impl PaintFilterSource for PainterFilterSource {
    fn filter_inputs(&self) -> &[PaintFilterInput] { &self.0 }
}
impl PainterFilterSource {
    pub fn from_view(view: &DisplayListView<'_>) -> Result<Self, SceneError> {
        let has_filters = view.effects().iter().any(|n|
            !n.filter.is_empty() || !n.backdrop_filter.is_empty());
        let has_clip = view.effects().iter().any(|n| n.clip.is_some());
        // Painter filters and clip masks both execute on the group's local,
        // pre-transform surface (see `composite_group`'s inverse-mapped mask
        // test); retained filters/clip currently apply in the same identity-
        // space `DrawQuad::Pass` stage. Preserve correctness via full paint
        // whenever any transform in the view is not identity, rather than
        // reasoning per-ancestor about which effect it actually affects.
        if (has_filters || has_clip) && view.transforms().iter().any(|n|
            n.matrix != [1., 0., 0., 1., 0., 0.])
        {
            return Err(SceneError::InvalidState);
        }
        let mut inputs = Vec::new();
        for (index, node) in view.effects().iter().enumerate().skip(1) {
            if node.filter.is_empty() && node.backdrop_filter.is_empty() && node.clip.is_none() {
                continue;
            }
            let bounds = node.backdrop_bounds;
            let clip_path = match node.clip {
                Some(mask) => Some(import_clip(mask).ok_or(SceneError::InvalidState)?),
                None => None,
            };
            let input = PaintFilterInput {
                effect: EffectId(index), space: PaintFilterSpace::Raster,
                filters: node.filter.ops().map(import_filter).collect(),
                backdrop_filters: node.backdrop_filter.ops().map(import_filter).collect(),
                backdrop_bounds: (!node.backdrop_filter.is_empty()).then_some(
                    Rect::new(bounds.x, bounds.y, bounds.width, bounds.height)),
                clip_path,
            };
            EffectOperations {
                filters: input.filters.clone(),
                backdrop_filters: input.backdrop_filters.clone(),
                backdrop_bounds: input.backdrop_bounds,
                clip_path: input.clip_path.clone(),
                ..Default::default()
            }.validate().map_err(|_| SceneError::InvalidState)?;
            inputs.push(input);
        }
        Ok(Self(inputs))
    }
}

/// Convert a resolved painter `inset()`/`circle()`/`ellipse()` clip mask into
/// the retained shape. `circle()`/`ellipse()` resolve onto their own exact
/// bounding box with every corner sharing one `(rx, ry)` pair (equal for
/// `circle()`, independent per axis for `ellipse()`); that case is checked
/// first and mapped onto the retained `Ellipse` variant so its unbounded
/// elliptical test matches the source shape exactly rather than approximating
/// it with four independent rounded corners. Otherwise this is an `inset()`,
/// whose painter parser never splits a corner's radius into separate
/// horizontal/vertical values (no `/` clause support), so every corner's
/// `(rx, ry)` pair is always equal; a mismatch there is defensive and falls
/// back rather than guessing.
pub(crate) fn import_clip(mask: amazing_paint::clip::ClipMask) -> Option<EffectClip> {
    let amazing_paint::clip::ClipMask { x, y, width, height, radii } = mask;
    let (rx, ry) = radii[0];
    if radii.iter().all(|r| *r == (rx, ry)) && rx > 0. && ry > 0.
        && width == rx * 2. && height == ry * 2.
    {
        return Some(EffectClip::Ellipse { center: (x + rx, y + ry), radii: (rx, ry) });
    }
    if radii.iter().any(|(rx, ry)| rx != ry) {
        return None;
    }
    Some(EffectClip::Inset {
        rect: Rect::new(x, y, width, height),
        radii: [radii[0].0, radii[1].0, radii[2].0, radii[3].0],
    })
}
pub(crate) fn import_filter(op: amazing_paint::filter::FilterOp) -> FilterOperation {
    use amazing_paint::filter::FilterOp as P;
    use FilterOperation as C;
    match op {
        P::Brightness(v) => C::Brightness(v), P::Contrast(v) => C::Contrast(v),
        P::Grayscale(v) => C::Grayscale(v), P::HueRotate(v) => C::HueRotate(v),
        P::Invert(v) => C::Invert(v), P::Opacity(v) => C::Opacity(v),
        P::Saturate(v) => C::Saturate(v), P::Sepia(v) => C::Sepia(v),
        P::Blur(v) => C::Blur(v),
        P::DropShadow { dx, dy, std, color } =>
            C::DropShadow { offset: (dx,dy), sigma: std, color },
    }
}

/// Import the real painter plans together with opacity, blend and isolation.
pub fn paint_view_metadata(
    view: &DisplayListView<'_>,
) -> Result<(PropertyTrees, Vec<PaintChunk>), SceneError> {
    let provider = PainterFilterSource::from_view(view)?;
    paint_view_metadata_with_filters(view, provider.filter_inputs())
}
