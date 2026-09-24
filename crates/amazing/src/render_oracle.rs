//! Diagnostic compositor comparison at one frozen rendering update.
//!
//! The driver retains CSS, layout and compositor state across rendering updates.
//! A fresh style, layout and paint result at the same frozen DOM epoch remains
//! the pixel reference until the retained path has exact parity.
use amazing_compositor::{Compositor, PaintDelta, PaintItemKey, PaintViewSource, Presenter, ReferenceCpuPresenter};
use amazing_css::{AttributeStyleUpdate, CachedBoxSize, CompositorStyleDelta, MutationKind, RenderMutation as StyleMutation, StyleChangeKind, StyleDelta, StyleInvalidator, StyledDom, Stylesheet, recompute_roots, selectors::PseudoElement};
use amazing_dom::{Dom, NodeIdx};
use amazing_dom::render_mutation::{RenderMutation, RenderMutationKind};
use amazing_layout::{Flow, LayoutBox, retained::{GeometryDelta, InputChanges, LayoutTree, UpdateOutcome}};
use amazing_paint::{Surface, controls::NoControls, display_list_view::{DisplayListView, PaintItem, PaintViewUpdate, TransformNode, ClipNode, EffectNode, ScrollNode}};
use amazing_text::TextSystem;
use std::collections::{BTreeMap, BTreeSet};
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
    layout: Option<LayoutTree>,
    paint_generations: BTreeMap<NodeIdx, u64>,
    session: Option<RetainedCompositorSession>,
    last_work: FrameWork,
}

/// The compositor and its presenter have one scene lifetime. A full-painter
/// fallback discards both so a later property update cannot target stale IDs.
struct RetainedCompositorSession {
    compositor: Compositor,
    presenter: ReferenceCpuPresenter,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct FrameWork {
    pub(crate) style_rebuild_calls: usize,
    pub(crate) layout_update_calls: usize,
    pub(crate) paint_view_rebuild_calls: usize,
    pub(crate) style_recalculated: usize,
    pub(crate) layout_dirty: usize,
    pub(crate) layout_full: bool,
    pub(crate) paint_changed: usize,
    pub(crate) paint_full: bool,
    pub(crate) tile_uploads: usize,
    pub(crate) tiles_dirtied: usize,
}

pub(crate) struct StyleCandidate {
    pub(crate) styles: StyledDom,
    pub(crate) mutations: usize,
    pub(crate) recalculated: usize,
    pub(crate) full_fallback: bool,
    pub(crate) changed: BTreeMap<NodeIdx, Vec<String>>,
    pub(crate) change_kinds: BTreeMap<NodeIdx, StyleChangeKind>,
    pub(crate) inputs: InputChanges,
}

pub(crate) fn report_style_work(candidate: &StyleCandidate) {
    eprintln!("amazing oracle: style mutations={} recalculated={} full_fallback={}",
        candidate.mutations, candidate.recalculated, candidate.full_fallback);
}

impl RenderCoordinator {
    /// The rendering update offers a retained scene the journal before any
    /// style, layout or paint work. A rejected classification follows the
    /// ordinary full rendering path with the same journal intact.
    pub(crate) fn try_property_only(
        &mut self,
        dom: &Dom,
        sheets: &[Stylesheet],
        viewport: (u32, u32),
        journal: &[RenderMutation],
    ) -> Option<Surface> {
        let _delta = self.classify_css_value_deltas(dom, sheets, viewport, journal)?;
        // A classified value still needs COMP-11's validated cached-frame
        // adapter before the full pipeline can be skipped.
        None
    }

    fn classify_css_value_deltas(&self, dom: &Dom, sheets: &[Stylesheet],
        viewport: (u32, u32), journal: &[RenderMutation]) -> Option<CompositorStyleDelta> {
        if self.session.is_none() || self.sheets != sheets || self.viewport != viewport {
            return None;
        }
        let [entry] = journal else { return None; };
        if dom.node_generation(entry.target.node) != Some(entry.target.generation)
            || self.generations.get(entry.target.node.0 as usize) != Some(&entry.target.generation)
        { return None; }
        let RenderMutationKind::Attribute { namespace_uri: None, local_name, old_value, new_value, .. } = &entry.kind
            else { return None; };
        // CSS-13's fast classifier currently resolves inline style edits. A
        // class edit returns Recomputed, so leave it for the normal candidate
        // path without doing that style work twice.
        if local_name != "style" { return None; }
        let old = old_value.as_ref().map(|value| String::from_utf16(value)).transpose().ok()?;
        let new = new_value.as_ref().map(|value| String::from_utf16(value)).transpose().ok()?;
        let box_size = self.layout.as_ref().and_then(|layout| layout_box_size(layout.root(), entry.target.node));
        let result = self.invalidator.as_ref()?.update_attribute_with_box(
            dom, self.previous.as_ref()?, entry.target.node, local_name,
            old.as_deref(), new.as_deref(), self.stylesheet_epoch, box_size,
        );
        match result {
            AttributeStyleUpdate::CompositorOnly(delta) => Some(delta),
            AttributeStyleUpdate::Recomputed(_) => None,
        }
    }

