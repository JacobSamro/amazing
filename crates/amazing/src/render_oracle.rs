//! Diagnostic compositor comparison at one frozen rendering update.
//!
//! The driver currently rebuilds style, layout and paint after DOM changes.
//! This is the pixel boundary of the incremental oracle: an eventual retained
//! candidate can be substituted without changing the full reference path.
use amazing_compositor::{PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_css::{MutationKind, RenderMutation as StyleMutation, StyleInvalidator, StyledDom, Stylesheet, recompute_roots, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_dom::render_mutation::{RenderMutation, RenderMutationKind};
use amazing_layout::{Flow, LayoutBox};
use amazing_paint::{Surface, controls::NoControls, display_list_view::{DisplayListView, PaintItem, TransformNode, ClipNode, EffectNode, ScrollNode}};
use amazing_text::TextSystem;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::page::PagePaint;

static PARITY_LOSSES: AtomicU64 = AtomicU64::new(0);
static FULL_FALLBACKS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn parity_loss(detail: &str, candidate: Option<&Surface>, full: &Surface) {
    let losses = PARITY_LOSSES.fetch_add(1, Ordering::Relaxed) + 1;
    let fallbacks = FULL_FALLBACKS.fetch_add(1, Ordering::Relaxed) + 1;
    let diff = candidate.and_then(|candidate| write_diff(candidate, full, losses));
    eprintln!("amazing oracle: parity loss: {detail}; presenting full; parity_losses={losses} full_fallbacks={fallbacks} diff={diff:?}");
}

fn write_diff(candidate: &Surface, full: &Surface, serial: u64) -> Option<std::path::PathBuf> {
    if candidate.width != full.width || candidate.height != full.height { return None; }
    let mut diff = Surface::new(full.width, full.height);
    for ((out, a), b) in diff.pixels.chunks_exact_mut(4)
        .zip(candidate.pixels.chunks_exact(4)).zip(full.pixels.chunks_exact(4)) {
        out[0] = a[0].abs_diff(b[0]);
        out[1] = a[1].abs_diff(b[1]);
        out[2] = a[2].abs_diff(b[2]);
        out[3] = 255;
    }
    let path = std::env::temp_dir().join(format!("amazing-oracle-diff-{}-{serial}.ppm", std::process::id()));
    diff.save_ppm(path.to_str()?).ok()?;
    Some(path)
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

#[derive(Default)]
pub(crate) struct RenderCoordinator {
    previous: Option<StyledDom>,
    sheets: Vec<Stylesheet>,
    generations: Vec<u64>,
    invalidator: Option<StyleInvalidator>,
    stylesheet_epoch: u64,
    viewport: (u32, u32),
}

pub(crate) struct StyleCandidate {
    pub(crate) styles: StyledDom,
    pub(crate) mutations: usize,
    pub(crate) recalculated: usize,
    pub(crate) full_fallback: bool,
}

pub(crate) fn report_style_work(candidate: &StyleCandidate) {
    eprintln!("amazing oracle: style mutations={} recalculated={} full_fallback={}",
        candidate.mutations, candidate.recalculated, candidate.full_fallback);
}

impl RenderCoordinator {
    /// Drain only the journal prefix visible at this boundary. Node generation
    /// is checked against the prior cache before allowing a CSS root update.
    pub(crate) fn candidate(&mut self, dom: &mut Dom, sheets: &[Stylesheet], viewport: (u32, u32)) -> StyleCandidate {
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let sheets_changed = self.invalidator.is_none() || self.sheets != sheets;
        let viewport_changed = self.viewport != viewport;
        if sheets_changed {
            self.stylesheet_epoch = self.stylesheet_epoch.wrapping_add(1);
            self.invalidator = Some(StyleInvalidator::new(sheets, self.stylesheet_epoch));
        }
        let full = self.previous.is_none() || sheets_changed || viewport_changed;
        let mutations = journal.len();
        if full {
            return StyleCandidate {
                styles: amazing_css::compute(dom, sheets), mutations,
                recalculated: dom.node_count(), full_fallback: true,
            };
        }
        let changes = journal.iter().map(|entry| self.to_style_mutation(entry)).collect::<Vec<_>>();
        let roots = self.invalidator.as_ref().unwrap().collect(&changes, self.stylesheet_epoch);
        let delta = recompute_roots(dom, self.previous.as_ref().unwrap(), roots);
        StyleCandidate {
            styles: delta.styles,
            mutations,
            recalculated: delta.recalculated.len(),
            full_fallback: delta.full_rebuild,
        }
    }

    pub(crate) fn commit(&mut self, dom: &Dom, sheets: &[Stylesheet], viewport: (u32, u32), full: &StyledDom) {
        self.previous = Some(full.clone());
        self.sheets = sheets.to_vec();
        self.viewport = viewport;
        self.generations = (0..dom.node_count()).map(|index|
            dom.node_generation(NodeIdx(index as u32)).expect("arena node").generation).collect();
    }

    fn to_style_mutation(&self, entry: &RenderMutation) -> StyleMutation {
        let stable = self.generations.get(entry.target.node.0 as usize)
            == Some(&entry.target.generation);
        let kind = match &entry.kind {
            RenderMutationKind::Attribute { namespace_uri: None, local_name, old_value, new_value, .. } => {
                match (old_value.as_deref().map(String::from_utf16).transpose(),
                       new_value.as_deref().map(String::from_utf16).transpose()) {
                    (Ok(old), Ok(new)) => MutationKind::Attribute {
                        name: local_name.clone(), old, new,
                    },
                    _ => MutationKind::Unknown,
                }
            }
            RenderMutationKind::State { name, old_value, new_value } => MutationKind::State {
                pseudo: (*name).into(), old: *old_value, new: *new_value,
            },
            RenderMutationKind::Tree { .. } => MutationKind::Tree,
            RenderMutationKind::CharacterData { .. } => MutationKind::Text,
            RenderMutationKind::SlotAssignment { .. } => MutationKind::ShadowSlot,
            _ => MutationKind::Unknown,
        };
        StyleMutation {
            node: entry.target.node,
            generation: if stable { 0 } else { entry.target.generation },
            kind,
        }
    }
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
    fn journal_attribute_recalculates_a_stable_node_without_full_style() {
        let mut dom = amazing_html::parse("<div id=sample>ok</div>");
        let node = (0..dom.node_count()).map(|index| NodeIdx(index as u32))
            .find(|node| dom.name(*node) == Some("div")).unwrap();
        let sheets = [amazing_css::parse_stylesheet(".changed { color: red }")];
        let mut coordinator = RenderCoordinator::default();
        let first = coordinator.candidate(&mut dom, &sheets, (64, 64));
        assert!(first.full_fallback);
        coordinator.commit(&dom, &sheets, (64, 64), &first.styles);
        dom.set_attribute(node, "class", "changed").unwrap();
        let changed = coordinator.candidate(&mut dom, &sheets, (64, 64));
        assert_eq!(changed.mutations, 1);
        assert!(!changed.full_fallback);
        assert!(changed.recalculated > 0);
        let full = amazing_css::compute(&dom, &sheets);
        assert_eq!(first_style_mismatch(&dom, &changed.styles, &full), None);
        coordinator.commit(&dom, &sheets, (64, 64), &full);
        dom.create_element(node, "span");
        let structural = coordinator.candidate(&mut dom, &sheets, (64, 64));
        assert!(structural.full_fallback);
        assert_eq!(first_style_mismatch(&dom, &structural.styles, &amazing_css::compute(&dom, &sheets)), None);
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
        let path = write_diff(&candidate, &full, 0).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.ends_with(&[4, 4, 4]));
        std::fs::remove_file(path).unwrap();
    }
}
