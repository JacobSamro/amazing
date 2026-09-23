//! Diagnostic compositor comparison at one frozen rendering update.
//!
//! The driver currently rebuilds style, layout and paint after DOM changes.
//! This is the pixel boundary of the incremental oracle: an eventual retained
//! candidate can be substituted without changing the full reference path.
use amazing_compositor::{PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_css::{StyledDom, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_layout::{Flow, LayoutBox};
use amazing_paint::{Surface, controls::NoControls, display_list_view::{DisplayListView, PaintItem, TransformNode, ClipNode, EffectNode, ScrollNode}};
use amazing_text::TextSystem;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::page::PagePaint;

static PARITY_LOSSES: AtomicU64 = AtomicU64::new(0);
static FULL_FALLBACKS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn parity_loss(detail: &str) {
    let losses = PARITY_LOSSES.fetch_add(1, Ordering::Relaxed) + 1;
    let fallbacks = FULL_FALLBACKS.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!("amazing oracle: parity loss: {detail}; presenting full; parity_losses={losses} full_fallbacks={fallbacks}");
}

pub(crate) fn full_fallback(detail: &str) {
    let fallbacks = FULL_FALLBACKS.fetch_add(1, Ordering::Relaxed) + 1;
    eprintln!("amazing oracle: compositor fallback: {detail}; presenting full; parity_losses={} full_fallbacks={fallbacks}", PARITY_LOSSES.load(Ordering::Relaxed));
}

pub(crate) fn matched(width: u32, height: u32) {
    eprintln!("amazing oracle: compositor/full pixel parity at {width}x{height}; parity_losses={} full_fallbacks={}", PARITY_LOSSES.load(Ordering::Relaxed), FULL_FALLBACKS.load(Ordering::Relaxed));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PixelMismatch {
    pub x: u32,
    pub y: u32,
    pub candidate: [u8; 4],
    pub full: [u8; 4],
}

pub(crate) fn first_pixel_mismatch(candidate: &Surface, full: &Surface) -> Option<PixelMismatch> {
    if candidate.width != full.width || candidate.height != full.height {
        return Some(PixelMismatch {
            x: 0,
            y: 0,
            candidate: candidate.get_pixel(0, 0).unwrap_or([0; 4]),
            full: full.get_pixel(0, 0).unwrap_or([0; 4]),
        });
    }
    let width = candidate.width;
    candidate.pixels.chunks_exact(4).zip(full.pixels.chunks_exact(4))
        .enumerate().find_map(|(index, (candidate, full))| {
            (candidate != full).then(|| PixelMismatch {
                x: index as u32 % width,
                y: index as u32 / width,
                candidate: candidate.try_into().unwrap(),
                full: full.try_into().unwrap(),
            })
        })
}

pub(crate) fn enabled() -> bool {
    std::env::var_os("AMAZING_INVALIDATION_ORACLE").is_some_and(|value| value == "1")
}

pub(crate) fn first_style_mismatch(dom: &Dom, candidate: &StyledDom, full: &StyledDom) -> Option<String> {
    for index in 0..dom.node_count() {
        let node = NodeIdx(index as u32);
        let mut properties = BTreeSet::new();
        for style in [candidate.get(node), full.get(node)].into_iter().flatten() {
            properties.extend(style.iter().map(|(name, _)| name.to_owned()));
        }
        if candidate.get(node).is_some() != full.get(node).is_some() {
            return Some(format!("style node {index}: element presence"));
        }
        for property in properties {
            if candidate.get(node).and_then(|style| style.get(&property))
                != full.get(node).and_then(|style| style.get(&property)) {
                return Some(format!("style node {index}: {property}"));
            }
        }
        if candidate.get(node) != full.get(node) {
            return Some(format!("style node {index}: computed metadata"));
        }
        for pseudo in PseudoElement::STYLED {
            if candidate.get_pseudo(node, pseudo) != full.get_pseudo(node, pseudo)
                || candidate.pseudo_matched(node, pseudo) != full.pseudo_matched(node, pseudo)
                || format!("{:?}", candidate.generated_content(node, pseudo))
                    != format!("{:?}", full.generated_content(node, pseudo)) {
                return Some(format!("style node {index}: {pseudo:?}"));
            }
        }
    }
    None
}

pub(crate) fn first_layout_mismatch(candidate: &LayoutBox, full: &LayoutBox) -> Option<String> {
    fn children(layout: &LayoutBox) -> Option<&[LayoutBox]> {
        match &layout.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => Some(children),
            _ => None,
        }
    }
    fn walk(candidate: &LayoutBox, full: &LayoutBox, path: &str) -> Option<String> {
        if format!("{candidate:?}") == format!("{full:?}") { return None; }
        let label = candidate.node.map_or_else(|| path.to_owned(), |node| format!("node {}", node.0));
        if candidate.node != full.node { return Some(format!("{label}: identity")); }
        if candidate.style != full.style { return Some(format!("{label}: box style")); }
        if candidate.border_box != full.border_box { return Some(format!("{label}: border box")); }
        if candidate.content_box != full.content_box { return Some(format!("{label}: content box")); }
        if format!("{:?}", candidate.lines) != format!("{:?}", full.lines) {
            return Some(format!("{label}: line fragments"));
        }
        if let (Some(a), Some(b)) = (children(candidate), children(full)) {
            if a.len() != b.len() { return Some(format!("{label}: child count")); }
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                if let Some(mismatch) = walk(a, b, &format!("{path}/{index}")) { return Some(mismatch); }
            }
        } else if std::mem::discriminant(&candidate.flow) != std::mem::discriminant(&full.flow) {
            return Some(format!("{label}: flow kind"));
        }
        if candidate.positioned.len() != full.positioned.len() {
            return Some(format!("{label}: positioned count"));
        }
        for (index, (a, b)) in candidate.positioned.iter().zip(&full.positioned).enumerate() {
            if let Some(mismatch) = walk(a, b, &format!("{path}/positioned/{index}")) { return Some(mismatch); }
        }
        // Includes overflow, float state, inline fragments and private box
        // metadata that have no dedicated comparison API yet.
        Some(format!("{label}: remaining geometry or fragments"))
    }
    walk(candidate, full, "root")
}

#[derive(Clone)]
pub(crate) struct PaintSnapshot {
    items: Vec<PaintItem>,
    transforms: Vec<TransformNode>,
    clips: Vec<ClipNode>,
    effects: Vec<EffectNode>,
    scrolls: Vec<ScrollNode>,
}

impl PaintSnapshot {
    pub(crate) fn from_view(view: &DisplayListView<'_>) -> Self {
        Self {
            items: view.items().to_vec(),
            transforms: view.transforms().to_vec(),
            clips: view.clips().to_vec(),
            effects: view.effects().to_vec(),
            scrolls: view.scrolls().to_vec(),
        }
    }

    pub(crate) fn first_mismatch(&self, full: &Self) -> Option<String> {
        if self.items.len() != full.items.len() {
            return Some(format!("paint item count {} != {}", self.items.len(), full.items.len()));
        }
        for (index, (candidate, full)) in self.items.iter().zip(&full.items).enumerate() {
            if candidate != full { return Some(format!("paint item {index}: {candidate:?} != {full:?}")); }
        }
        if self.transforms != full.transforms { return Some("paint transform tree".into()); }
        if self.clips != full.clips { return Some("paint clip tree".into()); }
        if self.effects != full.effects { return Some("paint effect tree".into()); }
        if self.scrolls != full.scrolls { return Some("paint scroll tree".into()); }
        None
    }
}

pub(crate) struct CandidateFrame {
    pub(crate) pixels: Surface,
    pub(crate) paint: PaintSnapshot,
}

/// Builds a fresh compositor frame without touching the full-paint reference.
/// The DOM borrow held by the caller freezes script-visible state for both.
pub(crate) fn compositor_candidate(
    root: &LayoutBox,
    width: u32,
    height: u32,
    text: &dyn TextSystem,
    paint: &PagePaint<'_>,
) -> Result<CandidateFrame, String> {
    let view = DisplayListView::new(root, width, height, text, paint, &NoControls);
    let paint = PaintSnapshot::from_view(&view);
    let source = PaintViewSource::new(&view);
    let mut compositor = source.compositor(width, height).map_err(|error| error.to_string())?;
    let frame = source.build_frame(&mut compositor, width, height)
        .map_err(|error| error.to_string())?;
    let pixels = ReferenceCpuPresenter::default().present(&frame)
        .map_err(|error| format!("{error:?}"))?;
    Ok(CandidateFrame { pixels, paint })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_oracle_names_the_first_changed_property() {
        let dom = amazing_html::parse("<div id=sample>ok</div>");
        let green = amazing_css::parse_stylesheet("#sample { color: green }");
        let red = amazing_css::parse_stylesheet("#sample { color: red }");
        let candidate = amazing_css::compute(&dom, &[green]);
        let full = amazing_css::compute(&dom, &[red]);
        assert!(first_style_mismatch(&dom, &candidate, &full)
            .is_some_and(|message| message.ends_with(": color")));
    }

    #[test]
    fn first_pixel_reports_coordinates_and_both_colors() {
        let mut candidate = Surface::new(3, 2);
        let mut full = candidate.clone();
        candidate.set_pixel(2, 1, [1, 2, 3, 4]);
        full.set_pixel(2, 1, [5, 6, 7, 8]);
        assert_eq!(first_pixel_mismatch(&candidate, &full), Some(PixelMismatch {
            x: 2, y: 1, candidate: [1, 2, 3, 4], full: [5, 6, 7, 8],
        }));
    }
}