    /// Compare the previous paint view with the next full view before mutating
    /// the retained layout tree. The full view is also the independent oracle
    /// for the layout update below.
    pub(crate) fn update_layout_and_paint(
        &mut self,
        dom: &Dom,
        styled: &StyledDom,
        candidate: StyleCandidate,
        style_divergence: bool,
        full_root: &LayoutBox,
        width: u32,
        height: u32,
        text: &dyn TextSystem,
        paint: &PagePaint<'_>,
        viewport_width: f32,
        viewport_height: Option<f32>,
    ) -> (GeometryDelta, Option<PaintViewUpdate>) {
        self.last_work = FrameWork { style_rebuild_calls: 1, layout_update_calls: 1,
            style_recalculated: candidate.recalculated, ..FrameWork::default() };
        let resource_change = !candidate.inputs.text.is_empty() || !candidate.inputs.replaced.is_empty()
            || candidate.changed.values().flatten().any(|name| matches!(name.as_str(),
                "background-image" | "list-style-image" | "content" | "font-family" | "font-size"));
        let paint_update = self.layout.as_ref().and_then(|old| {
            if resource_change { return None; }
            let old_paint = PagePaint { media: paint.media, dom, styles: old.styles() };
            let before = DisplayListView::with_node_generations(
                old.root(), width, height, text, &old_paint, &NoControls,
                &|node| self.paint_generations.get(&node).copied(),
            );
            let after = DisplayListView::with_dom_generations(
                full_root, width, height, text, paint, &NoControls, dom,
            );
            Some(before.update(&after))
        });
        if paint_update.is_some() { self.last_work.paint_view_rebuild_calls += 2; }
        let delta = StyleDelta {
            styles: styled.clone(),
            changed: candidate.changed,
            change_kinds: candidate.change_kinds,
            recalculated: Vec::new(),
            full_rebuild: candidate.full_fallback || style_divergence
                || first_style_mismatch(dom, &candidate.styles, styled).is_some(),
        };
        let geometry = if let Some(layout) = self.layout.as_mut() {
            layout.update(dom, delta, &candidate.inputs, text, paint.media,
                viewport_width, viewport_height)
        } else {
            self.layout = Some(LayoutTree::new(dom, styled.clone(), text, paint.media,
                viewport_width, viewport_height));
            GeometryDelta { outcome: UpdateOutcome::RequiresFullRebuild("initial layout"),
                dirty: Vec::new(), changed: Vec::new() }
        };
        self.last_work.layout_dirty = geometry.dirty.len();
        self.last_work.layout_full = !matches!(geometry.outcome, UpdateOutcome::Retained);
        self.last_work.paint_changed = match &paint_update {
            Some(PaintViewUpdate::Delta(delta)) => delta.changes.len(),
            _ => 0,
        };
        self.last_work.paint_full = !matches!(paint_update, Some(PaintViewUpdate::Delta(_)));
        self.paint_generations = (0..dom.node_count()).filter_map(|index| {
            let node = NodeIdx(index as u32);
            dom.node_generation(node).map(|generation| (node, generation))
        }).collect();
        (geometry, paint_update)
    }

    pub(crate) fn last_work(&self) -> FrameWork { self.last_work }

    pub(crate) fn reset_scene(&mut self) {
        self.session = None;
    }

    pub(crate) fn layout_root(&self) -> &LayoutBox {
        self.layout.as_ref().expect("oracle layout initialized").root()
    }

