//! Producer boundary for filter metadata not yet exported by the painter.
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
    let (mut trees, chunks) = paint_view_metadata(view)?;
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
        trees.set_effect_operations(input.effect, operations)
            .map_err(|_| SceneError::InvalidState)?;
    }
    Ok((trees, chunks))
}
