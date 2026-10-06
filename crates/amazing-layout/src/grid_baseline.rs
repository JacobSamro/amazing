// Portions ported from DioxusLabs/taffy (src/compute/grid/track_sizing.rs,
// resolve_item_baselines) @ 04965fc5ac0c0e9423f5aab8ca29b48231738c7a, MIT.
// Copyright (c) 2018 Visly Inc.; Copyright (c) 2026 Taffy Authors.
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Preference { First, Last }
#[derive(Clone, Copy)]
pub(super) struct Metric {
    pub row_start: usize,
    pub row_end: usize,
    pub preference: Preference,
    pub participates: bool,
    pub requested: bool,
    pub auto_block_margin: bool,
    pub height: f32,
    pub baseline: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
}
impl Metric {
    pub(super) fn key(self) -> (usize, Preference) {
        match self.preference {
            Preference::First => (self.row_start, self.preference),
            Preference::Last => (self.row_end, self.preference),
        }
    }
    fn edge_distance(self) -> f32 {
        match self.preference {
            Preference::First => self.baseline + self.margin_top,
            Preference::Last => self.height - self.baseline + self.margin_bottom,
        }
    }
}

/// Whole group ascent/descent pass. The caller supplies measured border-box
/// geometry and real first/last baselines after the width-layout producer.
/// Participation must exclude auto block margins and cyclic percentage sizes.
/// Last-baseline edge grouping is our CSS-alignment adaptation; upstream's
/// pinned algorithm provides only first horizontal baseline groups.
pub(super) fn group_distances(metrics: &[Metric]) -> BTreeMap<(usize, Preference), f32> {
    let mut groups = BTreeMap::<(usize, Preference), f32>::new();
    for metric in metrics.iter().copied().filter(|metric| metric.participates) {
        let edge = metric.edge_distance();
        groups.entry(metric.key()).and_modify(|old| *old = old.max(edge)).or_insert(edge);
    }
    groups
}

/// The shim participates in intrinsic row-size contributions rather than being
/// applied solely after tracks were sized. Actual margins remain unchanged.
pub(super) fn intrinsic_shims(metrics: &[Metric]) -> Vec<f32> {
    let groups = group_distances(metrics);
    metrics.iter().copied().map(|metric| {
        if metric.participates { (groups[&metric.key()] - metric.edge_distance()).max(0.0) }
        else { 0.0 }
    }).collect()
}

/// Baselines are absolute used coordinates from genuine laid-out line boxes.
/// Out-of-flow descendants do not supply the parent's in-flow baseline.
pub(super) fn first_baseline(item: &crate::LayoutBox) -> f32 { box_baseline(item, false) }
pub(super) fn last_baseline(item: &crate::LayoutBox) -> f32 { box_baseline(item, true) }
fn box_baseline(item: &crate::LayoutBox, last: bool) -> f32 {
    use crate::Flow;
    let end = item.border_box.y + item.border_box.h;
    if crate::containment::layout_containment(&item.style) { return end; }
    let own_line = if last { item.lines.last() } else { item.lines.first() };
    let baseline = if let Some(line) = own_line { line.baseline } else {
        let children = match &item.flow {
            Flow::Blocks(children) | Flow::Flex(children) | Flow::Grid(children) => Some(children),
            _ => None,
        };
        let in_flow = |child: &&crate::LayoutBox| {
            !child.style.get("position").is_some_and(|value| value.eq_ignore_ascii_case("absolute") || value.eq_ignore_ascii_case("fixed"))
        };
        let descendant = children.and_then(|children| {
            if last {children.iter().rev().find(in_flow)} else {children.iter().find(in_flow)}
        });
        descendant.map_or(end,|child|box_baseline(child,last))
    };
    let overflow=item.style.get("overflow-y").or_else(||item.style.get("overflow")).unwrap_or("visible");
    if ["auto","scroll","hidden"].iter().any(|value|overflow.eq_ignore_ascii_case(value)) {
        baseline.max(item.border_box.y).min(end)
    } else { baseline }
}


pub(super) fn preference(item: &crate::LayoutBox, container: &amazing_css::ComputedStyle) -> Option<Preference> {
    if container.get("writing-mode").is_some_and(|mode| !mode.eq_ignore_ascii_case("horizontal-tb")) { return None; }
    let value=item.style.get("align-self").filter(|value|!value.eq_ignore_ascii_case("auto"))
        .or_else(||container.get("align-items")).unwrap_or("normal");
    match value.to_ascii_lowercase().split_ascii_whitespace().collect::<Vec<_>>().as_slice() {
        ["baseline"] | ["first","baseline"] | ["baseline","first"] => Some(Preference::First),
        ["last","baseline"] | ["baseline","last"] => Some(Preference::Last),
        _ => None,
    }
}

fn own_y_start_min(item: &crate::LayoutBox) -> bool {
    let rtl=item.style.get("direction").is_some_and(|value|value.eq_ignore_ascii_case("rtl"));
    match item.style.get("writing-mode").unwrap_or("horizontal-tb").to_ascii_lowercase().as_str() {
        "vertical-rl" | "vertical-lr" | "sideways-rl" => !rtl,
        "sideways-lr" => rtl,
        _ => true,
    }
}

/// Position sharing groups in final physical areas. First groups use their
/// shared start anchor. Last groups prefer their shared end anchor, but retain
/// safe fallback by moving the whole group past every member's start bound.
/// Auto margins keep priority; cyclic or unsupported orthogonal participants
/// use their own safe first/last fallback instead of entering a sharing group.
pub(super) fn position_rows(items: &mut [crate::LayoutBox], metrics: &[Metric], tops: &[f32], heights: &[f32]) {
    let mut anchors=BTreeMap::<(usize,Preference),(f32,f32)>::new();
    for (index,metric) in metrics.iter().copied().enumerate().filter(|(_,metric)|metric.participates) {
        let start=tops[index]+metric.margin_top+metric.baseline;
        let end=tops[index]+heights[index]-metric.height-metric.margin_bottom+metric.baseline;
        anchors.entry(metric.key()).and_modify(|anchor| {anchor.0=anchor.0.max(start);anchor.1=anchor.1.min(end);}).or_insert((start,end));
    }
    for (index,(item,metric)) in items.iter_mut().zip(metrics).enumerate() {
        if !metric.requested || metric.auto_block_margin {continue;}
        let y=if metric.participates {
            let (start,end)=anchors[&metric.key()];
            let anchor=match metric.preference {Preference::First=>start,Preference::Last=>end.max(start)};
            anchor-metric.baseline
        } else {
            let fits=heights[index]>=metric.height+metric.margin_top+metric.margin_bottom;
            let minimum=if !fits {true} else {match metric.preference {Preference::First=>own_y_start_min(item),Preference::Last=>!own_y_start_min(item)}};
            if minimum {tops[index]+metric.margin_top} else {tops[index]+heights[index]-metric.height-metric.margin_bottom}
        };
        crate::flex::translate(item,0.0,y-item.border_box.y);
    }
}

#[cfg(test)]
#[path = "grid_baseline_tests.rs"]
mod tests;
