//! Retained layout boundary for the rendering-update coordinator (ADR 0010).
//! Only updates proved to preserve box geometry use the in-place path. Other
//! changes carry explicit fallback reasons while refreshing the retained tree.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use amazing_css::{StyleDelta, StyledDom};
use amazing_dom::{Dom, NodeIdx};
use amazing_text::TextSystem;

use crate::{layout_styled_document, Flow, LayoutBox, Rect, ReplacedElementProvider};

/// The strongest layout effect of a changed input. Ordering is significant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirtySeverity {
    Repaint,
    Reposition,
    Relayout,
    RebuildBoxes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoxKey {
    pub node: NodeIdx,
    pub generation: u64,
    pub source_order: Option<u64>,
    pub principal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirtyBox {
    pub node: NodeIdx,
    pub generation: u64,
    pub severity: DirtySeverity,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxGeometry {
    pub border: Rect,
    pub content: Rect,
    /// Union of the box, positioned descendants and inline fragments.
    pub overflow: Rect,
    /// Current layout hit region; paint may further clip this rectangle.
    pub hit: Rect,
    /// Fingerprint of exact line and fragment placement inside this box.
    pub fragments: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometryChange {
    pub key: BoxKey,
    /// Index among boxes with the same DOM/generation/source-order key.
    pub ordinal: usize,
    pub old: Option<BoxGeometry>,
    pub new: Option<BoxGeometry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    Retained,
    RequiresFullRebuild(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeometryDelta {
    pub outcome: UpdateOutcome,
    pub dirty: Vec<DirtyBox>,
    pub changed: Vec<GeometryChange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoxRef {
    pub key: BoxKey,
    pub ordinal: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutDamage {
    pub box_ref: BoxRef,
    pub before: Option<BoxGeometry>,
    pub after: Option<BoxGeometry>,
    /// The union of old and new visual overflow, including moved fragments.
    pub rect: Rect,
}

#[derive(Debug, Default)]
pub struct InputChanges {
    pub text: Vec<NodeIdx>,
    pub replaced: Vec<NodeIdx>,
}

/// Owns the last laid-out boxes and the DOM slot generations they belong to.
/// A caller uses `root()` for paint after every update, including fallbacks.
#[derive(Debug)]
pub struct LayoutTree {
    root: LayoutBox,
    styles: StyledDom,
    generations: BTreeMap<NodeIdx, u64>,
    geometry: BTreeMap<BoxKey, Vec<BoxGeometry>>,
    dirty: BTreeMap<BoxKey, DirtySeverity>,
    damage: Vec<LayoutDamage>,
    viewport: (f32, Option<f32>),
}

impl LayoutTree {
    pub fn new(
        dom: &Dom,
        styles: StyledDom,
        text: &dyn TextSystem,
        replaced: &dyn ReplacedElementProvider,
        viewport_width: f32,
        viewport_height: Option<f32>,
    ) -> Self {
        let root = layout_styled_document(
            dom,
            &styles,
            text,
            replaced,
            viewport_width,
            viewport_height,
        );
        let generations = generations(dom);
        let geometry = collect_geometry(&root, &generations);
        Self {
            root,
            styles,
            generations,
            geometry,
            dirty: BTreeMap::new(),
            damage: Vec::new(),
            viewport: (viewport_width, viewport_height),
        }
    }

    pub fn root(&self) -> &LayoutBox {
        &self.root
    }

    pub fn styles(&self) -> &StyledDom {
        &self.styles
    }

    /// Dirty bits from the most recent update, keyed by DOM slot generation.
    pub fn dirty_boxes(&self) -> &BTreeMap<BoxKey, DirtySeverity> {
        &self.dirty
    }

    /// Borrow the last update's bounded paint damage. Each entry owns both
    /// geometries; `box_ref` resolves into this tree's current root when
    /// `after` is present, or removes the old paint item otherwise.
    pub fn damage(&self) -> &[LayoutDamage] {
        &self.damage
    }

    /// Resolve an affected box in the current tree. A removed box has no
    /// current reference; its damage entry still owns its previous geometry.
    pub fn box_for_ref(&self, reference: BoxRef) -> Option<&LayoutBox> {
        if self.generations.get(&reference.key.node) != Some(&reference.key.generation) {
            return None;
        }
        let mut ordinal = 0;
        find_box_ref(&self.root, reference, &mut ordinal)
    }

    pub fn update(
        &mut self,
        dom: &Dom,
        style_delta: StyleDelta,
        inputs: &InputChanges,
        text: &dyn TextSystem,
        replaced: &dyn ReplacedElementProvider,
        viewport_width: f32,
        viewport_height: Option<f32>,
    ) -> GeometryDelta {
        let next_generations = generations(dom);
        if !cfg!(debug_assertions)
            && std::env::var_os("AMAZING_LAYOUT_ORACLE").is_none()
            && !style_delta.full_rebuild
            && style_delta.changed.is_empty()
            && inputs.text.is_empty()
            && inputs.replaced.is_empty()
            && self.generations == next_generations
            && self.viewport == (viewport_width, viewport_height)
        {
            self.styles = style_delta.styles;
            self.dirty.clear();
            self.damage.clear();
            return GeometryDelta {
                outcome: UpdateOutcome::Retained,
                dirty: Vec::new(),
                changed: Vec::new(),
            };
        }
        let old_geometry = self.geometry.clone();
        let mut dirty = BTreeMap::<NodeIdx, DirtySeverity>::new();
        for (&node, properties) in &style_delta.changed {
            let severity = properties
                .iter()
                .map(|name| classify_property(name))
                .max()
                .unwrap_or(DirtySeverity::RebuildBoxes);
            mark_dirty(dom, node, severity, &mut dirty, &BTreeSet::new());
        }
        for &node in inputs.text.iter().chain(&inputs.replaced) {
            mark_dirty(
                dom,
                node,
                DirtySeverity::Relayout,
                &mut dirty,
                &BTreeSet::new(),
            );
        }
        let reused_generation = self.generations == next_generations;
        let same_viewport = self.viewport == (viewport_width, viewport_height);
        let only_opacity = inputs.text.is_empty()
            && inputs.replaced.is_empty()
            && style_delta.changed.values().all(|properties| {
                !properties.is_empty() && properties.iter().all(|name| name == "opacity")
            });
        let retained_relayout = !style_delta.full_rebuild
            && reused_generation
            && same_viewport
            && inputs.text.is_empty()
            && inputs.replaced.is_empty()
            && try_relayout_contained(&mut self.root, dom, &self.styles, &style_delta, text);
        let mut reason = if style_delta.full_rebuild {
            Some("style requested a full rebuild")
        } else if !reused_generation {
            Some("DOM node generation changed")
        } else if !same_viewport {
            Some("viewport changed")
        } else if !inputs.text.is_empty() || !inputs.replaced.is_empty() {
            Some("text or replaced intrinsic input changed")
        } else if !only_opacity && !retained_relayout && !style_delta.changed.is_empty() {
            Some("layout or box style changed")
        } else {
            None
        };
        if reason.is_none() && !retained_relayout {
            for &node in style_delta.changed.keys() {
                let Some(style) = style_delta.styles.get(node) else {
                    reason = Some("changed style has no live box");
                    break;
                };
                if !replace_box_style(&mut self.root, node, style) {
                    reason = Some("changed style has no retained principal box");
                    break;
                }
            }
        }
        if let Some(reason) = reason {
            self.root = layout_styled_document(
                dom,
                &style_delta.styles,
                text,
                replaced,
                viewport_width,
                viewport_height,
            );
            self.styles = style_delta.styles;
            self.generations = next_generations;
            self.viewport = (viewport_width, viewport_height);
            let new_geometry = collect_geometry(&self.root, &self.generations);
            self.geometry = new_geometry.clone();
            let boundaries = stable_containment_boundaries(
                dom,
                &self.styles,
                &style_delta.changed,
                &old_geometry,
                &new_geometry,
                &self.generations,
            );
            dirty.clear();
            for (&node, properties) in &style_delta.changed {
                let severity = properties
                    .iter()
                    .map(|name| classify_property(name))
                    .max()
                    .unwrap_or(DirtySeverity::RebuildBoxes);
                mark_dirty(dom, node, severity, &mut dirty, &boundaries);
            }
            for &node in inputs.text.iter().chain(&inputs.replaced) {
                mark_dirty(dom, node, DirtySeverity::Relayout, &mut dirty, &boundaries);
            }
            let dirty = dirty_boxes(dom, dirty);
            self.dirty = dirty_keys(&dirty, &new_geometry);
            self.damage = collect_damage(&old_geometry, &new_geometry, &self.dirty);
            return GeometryDelta {
                outcome: UpdateOutcome::RequiresFullRebuild(reason),
                dirty,
                changed: compare_geometry(old_geometry, new_geometry),
            };
        }
        self.styles = style_delta.styles;
        let mut outcome = UpdateOutcome::Retained;
        // Debug oracle: geometry, overflow and hit regions must match a fresh
        // layout. The full debug representation also catches line/fragment
        // differences that have the same bounding rectangles.
        if cfg!(debug_assertions) || std::env::var_os("AMAZING_LAYOUT_ORACLE").is_some() {
            let full = layout_styled_document(
                dom,
                &self.styles,
                text,
                replaced,
                viewport_width,
                viewport_height,
            );
            if collect_geometry(&self.root, &self.generations)
                != collect_geometry(&full, &next_generations)
                || format!("{:?}", self.root) != format!("{full:?}")
            {
                self.root = full;
                outcome = UpdateOutcome::RequiresFullRebuild("layout parity oracle mismatch");
            }
        }
        let new_geometry = collect_geometry(&self.root, &self.generations);
        self.geometry = new_geometry.clone();
        let dirty = dirty_boxes(dom, dirty);
        self.dirty = dirty_keys(&dirty, &new_geometry);
        self.damage = collect_damage(&old_geometry, &new_geometry, &self.dirty);
        GeometryDelta {
            outcome,
            dirty,
            changed: compare_geometry(old_geometry, new_geometry),
        }
    }
}

fn find_box_ref<'a>(box_: &'a LayoutBox, reference: BoxRef, ordinal: &mut usize) -> Option<&'a LayoutBox> {
    for child in &box_.positioned {
        if let Some(found) = find_box_ref(child, reference, ordinal) { return Some(found); }
    }
    if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
        for child in children {
            if let Some(found) = find_box_ref(child, reference, ordinal) { return Some(found); }
        }
    }
    if box_.node == Some(reference.key.node)
        && box_.source_order == reference.key.source_order
        && (box_.generated_pseudo.is_none() == reference.key.principal)
    {
        if *ordinal == reference.ordinal { return Some(box_); }
        *ordinal += 1;
    }
    None
}

/// The narrow fast path relayouts one existing block formatting context. Its
/// border and margins must stay fixed, so siblings and ancestors keep their
/// placement. All other dependencies use the explicit full-layout path.
fn try_relayout_contained(
    root: &mut LayoutBox,
    dom: &Dom,
    old_styles: &StyledDom,
    delta: &StyleDelta,
    text: &dyn TextSystem,
) -> bool {
    let Some((&changed, properties)) = delta.changed.iter().next() else { return false };
    if delta.changed.len() != 1 || properties.is_empty() || !properties.iter().all(|property| {
        matches!(property.as_str(), "width" | "height" | "min-width" | "max-width"
            | "min-height" | "max-height" | "padding-top" | "padding-right"
            | "padding-bottom" | "padding-left" | "margin-top" | "margin-right"
            | "margin-bottom" | "margin-left")
    }) { return false; }
    let mut ancestor = Some(changed);
    let mut boundary = None;
    while let Some(node) = ancestor {
        let contained = |styles: &StyledDom| styles.get(node)
            .and_then(|style| style.get("contain"))
            .is_some_and(|value| value.split_whitespace()
                .any(|token| matches!(token, "layout" | "content" | "strict")));
        if contained(old_styles) && contained(&delta.styles) {
            boundary = Some(node);
            break;
        }
        ancestor = dom.get(node).parent;
    }
    let Some(boundary) = boundary else { return false };
    fn relayout(
        box_: &mut LayoutBox, boundary: NodeIdx, changed: NodeIdx,
        styles: &StyledDom, text: &dyn TextSystem, parent_width: Option<f32>,
    ) -> Option<bool> {
        if box_.node == Some(boundary) && box_.generated_pseudo.is_none() {
            let width = parent_width?;
            if box_.display != crate::Display::Block || box_.style.get("display") != Some("flow-root")
                || box_.table_wrapper
                || !box_.positioned.is_empty() || box_.style.get("float").is_some_and(|v| v != "none")
            { return Some(false); }
            let old_border = box_.border_box;
            let old_margin = box_.margin;
            if !replace_box_style(box_, changed, styles.get(changed)?) { return Some(false); }
            let x = old_border.x - old_margin.left.unwrap_or(0.0);
            let y = old_border.y - old_margin.top.unwrap_or(0.0);
            crate::block::layout_block_with_text(
                box_, x, y, width, &crate::font::FontSource::fallback(), text,
            );
            let margin = box_.margin;
            return Some(box_.border_box == old_border
                && (margin.top, margin.right, margin.bottom, margin.left)
                    == (old_margin.top, old_margin.right, old_margin.bottom, old_margin.left));
        }
        let width = box_.content_box.w;
        if let Flow::Blocks(children) = &mut box_.flow {
            for child in children {
                if let Some(result) = relayout(child, boundary, changed, styles, text, Some(width)) {
                    return Some(result);
                }
            }
        }
        None
    }
    relayout(root, boundary, changed, &delta.styles, text, None).unwrap_or(false)
}

fn classify_property(name: &str) -> DirtySeverity {
    match name {
        "opacity" | "color" | "background-color" | "outline-color" | "visibility" | "z-index"
        | "box-shadow" | "text-shadow" => DirtySeverity::Repaint,
        "top" | "right" | "bottom" | "left" | "transform" => DirtySeverity::Reposition,
        "display"
        | "position"
        | "float"
        | "content"
        | "quotes"
        | "counter-reset"
        | "counter-increment"
        | "counter-set"
        | "list-style"
        | "list-style-type"
        | "list-style-image"
        | "list-style-position"
        | "::Before"
        | "::After"
        | "::Marker"
        | "<computed-state>" => DirtySeverity::RebuildBoxes,
        _ if name.starts_with("::") => DirtySeverity::RebuildBoxes,
        _ => DirtySeverity::Relayout,
    }
}

fn generations(dom: &Dom) -> BTreeMap<NodeIdx, u64> {
    (0..dom.node_count())
        .filter_map(|index| {
            let node = NodeIdx(index as u32);
            dom.node_generation(node)
                .map(|generation| (node, generation))
        })
        .collect()
}

fn mark_dirty(
    dom: &Dom,
    node: NodeIdx,
    severity: DirtySeverity,
    dirty: &mut BTreeMap<NodeIdx, DirtySeverity>,
    boundaries: &BTreeSet<NodeIdx>,
) {
    let mut current = Some(node);
    let mut seen = BTreeSet::new();
    while let Some(id) = current.filter(|id| dom.node_generation(*id).is_some()) {
        if !seen.insert(id) {
            break;
        }
        let entry = dirty.entry(id).or_insert(severity);
        *entry = (*entry).max(severity);
        if severity == DirtySeverity::Repaint || boundaries.contains(&id) {
            break;
        }
        // A containing block may stop propagation only after its outgoing
        // size and overflow are checked. Until then the root is the safe edge.
        current = dom.get(id).parent;
    }
}

fn stable_containment_boundaries(
    dom: &Dom,
    styles: &StyledDom,
    changed: &BTreeMap<NodeIdx, Vec<String>>,
    before: &BTreeMap<BoxKey, Vec<BoxGeometry>>,
    after: &BTreeMap<BoxKey, Vec<BoxGeometry>>,
    generations: &BTreeMap<NodeIdx, u64>,
) -> BTreeSet<NodeIdx> {
    generations
        .iter()
        .filter_map(|(&node, &generation)| {
            if changed.contains_key(&node) {
                return None;
            }
            let contain = styles.get(node)?.get("contain")?;
            if !contain
                .split_whitespace()
                .any(|token| matches!(token, "layout" | "content" | "strict"))
            {
                return None;
            }
            let (key, old) = before.iter().find(|(key, _)| {
                key.node == node && key.generation == generation && key.principal
            })?;
            let new = after.get(key)?;
            (old == new && dom.node_generation(node) == Some(generation)).then_some(node)
        })
        .collect()
}

fn dirty_boxes(dom: &Dom, dirty: BTreeMap<NodeIdx, DirtySeverity>) -> Vec<DirtyBox> {
    dirty
        .into_iter()
        .filter_map(|(node, severity)| {
            dom.node_generation(node).map(|generation| DirtyBox {
                node,
                generation,
                severity,
            })
        })
        .collect()
}

fn dirty_keys(
    dirty: &[DirtyBox],
    geometry: &BTreeMap<BoxKey, Vec<BoxGeometry>>,
) -> BTreeMap<BoxKey, DirtySeverity> {
    let mut bits = BTreeMap::new();
    for entry in dirty {
        let mut found = false;
        for key in geometry
            .keys()
            .filter(|key| key.node == entry.node && key.generation == entry.generation)
        {
            bits.insert(*key, entry.severity);
            found = true;
        }
        if !found {
            bits.insert(
                BoxKey {
                    node: entry.node,
                    generation: entry.generation,
                    source_order: None,
                    principal: false,
                },
                entry.severity,
            );
        }
    }
    bits
}

fn replace_box_style(
    root: &mut LayoutBox,
    node: NodeIdx,
    style: &amazing_css::ComputedStyle,
) -> bool {
    let mut found = false;
    if root.node == Some(node) && root.generated_pseudo.is_none() {
        root.style = style.clone();
        found = true;
    }
    for child in &mut root.positioned {
        found |= replace_box_style(child, node, style);
    }
    match &mut root.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
            for child in children {
                found |= replace_box_style(child, node, style);
            }
        }
        _ => {}
    }
    found
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}

fn collect_geometry(
    root: &LayoutBox,
    generations: &BTreeMap<NodeIdx, u64>,
) -> BTreeMap<BoxKey, Vec<BoxGeometry>> {
    fn walk(
        box_: &LayoutBox,
        generations: &BTreeMap<NodeIdx, u64>,
        out: &mut BTreeMap<BoxKey, Vec<BoxGeometry>>,
    ) -> Rect {
        let mut overflow = box_.border_box;
        for child in &box_.positioned {
            overflow = union(overflow, walk(child, generations, out));
        }
        match &box_.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => {
                for child in children {
                    overflow = union(overflow, walk(child, generations, out));
                }
            }
            Flow::Inlines(_) => {
                for line in &box_.lines {
                    for glyph in &line.glyphs {
                        overflow = union(
                            overflow,
                            Rect {
                                x: glyph.x,
                                y: line.y_top,
                                w: glyph.advance_x.abs().max(glyph.font_size),
                                h: glyph.advance_y.abs().max(line.height),
                            },
                        );
                    }
                    for image in &line.replaced {
                        overflow = union(
                            overflow,
                            Rect {
                                x: image.x,
                                y: image.y,
                                w: image.width,
                                h: image.height,
                            },
                        );
                    }
                }
            }
            Flow::Empty => {}
        }
        if let Some(node) = box_.node {
            if let Some(&generation) = generations.get(&node) {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                format!("{:?}", box_.lines).hash(&mut hasher);
                out.entry(BoxKey {
                    node,
                    generation,
                    source_order: box_.source_order,
                    principal: box_.generated_pseudo.is_none(),
                })
                .or_default()
                .push(BoxGeometry {
                    border: box_.border_box,
                    content: box_.content_box,
                    overflow,
                    hit: box_.border_box,
                    fragments: hasher.finish(),
                });
            }
        }
        overflow
    }
    let mut out = BTreeMap::new();
    walk(root, generations, &mut out);
    out
}

fn compare_geometry(
    old: BTreeMap<BoxKey, Vec<BoxGeometry>>,
    new: BTreeMap<BoxKey, Vec<BoxGeometry>>,
) -> Vec<GeometryChange> {
    let keys: BTreeSet<_> = old.keys().chain(new.keys()).copied().collect();
    let mut changes = Vec::new();
    for key in keys {
        let before = old.get(&key);
        let after = new.get(&key);
        for index in 0..before.map_or(0, Vec::len).max(after.map_or(0, Vec::len)) {
            let old = before.and_then(|items| items.get(index)).copied();
            let new = after.and_then(|items| items.get(index)).copied();
            if old != new {
                changes.push(GeometryChange {
                    key,
                    ordinal: index,
                    old,
                    new,
                });
            }
        }
    }
    changes
}

fn collect_damage(
    old: &BTreeMap<BoxKey, Vec<BoxGeometry>>,
    new: &BTreeMap<BoxKey, Vec<BoxGeometry>>,
    dirty: &BTreeMap<BoxKey, DirtySeverity>,
) -> Vec<LayoutDamage> {
    let keys: BTreeSet<_> = old.keys().chain(new.keys()).chain(dirty.keys()).copied().collect();
    let mut damage = Vec::new();
    for key in keys {
        let before = old.get(&key);
        let after = new.get(&key);
        for ordinal in 0..before.map_or(0, Vec::len).max(after.map_or(0, Vec::len)) {
            let before = before.and_then(|items| items.get(ordinal)).copied();
            let after = after.and_then(|items| items.get(ordinal)).copied();
            if before == after && !dirty.contains_key(&key) { continue; }
            let rect = match (before, after) {
                (Some(a), Some(b)) => union(a.overflow, b.overflow),
                (Some(a), None) => a.overflow,
                (None, Some(b)) => b.overflow,
                (None, None) => continue,
            };
            damage.push(LayoutDamage { box_ref: BoxRef { key, ordinal }, before, after, rect });
        }
    }
    damage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoReplacedElements;
    use amazing_text::FallbackTextSystem;

    fn styled(dom: &Dom, css: &str) -> StyledDom {
        amazing_css::compute(dom, &[amazing_css::parse_stylesheet(css)])
    }

    fn node_named(dom: &Dom, name: &str) -> NodeIdx {
        (0..dom.node_count())
            .map(|i| NodeIdx(i as u32))
            .find(|&node| dom.name(node) == Some(name))
            .unwrap()
    }

    fn delta(styles: StyledDom, node: NodeIdx, property: &str) -> StyleDelta {
        StyleDelta {
            styles,
            changed: BTreeMap::from([(node, vec![property.to_string()])]),
            change_kinds: BTreeMap::new(),
            recalculated: vec![node],
            full_rebuild: false,
        }
    }

    #[test]
    fn opacity_updates_retained_style_without_changing_geometry() {
        let dom = amazing_html::parse("<body><div style='width:30px'>x</div></body>");
        let node = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let mut tree = LayoutTree::new(
            &dom,
            styled(&dom, "div{opacity:1}"),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        let before = collect_geometry(tree.root(), &generations(&dom));
        let updated = styled(&dom, "div{opacity:.5}");
        let result = tree.update(
            &dom,
            delta(updated, node, "opacity"),
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(result.outcome, UpdateOutcome::Retained);
        assert!(result.changed.is_empty());
        assert_eq!(collect_geometry(tree.root(), &generations(&dom)), before);
        assert_eq!(result.dirty.len(), 1);
        assert_eq!(result.dirty[0].severity, DirtySeverity::Repaint);
        assert!(tree
            .dirty_boxes()
            .iter()
            .any(|(key, severity)| key.node == node
                && key.generation == dom.node_generation(node).unwrap()
                && *severity == DirtySeverity::Repaint));
        assert!(tree.damage().iter().any(|damage| damage.box_ref.key.node == node
            && damage.box_ref.key.generation == dom.node_generation(node).unwrap()
            && damage.before == damage.after));
    }

    #[test]
    fn contained_block_relayouts_child_and_owns_before_after_damage() {
        let dom = amazing_html::parse("<body><section><div>x</div></section><p>after</p></body>");
        let section = node_named(&dom, "section");
        let child = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let before = "body{margin:0}section{display:flow-root;contain:layout;width:100px;height:80px}section>div{width:20px;height:20px}";
        let after = "body{margin:0}section{display:flow-root;contain:layout;width:100px;height:80px}section>div{width:40px;height:20px}";
        let mut tree = LayoutTree::new(&dom, styled(&dom, before), &text, &NoReplacedElements, 200.0, None);
        let old_section = collect_geometry(tree.root(), &generations(&dom)).into_iter()
            .find(|(key, _)| key.node == section && key.principal).unwrap().1[0].border;
        let result = tree.update(&dom, delta(styled(&dom, after), child, "width"),
            &InputChanges::default(), &text, &NoReplacedElements, 200.0, None);
        assert_eq!(result.outcome, UpdateOutcome::Retained);
        assert!(result.changed.iter().any(|change| change.key.node == child));
        let new_section = collect_geometry(tree.root(), &generations(&dom)).into_iter()
            .find(|(key, _)| key.node == section && key.principal).unwrap().1[0].border;
        assert_eq!(old_section, new_section);
        let damage = tree.damage().iter().find(|damage| damage.box_ref.key.node == child).unwrap();
        assert_eq!(damage.box_ref.key.generation, dom.node_generation(child).unwrap());
        assert_ne!(damage.before, damage.after);
        assert_eq!(tree.box_for_ref(damage.box_ref).unwrap().node, Some(child));
    }

    /// Run explicitly on a build host to track ADR 0010's 20 ms p95 target.
    #[test]
    #[ignore]
    fn contained_relayout_p95_500_siblings() {
        let mut html = String::from("<body><section><div>x</div></section>");
        for _ in 0..500 { html.push_str("<p>stable</p>"); }
        html.push_str("</body>");
        let dom = amazing_html::parse(&html);
        let child = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let base = "body{margin:0}section{display:flow-root;contain:layout;width:100px;height:80px}";
        let narrow = styled(&dom, &format!("{base}section>div{{width:20px;height:20px}}"));
        let wide = styled(&dom, &format!("{base}section>div{{width:40px;height:20px}}"));
        let mut tree = LayoutTree::new(&dom, narrow.clone(), &text, &NoReplacedElements, 800.0, None);
        let mut times = Vec::new();
        for index in 0..40 {
            let next = if index % 2 == 0 { wide.clone() } else { narrow.clone() };
            let start = std::time::Instant::now();
            let result = tree.update(&dom, delta(next, child, "width"),
                &InputChanges::default(), &text, &NoReplacedElements, 800.0, None);
            assert_eq!(result.outcome, UpdateOutcome::Retained);
            times.push(start.elapsed());
        }
        times.sort_unstable();
        let p95 = times[37];
        eprintln!("LAYOUT-17 contained relayout 500 siblings p95={:.3}ms", p95.as_secs_f64() * 1000.0);
    }

    #[test]
    fn css_style_delta_feeds_retained_opacity_update() {
        let mut dom = amazing_html::parse("<body><div>x</div></body>");
        let node = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let sheet = amazing_css::parse_stylesheet(".dim{opacity:.5}");
        let before = amazing_css::compute(&dom, &[sheet.clone()]);
        let mut tree = LayoutTree::new(
            &dom,
            before.clone(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        dom.set_attribute(node, "class", "dim").unwrap();
        let invalidator = amazing_css::StyleInvalidator::new(&[sheet], 1);
        let roots = invalidator.collect(
            &[amazing_css::RenderMutation::attribute(
                node,
                "class",
                None,
                Some("dim"),
            )],
            1,
        );
        let delta = amazing_css::recompute_roots(&dom, &before, roots);
        assert_eq!(delta.changed.get(&node), Some(&vec!["opacity".to_owned()]));
        let result = tree.update(
            &dom,
            delta,
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(result.outcome, UpdateOutcome::Retained);
        assert!(result.changed.is_empty());
    }

    #[test]
    fn width_change_rebuilds_and_reports_geometry() {
        let dom = amazing_html::parse("<body><div>x</div></body>");
        let node = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let mut tree = LayoutTree::new(
            &dom,
            styled(&dom, "div{width:30px}"),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        let result = tree.update(
            &dom,
            delta(styled(&dom, "div{width:50px}"), node, "width"),
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert!(matches!(
            result.outcome,
            UpdateOutcome::RequiresFullRebuild(_)
        ));
        assert!(result.changed.iter().any(|change| change.key.node == node
            && change.old.unwrap().border.w != change.new.unwrap().border.w));
        assert!(result
            .dirty
            .iter()
            .any(|dirty| dirty.node == node && dirty.severity == DirtySeverity::Relayout));
    }

    #[test]
    fn oracle_rejects_a_misclassified_geometry_change() {
        let dom = amazing_html::parse("<body><div>x</div></body>");
        let node = node_named(&dom, "div");
        let text = FallbackTextSystem::new();
        let mut tree = LayoutTree::new(
            &dom,
            styled(&dom, "div{width:30px}"),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        // A faulty caller says opacity changed while the computed width did.
        // The debug oracle must discard the retained placement.
        let result = tree.update(
            &dom,
            delta(styled(&dom, "div{width:50px}"), node, "opacity"),
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(
            result.outcome,
            UpdateOutcome::RequiresFullRebuild("layout parity oracle mismatch")
        );
        assert!(result.changed.iter().any(|change| change.key.node == node));
    }

    #[test]
    fn text_input_requires_full_rebuild_even_without_style_changes() {
        let dom = amazing_html::parse("<body><p>x</p></body>");
        let node = node_named(&dom, "p");
        let text = FallbackTextSystem::new();
        let styles = styled(&dom, "");
        let mut tree = LayoutTree::new(
            &dom,
            styles.clone(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        let result = tree.update(
            &dom,
            StyleDelta {
                styles,
                changed: BTreeMap::new(),
                change_kinds: BTreeMap::new(),
                recalculated: Vec::new(),
                full_rebuild: false,
            },
            &InputChanges {
                text: vec![node],
                ..Default::default()
            },
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(
            result.outcome,
            UpdateOutcome::RequiresFullRebuild("text or replaced intrinsic input changed")
        );
    }

    #[test]
    fn unchanged_containment_output_stops_dirty_ancestors() {
        let dom = amazing_html::parse("<body><section><div>x</div></section></body>");
        let section = node_named(&dom, "section");
        let child = node_named(&dom, "div");
        let body = node_named(&dom, "body");
        let text = FallbackTextSystem::new();
        let container = "section{contain:layout;width:100px;height:100px}";
        let mut tree = LayoutTree::new(
            &dom,
            styled(&dom, &format!("{container} div{{width:30px}}")),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(
            tree.styles
                .get(section)
                .and_then(|style| style.get("contain")),
            Some("layout")
        );
        let result = tree.update(
            &dom,
            delta(
                styled(&dom, &format!("{container} div{{width:50px}}")),
                child,
                "width",
            ),
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert!(result.dirty.iter().any(|entry| entry.node == section));
        assert!(result.dirty.iter().any(|entry| entry.node == child));
        assert!(!result.dirty.iter().any(|entry| entry.node == body));
    }

    #[test]
    fn changed_containment_output_dirties_ancestors() {
        let dom = amazing_html::parse("<body><section><div>x</div></section></body>");
        let child = node_named(&dom, "div");
        let body = node_named(&dom, "body");
        let text = FallbackTextSystem::new();
        let mut tree = LayoutTree::new(
            &dom,
            styled(&dom, "section{contain:layout} div{height:30px}"),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        let result = tree.update(
            &dom,
            delta(
                styled(&dom, "section{contain:layout} div{height:50px}"),
                child,
                "height",
            ),
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert!(result.dirty.iter().any(|entry| entry.node == body));
    }
}

#[cfg(test)]
mod generation_tests {
    use super::*;
    use crate::NoReplacedElements;
    use amazing_text::FallbackTextSystem;

    #[test]
    fn new_dom_generation_never_reuses_old_box_cache() {
        let mut dom = amazing_html::parse("<body><p>x</p></body>");
        let text = FallbackTextSystem::new();
        let before = amazing_css::compute(&dom, &[]);
        let mut tree = LayoutTree::new(&dom, before, &text, &NoReplacedElements, 200.0, None);
        let body = (0..dom.node_count())
            .map(|i| NodeIdx(i as u32))
            .find(|&node| dom.name(node) == Some("body"))
            .unwrap();
        let inserted = dom.create_element(body, "div");
        let after = amazing_css::compute(&dom, &[]);
        let result = tree.update(
            &dom,
            StyleDelta {
                styles: after,
                changed: BTreeMap::new(),
                change_kinds: BTreeMap::new(),
                recalculated: Vec::new(),
                full_rebuild: false,
            },
            &InputChanges::default(),
            &text,
            &NoReplacedElements,
            200.0,
            None,
        );
        assert_eq!(
            result.outcome,
            UpdateOutcome::RequiresFullRebuild("DOM node generation changed")
        );
        assert!(collect_geometry(tree.root(), &generations(&dom))
            .keys()
            .any(|key| key.node == inserted));
        let added = tree.damage().iter().find(|entry| entry.box_ref.key.node == inserted).unwrap();
        assert_eq!(added.box_ref.key.generation, dom.node_generation(inserted).unwrap());
        assert!(added.before.is_none());
        assert!(added.after.is_some());
        assert_eq!(tree.box_for_ref(added.box_ref).unwrap().node, Some(inserted));
    }
}
