//! Selector-feature invalidation and incremental computed-style recomputation.
//! The mutation shape is synthetic until the DOM render journal is available.
use super::*;
use crate::selectors::{Combinator, Compound, Pseudo, Simple};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationKind {
    Attribute {
        name: String,
        old: Option<String>,
        new: Option<String>,
    },
    State {
        pseudo: String,
        old: bool,
        new: bool,
    },
    Tree,
    Text,
    ShadowSlot,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMutation {
    pub node: NodeIdx,
    /// Reserved for the DOM journal's per-node arena generation.
    pub generation: u64,
    pub kind: MutationKind,
}

impl RenderMutation {
    pub fn attribute(node: NodeIdx, name: &str, old: Option<&str>, new: Option<&str>) -> Self {
        Self {
            node,
            generation: 0,
            kind: MutationKind::Attribute {
                name: name.to_ascii_lowercase(),
                old: old.map(str::to_owned),
                new: new.map(str::to_owned),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Feature {
    Class(String),
    Id(String),
    Attribute(String),
    State(String),
}

#[derive(Debug, Clone, Copy, Default)]
struct InvalidationSet {
    self_node: bool,
    descendants: bool,
    following_siblings: bool,
}

#[derive(Default)]
struct FeatureIndex {
    sets: BTreeMap<Feature, InvalidationSet>,
    has_relational: bool,
    has_nth_of: bool,
    has_structural: bool,
}

struct InvalidationContext {
    author_sheets: Vec<Stylesheet>,
    ua_rules: Vec<CompiledRule>,
    author_rules: Vec<CompiledRule>,
    features: FeatureIndex,
    epoch: u64,
}

/// A stylesheet-bound selector dependency index. Recreate it when sheets
/// change; a stale stylesheet epoch safely requests a full recompute.
#[derive(Clone)]
pub struct StyleInvalidator {
    context: Arc<InvalidationContext>,
}

/// Coalesced style work. The caller may inspect `full` before recomputation.
#[derive(Clone)]
pub struct RecalcRoots {
    pub roots: Vec<NodeIdx>,
    pub following_siblings: Vec<NodeIdx>,
    pub full: bool,
    pub stylesheet_epoch: u64,
    context: Arc<InvalidationContext>,
}

/// New retained style state and the properties that actually changed.
#[derive(Debug)]
pub struct StyleDelta {
    pub styles: StyledDom,
    pub changed: BTreeMap<NodeIdx, Vec<String>>,
    pub recalculated: Vec<NodeIdx>,
    pub full_rebuild: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleMismatch {
    pub node: NodeIdx,
    pub property: String,
    pub incremental: Option<String>,
    pub full: Option<String>,
}

impl StyleInvalidator {
    pub fn new(author_sheets: &[Stylesheet], stylesheet_epoch: u64) -> Self {
        let ua_rules = compile_sheets(&[parse_stylesheet(UA_SHEET)], true);
        let author_rules = compile_sheets(author_sheets, false);
        let mut features = FeatureIndex::default();
        for rule in ua_rules.iter().chain(author_rules.iter()) {
            for (selector, _) in &rule.selectors {
                for (index, compound) in selector.compounds.iter().enumerate() {
                    let siblings = selector.combinators[index..].iter().any(|combinator| {
                        matches!(
                            combinator,
                            Combinator::NextSibling | Combinator::LaterSibling
                        )
                    });
                    let effect = if index + 1 == selector.compounds.len() {
                        InvalidationSet {
                            self_node: true,
                            ..Default::default()
                        }
                    } else if siblings {
                        InvalidationSet {
                            following_siblings: true,
                            ..Default::default()
                        }
                    } else {
                        InvalidationSet {
                            descendants: true,
                            ..Default::default()
                        }
                    };
                    collect_compound(compound, effect, &mut features);
                }
            }
        }
        Self {
            context: Arc::new(InvalidationContext {
                author_sheets: author_sheets.to_vec(),
                ua_rules,
                author_rules,
                features,
                epoch: stylesheet_epoch,
            }),
        }
    }

    pub fn collect(&self, mutations: &[RenderMutation], stylesheet_epoch: u64) -> RecalcRoots {
        let mut roots = BTreeSet::new();
        let mut siblings = BTreeSet::new();
        let index = &self.context.features;
        let mut full = stylesheet_epoch != self.context.epoch;
        for mutation in mutations {
            // Until the DOM journal supplies a retained generation for each
            // cached style, a nonzero generation cannot be matched safely.
            if mutation.generation != 0 {
                full = true;
            }
            match &mutation.kind {
                MutationKind::Attribute { name, old, new } => {
                    if old == new {
                        continue;
                    }
                    // Inline style, presentational hints and inheritance can
                    // affect this element even without a selector dependency.
                    roots.insert(mutation.node);
                    if index.has_relational || index.has_nth_of {
                        full = true;
                    }
                    let mut keys = vec![Feature::Attribute(name.to_ascii_lowercase())];
                    if name.eq_ignore_ascii_case("class") {
                        for value in [old, new].into_iter().flatten() {
                            keys.extend(
                                value
                                    .split_ascii_whitespace()
                                    .map(|v| Feature::Class(v.to_owned())),
                            );
                        }
                    } else if name.eq_ignore_ascii_case("id") {
                        for value in [old, new].into_iter().flatten() {
                            keys.push(Feature::Id(value.to_owned()));
                        }
                    }
                    for key in keys {
                        if let Some(set) = index.sets.get(&key) {
                            if set.self_node || set.descendants {
                                roots.insert(mutation.node);
                            }
                            if set.following_siblings {
                                siblings.insert(mutation.node);
                            }
                        }
                    }
                }
                MutationKind::State { pseudo, old, new } => {
                    if old == new {
                        continue;
                    }
                    roots.insert(mutation.node);
                    if index.has_relational
                        || index.has_nth_of
                        || pseudo.eq_ignore_ascii_case("focus-within")
                    {
                        full = true;
                    }
                    if let Some(set) = index.sets.get(&Feature::State(pseudo.to_ascii_lowercase()))
                    {
                        if set.following_siblings {
                            siblings.insert(mutation.node);
                        }
                    }
                }
                // Tree edits can alter positional selectors, :has(), scope,
                // inherited style and counter order. Text can alter :empty or
                // first-letter; slotting changes the flattened ancestor path.
                MutationKind::Tree
                | MutationKind::Text
                | MutationKind::ShadowSlot
                | MutationKind::Unknown => {
                    full = true;
                }
            }
        }
        RecalcRoots {
            roots: roots.into_iter().collect(),
            following_siblings: siblings.into_iter().collect(),
            full,
            stylesheet_epoch,
            context: Arc::clone(&self.context),
        }
    }

    /// Diagnostic oracle; returns the first differing node and property.
    pub fn compare_to_full(&self, dom: &Dom, incremental: &StyledDom) -> Result<(), StyleMismatch> {
        let full = compute(dom, &self.context.author_sheets);
        first_mismatch(incremental, &full).map_or(Ok(()), Err)
    }
}

fn insert_feature(index: &mut FeatureIndex, feature: Feature, effect: InvalidationSet) {
    let target = index.sets.entry(feature).or_default();
    target.self_node |= effect.self_node;
    target.descendants |= effect.descendants;
    target.following_siblings |= effect.following_siblings;
}

fn collect_compound(compound: &Compound, effect: InvalidationSet, index: &mut FeatureIndex) {
    for simple in &compound.simple {
        match simple {
            Simple::Class(value) => insert_feature(index, Feature::Class(value.clone()), effect),
            Simple::Id(value) => insert_feature(index, Feature::Id(value.clone()), effect),
            Simple::Attr { name, .. } => {
                insert_feature(index, Feature::Attribute(name.clone()), effect)
            }
            Simple::Pseudo(Pseudo::Link) | Simple::Pseudo(Pseudo::Visited) => {
                insert_feature(index, Feature::Attribute("href".into()), effect);
            }
            Simple::Pseudo(pseudo) => {
                let name = match pseudo {
                    Pseudo::Hover => Some("hover"),
                    Pseudo::Active => Some("active"),
                    Pseudo::Focus => Some("focus"),
                    Pseudo::FocusWithin => Some("focus-within"),
                    Pseudo::Enabled => Some("enabled"),
                    Pseudo::Disabled => Some("disabled"),
                    Pseudo::Checked => Some("checked"),
                    Pseudo::Required => Some("required"),
                    Pseudo::Optional => Some("optional"),
                    Pseudo::Open => Some("open"),
                    Pseudo::FirstChild
                    | Pseudo::LastChild
                    | Pseudo::OnlyChild
                    | Pseudo::FirstOfType
                    | Pseudo::LastOfType
                    | Pseudo::OnlyOfType
                    | Pseudo::NthChild { .. }
                    | Pseudo::NthLastChild { .. }
                    | Pseudo::NthOfType { .. }
                    | Pseudo::NthLastOfType { .. }
                    | Pseudo::Empty => {
                        index.has_structural = true;
                        None
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    insert_feature(index, Feature::State(name.into()), effect);
                }
            }
            Simple::Lang(_) => {
                insert_feature(
                    index,
                    Feature::Attribute("lang".into()),
                    InvalidationSet {
                        descendants: true,
                        ..effect
                    },
                );
                insert_feature(
                    index,
                    Feature::Attribute("xml:lang".into()),
                    InvalidationSet {
                        descendants: true,
                        ..effect
                    },
                );
            }
            Simple::Dir(_) => insert_feature(
                index,
                Feature::Attribute("dir".into()),
                InvalidationSet {
                    descendants: true,
                    ..effect
                },
            ),
            Simple::Negation(args) | Simple::Matches(args) | Simple::Where(args) => {
                for nested in args {
                    collect_compound(nested, effect, index);
                }
            }
            Simple::ComplexNegation(args) | Simple::ComplexMatches(args) | Simple::ComplexWhere(args) => {
                index.has_relational = true;
                for selector in args {
                    for nested in &selector.compounds {
                        collect_compound(nested, effect, index);
                    }
                }
            }
            Simple::Has(_) => index.has_relational = true,
            Simple::NthChildOf { selectors, .. } => {
                index.has_structural = true;
                index.has_nth_of = true;
                for selector in selectors {
                    for nested in &selector.compounds {
                        collect_compound(nested, effect, index);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Recompute only coalesced roots and their inherited-style descendants.
/// Unknown dependencies or a stylesheet epoch change use the full oracle path.
pub fn recompute_roots(dom: &Dom, old_styles: &StyledDom, roots: RecalcRoots) -> StyleDelta {
    let full = roots.full || roots.stylesheet_epoch != roots.context.epoch;
    let mut work: BTreeSet<NodeIdx> = roots.roots.into_iter().collect();
    if !full {
        for seed in roots.following_siblings {
            if seed.0 as usize >= dom.node_count() {
                continue;
            }
            let mut sibling = dom.get(seed).next_sibling;
            while let Some(node) = sibling {
                if dom.name(node).is_some() {
                    work.insert(node);
                }
                sibling = dom.get(node).next_sibling;
            }
        }
    }
    let mut work: Vec<_> = work
        .into_iter()
        .filter(|node| {
            (node.0 as usize) < dom.node_count()
                && dom.name(*node).is_some()
                && dom.is_descendant(*node, NodeIdx(0))
        })
        .collect();
    work.sort_by_key(|node| {
        let mut depth = 0;
        let mut parent = dom.get(*node).parent;
        while let Some(node) = parent {
            depth += 1;
            parent = dom.get(node).parent;
        }
        (depth, node.0)
    });
    let mut selected = Vec::new();
    for node in work {
        if !selected
            .iter()
            .any(|ancestor| dom.is_descendant(node, *ancestor))
        {
            selected.push(node);
        }
    }
    let (styles, recalculated) = if full {
        let styles = compute(dom, &roots.context.author_sheets);
        let mut nodes: Vec<_> = styles.styles.keys().copied().map(NodeIdx).collect();
        nodes.sort();
        (styles, nodes)
    } else {
        let mut out = old_styles.clone();
        let body_link = hints::body_link(dom);
        let pseudo_targets = PseudoElement::STYLED.map(|pseudo| {
            roots
                .context
                .ua_rules
                .iter()
                .chain(roots.context.author_rules.iter())
                .any(|rule| rule.targets(pseudo))
        });
        let root_font_px = dom
            .children(NodeIdx(0))
            .find(|node| dom.name(*node) == Some("html"))
            .and_then(|html| old_styles.get(html))
            .and_then(|style| style.get("font-size"))
            .and_then(|css| css.strip_suffix("px"))
            .and_then(|px| px.parse::<f32>().ok())
            .unwrap_or(16.0);
        let mut recalculated = Vec::new();
        for root in selected {
            let parent_style = dom
                .get(root)
                .parent
                .and_then(|parent| out.get(parent))
                .cloned();
            let mut pending = vec![root];
            while let Some(node) = pending.pop() {
                if dom.name(node).is_some() {
                    recalculated.push(node);
                    for pseudo in PseudoElement::STYLED {
                        out.pseudo_styles.remove(&(node.0, pseudo));
                        out.matched_pseudos.remove(&(node.0, pseudo));
                    }
                }
                pending.extend(dom.children(node));
            }
            style_subtree(
                dom,
                root,
                parent_style.as_ref(),
                if dom.name(root) == Some("html") {
                    16.0
                } else {
                    root_font_px
                },
                &roots.context.ua_rules,
                &roots.context.author_rules,
                body_link.as_deref(),
                &pseudo_targets,
                None,
                &mut out,
            );
        }
        recalculated.sort();
        if !recalculated.is_empty() {
            out.generated.clear();
            crate::generated::resolve_document(dom, &mut out);
        }
        (out, recalculated)
    };
    let changed = changed_properties(old_styles, &styles);
    StyleDelta {
        styles,
        changed,
        recalculated,
        full_rebuild: full,
    }
}

fn changed_properties(old: &StyledDom, new: &StyledDom) -> BTreeMap<NodeIdx, Vec<String>> {
    let mut changed = BTreeMap::new();
    let nodes: BTreeSet<_> = old
        .styles
        .keys()
        .chain(new.styles.keys())
        .copied()
        .collect();
    for id in nodes {
        let before = old.styles.get(&id);
        let after = new.styles.get(&id);
        if before == after
            && PseudoElement::STYLED.iter().all(|pseudo| {
                old.generated.get(&(id, *pseudo)) == new.generated.get(&(id, *pseudo))
                    && old.get_pseudo(NodeIdx(id), *pseudo) == new.get_pseudo(NodeIdx(id), *pseudo)
                    && old.pseudo_matched(NodeIdx(id), *pseudo)
                        == new.pseudo_matched(NodeIdx(id), *pseudo)
            })
        {
            continue;
        }
        let mut names = BTreeSet::new();
        for style in [before, after].into_iter().flatten() {
            names.extend(style.iter().map(|(name, _)| name.to_owned()));
        }
        let mut differing: Vec<_> = names
            .into_iter()
            .filter(|name| before.and_then(|s| s.get(name)) != after.and_then(|s| s.get(name)))
            .collect();
        for pseudo in PseudoElement::STYLED {
            if old.get_pseudo(NodeIdx(id), pseudo) != new.get_pseudo(NodeIdx(id), pseudo)
                || old.generated.get(&(id, pseudo)) != new.generated.get(&(id, pseudo))
                || old.pseudo_matched(NodeIdx(id), pseudo)
                    != new.pseudo_matched(NodeIdx(id), pseudo)
            {
                differing.push(format!("::{pseudo:?}"));
            }
        }
        if differing.is_empty() {
            differing.push("<computed-state>".into());
        }
        changed.insert(NodeIdx(id), differing);
    }
    changed
}

fn first_mismatch(incremental: &StyledDom, full: &StyledDom) -> Option<StyleMismatch> {
    let nodes: BTreeSet<_> = incremental
        .styles
        .keys()
        .chain(full.styles.keys())
        .copied()
        .collect();
    for id in nodes {
        let node = NodeIdx(id);
        let before = incremental.get(node);
        let after = full.get(node);
        if before != after {
            let mut names = BTreeSet::new();
            for style in [before, after].into_iter().flatten() {
                names.extend(style.iter().map(|(name, _)| name.to_owned()));
            }
            for property in names {
                let a = before.and_then(|style| style.get(&property));
                let b = after.and_then(|style| style.get(&property));
                if a != b {
                    return Some(StyleMismatch {
                        node,
                        property,
                        incremental: a.map(str::to_owned),
                        full: b.map(str::to_owned),
                    });
                }
            }
            return Some(StyleMismatch {
                node,
                property: "<computed-state>".into(),
                incremental: None,
                full: None,
            });
        }
        for pseudo in PseudoElement::STYLED {
            if incremental.get_pseudo(node, pseudo) != full.get_pseudo(node, pseudo)
                || incremental.generated.get(&(id, pseudo)) != full.generated.get(&(id, pseudo))
                || incremental.pseudo_matched(node, pseudo) != full.pseudo_matched(node, pseudo)
            {
                return Some(StyleMismatch {
                    node,
                    property: format!("::{pseudo:?}"),
                    incremental: None,
                    full: None,
                });
            }
        }
    }
    None
}