    /// Drain only the journal prefix visible at this boundary. Node generation
    /// is checked against the prior cache before allowing a CSS root update.
    pub(crate) fn candidate(&mut self, dom: &Dom, sheets: &[Stylesheet], viewport: (u32, u32), journal: Vec<RenderMutation>) -> StyleCandidate {
        let sheets_changed = self.invalidator.is_none() || self.sheets != sheets;
        let viewport_changed = self.viewport != viewport;
        if sheets_changed {
            self.stylesheet_epoch = self.stylesheet_epoch.wrapping_add(1);
            self.invalidator = Some(StyleInvalidator::new(sheets, self.stylesheet_epoch));
        }
        let full = self.previous.is_none() || sheets_changed || viewport_changed;
        let mutations = journal.len();
        let inputs = InputChanges {
            text: journal.iter().filter_map(|entry| matches!(entry.kind, RenderMutationKind::CharacterData { .. })
                .then_some(entry.target.node)).collect(),
            replaced: journal.iter().filter_map(|entry| match &entry.kind {
                RenderMutationKind::Attribute { local_name, .. } if matches!(local_name.as_str(), "src" | "srcset" | "width" | "height") => Some(entry.target.node),
                _ => None,
            }).collect(),
        };
        if full {
            return StyleCandidate {
                styles: amazing_css::compute(dom, sheets), mutations,
                recalculated: dom.node_count(), full_fallback: true,
                changed: BTreeMap::new(), change_kinds: BTreeMap::new(), inputs,
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
            changed: delta.changed,
            change_kinds: delta.change_kinds,
            inputs,
        }
    }

    pub(crate) fn commit(&mut self, dom: &Dom, sheets: &[Stylesheet], viewport: (u32, u32), full: &StyledDom) {
        self.previous = Some(full.clone());
        self.sheets = sheets.to_vec();
        self.viewport = viewport;
        self.generations = (0..dom.node_count()).map(|index|
            // Free slots have no generation (None); they never match a live
            // node's key, so 0 is a safe placeholder.
            dom.node_generation(NodeIdx(index as u32)).unwrap_or(0)).collect();
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

fn layout_box_size(root: &LayoutBox, node: NodeIdx) -> Option<CachedBoxSize> {
    if root.node == Some(node) {
        return Some(CachedBoxSize { width: root.border_box.w, height: root.border_box.h });
    }
    let children = match &root.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children.as_slice(),
        _ => &[],
    };
    children.iter().chain(&root.positioned).find_map(|child| layout_box_size(child, node))
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
    pub(crate) uploads: usize,
    pub(crate) dirtied_tiles: usize,
}

/// Updates the retained compositor when the paint delta is safe; otherwise
/// creates a fresh scene. The full-paint oracle remains independent.
impl RenderCoordinator {
pub(crate) fn compositor_candidate(
    &mut self,
    dom: &Dom,
    width: u32,
    height: u32,
    text: &dyn TextSystem,
    paint: &PagePaint<'_>,
    paint_update: Option<&PaintViewUpdate>,
) -> Result<CandidateFrame, String> {
    self.last_work.paint_view_rebuild_calls += 1;
    let root = self.layout.as_ref().expect("oracle layout initialized").root();
    let view = DisplayListView::with_dom_generations(root, width, height, text, paint, &NoControls, dom);
    let paint = PaintSnapshot::from_view(&view);
    let source = PaintViewSource::new(&view);
    let mut dirtied_tiles = 0;
    let retained = match (self.session.as_mut(), paint_update) {
        (Some(session), Some(PaintViewUpdate::Delta(delta))) if delta.retired_resources.is_empty() => {
            let delta = PaintDelta { changes: delta.changes.iter().map(|change|
                amazing_compositor::PaintItemChange {
                    key: PaintItemKey(change.compositor_id), item_index: change.item_index,
                    old_bounds: amazing_compositor::Rect::new(change.old_bounds.x, change.old_bounds.y,
                        change.old_bounds.width, change.old_bounds.height),
                    new_bounds: amazing_compositor::Rect::new(change.new_bounds.x, change.new_bounds.y,
                        change.new_bounds.width, change.new_bounds.height),
                }).collect() };
            match session.compositor.apply_paint_delta(&delta) {
                Ok(count) => { dirtied_tiles = count; true }
                Err(error) => { eprintln!("amazing oracle: paint delta fallback: {error:?}"); false }
            }
        }
        _ => false,
    };
    if !retained {
        self.last_work.paint_full = true;
        let mut fresh = source.compositor(width, height).map_err(|error| error.to_string())?;
        if let Some(keys) = view.items().iter().map(|item| item.key.map(|key| PaintItemKey(key.compositor_id())))
            .collect::<Option<Vec<_>>>() {
            let _ = fresh.bind_paint_item_keys(&keys);
        }
        self.session = Some(RetainedCompositorSession {
            compositor: fresh, presenter: ReferenceCpuPresenter::default(),
        });
    }
    let session = self.session.as_mut().expect("oracle compositor initialized");
    let frame = source.build_frame(&mut session.compositor, width, height)
        .map_err(|error| error.to_string())?;
    let uploads = frame.resources.uploads.len();
    self.last_work.tile_uploads = uploads;
    self.last_work.tiles_dirtied = dirtied_tiles;
    let pixels = session.presenter.present(&frame)
        .map_err(|error| format!("{error:?}"))?;
    Ok(CandidateFrame { pixels, paint, uploads, dirtied_tiles })
}
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
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let first = coordinator.candidate(&dom, &sheets, (64, 64), journal);
        assert!(first.full_fallback);
        coordinator.commit(&dom, &sheets, (64, 64), &first.styles);
        dom.set_attribute(node, "class", "changed").unwrap();
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let changed = coordinator.candidate(&dom, &sheets, (64, 64), journal);
        assert_eq!(changed.mutations, 1);
        assert!(!changed.full_fallback);
        assert!(changed.recalculated > 0);
        let full = amazing_css::compute(&dom, &sheets);
        assert_eq!(first_style_mismatch(&dom, &changed.styles, &full), None);
        coordinator.commit(&dom, &sheets, (64, 64), &full);
        dom.create_element(node, "span");
        let epoch = dom.render_mutation_epoch();
        let journal = dom.take_render_mutations(epoch);
        let structural = coordinator.candidate(&dom, &sheets, (64, 64), journal);
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
