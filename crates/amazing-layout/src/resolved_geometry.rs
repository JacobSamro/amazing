//! CSSOM resolved-value geometry from actual layout, never reconstructed from
//! arbitrary ancestor coordinates. Missing principal/fragment geometry remains
//! unavailable so a runtime can use the property's computed-value fallback.
use crate::{Flow, LayoutBox, Rect, values::{Sides, SidesM}};
use amazing_dom::NodeIdx;
use amazing_css::selectors::PseudoElement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutSource { Element(NodeIdx), Pseudo(NodeIdx, PseudoElement) }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBoxKind { Principal, AtomicInline, Replaced, TableWrapper }
#[derive(Debug, Clone, Copy, Default)]
pub struct InsetOverconstraint {
    pub top: bool, pub right: bool, pub bottom: bool, pub left: bool,
}
/// Whether CSSOM resolves the property family from used geometry.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedPropertyApplicability {
    pub width: bool, pub height: bool, pub margin: bool, pub padding: bool,
}
impl ResolvedPropertyApplicability {
    fn of(box_: &LayoutBox) -> Self {
        use crate::table_tree::{role, Role};
        let role = role(box_);
        let rows = role == Role::Row || role.group();
        let columns = matches!(role, Role::Column | Role::ColumnGroup);
        Self { width: !rows, height: !columns,
            margin: !(rows || columns || role == Role::Cell), padding: !(rows || columns) }
    }
    fn replaced() -> Self { Self { width: true, height: true, margin: true, padding: true } }
}
#[derive(Debug, Clone, Copy)]
pub struct ResolvedStyleGeometry {
    pub kind: ResolvedBoxKind,
    pub applicability: ResolvedPropertyApplicability,
    /// Table wrapper positioning bounds; table CSS dimensions use the grid.
    pub table_wrapper_box: Option<Rect>,
    pub content_box: Rect,
    pub border_box: Rect,
    pub margin: SidesM,
    pub padding: Sides,
    pub border: Sides,
    /// Geometry is physical used pixels. CSSOM lengths divide by this factor.
    pub effective_zoom: f64,
    pub containing_block: Rect,
    /// None means the used inset is inapplicable or unsupported (e.g. sticky).
    pub insets: SidesM,
    /// The corresponding inset must use its computed value, per CSSOM §9.
    pub overconstrained: InsetOverconstraint,
}
#[derive(Clone, Copy)]
pub(crate) struct ResolvedPosition {
    pub(crate) snapshot: Option<u64>,
    pub containing_block: Rect,
    pub insets: SidesM,
    pub overconstrained: InsetOverconstraint,
}
impl ResolvedPosition {
    pub(crate) fn normal(containing_block: Rect) -> Self {
        Self { snapshot: None, containing_block, insets: SidesM::default(), overconstrained: InsetOverconstraint::default() }
    }
}

// Cache validation is not part of the retained renderer's Debug parity oracle.
impl std::fmt::Debug for ResolvedPosition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedPosition").field("containing_block", &self.containing_block)
            .field("insets", &self.insets).field("overconstrained", &self.overconstrained).finish()
    }
}
fn geometry_hash(box_: &LayoutBox, children: impl Iterator<Item=u64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    box_.node.hash(&mut hash); box_.generated_pseudo.hash(&mut hash);
    for rect in [box_.content_box, box_.border_box] {
        [rect.x.to_bits(),rect.y.to_bits(),rect.w.to_bits(),rect.h.to_bits()].hash(&mut hash);
    }
    for sides in [box_.padding,box_.border,box_.margin.unwrap_or(0.0)] {
        [sides.top.to_bits(),sides.right.to_bits(),sides.bottom.to_bits(),sides.left.to_bits()].hash(&mut hash);
    }
    [box_.margin.top,box_.margin.right,box_.margin.bottom,box_.margin.left]
        .map(|side|side.map(f32::to_bits)).hash(&mut hash);
    box_.style.effective_zoom().to_bits().hash(&mut hash);
    for property in ["display","position","direction","width","height","top","right","bottom","left",
        "margin-top","margin-right","margin-bottom","margin-left"] {
        box_.style.get(property).hash(&mut hash);
    }
    for fragment in box_.styled_replaced_fragments() {
        fragment.node.hash(&mut hash); fragment.generated_pseudo.hash(&mut hash);
        for rect in [fragment.content_box,fragment.border_box] {
            [rect.x.to_bits(),rect.y.to_bits(),rect.w.to_bits(),rect.h.to_bits()].hash(&mut hash);
        }
    }
    for child in children { child.hash(&mut hash); }
    hash.finish()
}
fn current_snapshot(box_: &LayoutBox) -> u64 {
    let children = match &box_.flow {
        Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => children.as_slice(), _ => &[],
    };
    geometry_hash(box_, children.iter().chain(&box_.positioned).map(current_snapshot))
}
/// One post-order pass at the end of actual positioning; queries validate one
/// immutable root snapshot. A retained geometry-only reflow cannot expose stale
/// CB/inset records and uses computed fallback until a complete positioning pass.
pub(crate) fn capture_geometry_snapshot(box_: &mut LayoutBox) -> u64 {
    let mut children = Vec::new();
    if let Flow::Blocks(items) | Flow::Flex(items) | Flow::Grid(items) = &mut box_.flow {
        for child in items { children.push(capture_geometry_snapshot(child)); }
    }
    for child in &mut box_.positioned { children.push(capture_geometry_snapshot(child)); }
    let snapshot = geometry_hash(box_, children.into_iter());
    if let Some(position) = &mut box_.resolved_position { position.snapshot = Some(snapshot); }
    snapshot
}

/// One actual principal box or one supported atomic fragment. Non-atomic
/// inline, absent, unlaid-out, and fragmented sources return None. Table wrapper
/// identity wins over its duplicate internal table grid. No scroll/sticky or
/// synthesized HTML-root geometry is supplied by this query.
pub fn resolved_style_geometry(root: &LayoutBox, source: LayoutSource) -> Option<ResolvedStyleGeometry> {
    if root.resolved_position.and_then(|position|position.snapshot) != Some(current_snapshot(root)) {
        return None;
    }
    fn visit(box_: &LayoutBox, source: LayoutSource, found: &mut Vec<ResolvedStyleGeometry>) {
        let identity = box_.node.map(|node| match box_.generated_pseudo {
            Some(pseudo) => LayoutSource::Pseudo(node, pseudo), None => LayoutSource::Element(node),
        });
        if identity == Some(source) && !crate::display_policy::non_atomic_inline(&box_.style)
            && !matches!(box_.style.get("display"), Some("none" | "contents"))
            && let Some(position) = box_.resolved_position
        {
            // CSS table decorations and dimensions belong to its inner grid;
            // margin and position belong to the wrapper (§17.4). Both are
            // actual captured boxes with the same retained source identity.
            let decoration = if box_.table_wrapper {
                if let Flow::Blocks(children) = &box_.flow {
                    children.iter().find(|child| child.node == box_.node
                        && child.generated_pseudo == box_.generated_pseudo && !child.table_wrapper)
                        .unwrap_or(box_)
                } else { box_ }
            } else { box_ };
            found.push(ResolvedStyleGeometry {
                kind: if box_.table_wrapper { ResolvedBoxKind::TableWrapper }
                    else if box_.replaced_size.is_some() || box_.svg_root.is_some() { ResolvedBoxKind::Replaced }
                    else if box_.inline_atomic { ResolvedBoxKind::AtomicInline }
                    else { ResolvedBoxKind::Principal },
                applicability: ResolvedPropertyApplicability::of(decoration),
                table_wrapper_box: box_.table_wrapper.then_some(box_.border_box),
                content_box: decoration.content_box, border_box: decoration.border_box,
                margin: box_.margin, padding: decoration.padding, border: decoration.border,
                effective_zoom: decoration.style.effective_zoom(),
                containing_block: position.containing_block, insets: position.insets,
                overconstrained: position.overconstrained,
            });
            // The table grid and generated image within this principal are
            // implementation detail, not additional boxes for this source.
            if box_.table_wrapper { return; }
        }
        if let Some(position) = box_.resolved_position {
            for fragment in box_.styled_replaced_fragments() {
                let identity = match fragment.generated_pseudo {
                    Some(pseudo) => LayoutSource::Pseudo(fragment.node, pseudo),
                    None => LayoutSource::Element(fragment.node),
                };
                if identity == source && !found.iter().any(|_| box_.node == Some(fragment.node)
                    && box_.generated_pseudo == fragment.generated_pseudo)
                {
                    found.push(ResolvedStyleGeometry {
                        kind: ResolvedBoxKind::Replaced,
                        applicability: ResolvedPropertyApplicability::replaced(), table_wrapper_box: None,
                        content_box: fragment.content_box,
                        border_box: fragment.border_box,
                        margin: SidesM { top: Some(fragment.margin.top), right: Some(fragment.margin.right),
                            bottom: Some(fragment.margin.bottom), left: Some(fragment.margin.left) },
                        padding: fragment.padding, border: fragment.border,
                        effective_zoom: fragment.style.effective_zoom(),
                        containing_block: box_.content_box,
                        insets: SidesM::default(), overconstrained: position.overconstrained,
                    });
                }
            }
        }
        if let Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) = &box_.flow {
            for child in children { visit(child, source, found); }
        }
        for child in &box_.positioned { visit(child, source, found); }
    }
    let mut found = Vec::new(); visit(root, source, &mut found);
    (found.len() == 1).then(|| found.remove(0))
}
