//! Compile layer order across sheets and find applicable scope instances.
use super::*;
use crate::parser::{LayerName, Scope};
use crate::selectors::{
    Combinator, Compound, Pseudo, PseudoElement, Simple, matches_in_scope, matches_pseudo_in_scope,
};

pub(super) struct CompiledRule {
    pub(super) selectors: Vec<(Selector, Specificity)>,
    pub(super) decls: Vec<Declaration>,
    pub(super) ua: bool,
    pub(super) layer: usize,
    scopes: Vec<CompiledScope>,
    owner: Option<NodeIdx>,
    pub(super) base_url: Option<String>,
    pub(super) resource_context: Option<u64>,
    pub(super) tree: Option<NodeIdx>,
    pub(super) context_depth: usize,
    implicit_scope_prefixes: Vec<bool>,
}
struct CompiledScope {
    start: Option<Vec<Selector>>,
    end: Option<Vec<Selector>>,
}

#[derive(Default)]
struct Layers {
    children: Vec<(LayerName, (bool, usize), Layers)>,
}
impl Layers {
    fn register(&mut self, path: &[LayerName], sheet: usize, projected: bool) -> Vec<usize> {
        let Some((name, tail)) = path.split_first() else {
            return vec![usize::MAX];
        };
        let identity = if matches!(name, LayerName::Anonymous(_)) {
            (projected, if projected { 0 } else { sheet })
        } else {
            (false, 0)
        };
        let index = self
            .children
            .iter()
            .position(|(n, s, _)| n == name && *s == identity)
            .unwrap_or_else(|| {
                self.children
                    .push((name.clone(), identity, Layers::default()));
                self.children.len() - 1
            });
        let mut key = vec![index];
        key.extend(self.children[index].2.register(tail, sheet, projected));
        key
    }
}

pub(super) fn compile_sheets(
    sheets: &[Stylesheet],
    ua: bool,
    environment: &MediaEnvironment,
) -> Vec<CompiledRule> {
    let mut layers = Layers::default();
    let mut pending = Vec::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        for rule in &sheet.rules {
            if !rule.media.iter().all(|query| {
                crate::media_queries::MediaQueryList::parse(query).evaluate(environment)
            }) {
                continue;
            }
            let key = layers.register(&rule.layer, sheet_index, sheet.projected_layers);
            let selectors: Option<Vec<_>> = rule
                .selector_srcs
                .iter()
                .map(|src| {
                    let selector = scoped_selector(src, !rule.scopes.is_empty())?;
                    let specificity = selector.specificity();
                    Some((selector, specificity))
                })
                .collect();
            let scopes: Option<Vec<_>> = rule
                .scopes
                .iter()
                .enumerate()
                .map(|(i, scope)| compile_scope(scope, i > 0))
                .collect();
            if let (Some(selectors), Some(scopes)) = (selectors, scopes) {
                if !selectors.is_empty() && !rule.declarations.is_empty() {
                    pending.push((
                        key,
                        CompiledRule {
                            selectors,
                            scopes,
                            decls: rule.declarations.clone(),
                            ua,
                            layer: 0,
                            owner: sheet.scope_root,
                            base_url: sheet.base_url.clone(),
                            resource_context: sheet.resource_context,
                            tree: None,
                            context_depth: 0,
                            implicit_scope_prefixes: rule.selector_srcs.iter().map(|source|
                                !rule.scopes.is_empty() && parse_selector(source).is_some_and(|selector|
                                    !selector.compounds.iter().any(has_scope))).collect(),
                        },
                    ));
                }
            }
        }
    }
    let mut keys: Vec<_> = pending.iter().map(|(key, _)| key.clone()).collect();
    keys.sort();
    keys.dedup();
    pending
        .into_iter()
        .map(|(key, mut rule)| {
            rule.layer = if key == [usize::MAX] {
                usize::MAX
            } else {
                keys.binary_search(&key).unwrap()
            };
            rule
        })
        .collect()
}

fn has_scope(comp: &Compound) -> bool {
    comp.simple.iter().any(|s| match s {
        Simple::Pseudo(Pseudo::Scope) => true,
        Simple::Negation(v) | Simple::Matches(v) | Simple::Where(v) => v.iter().any(has_scope),
        Simple::ComplexNegation(v) | Simple::ComplexMatches(v) | Simple::ComplexWhere(v) =>
            v.iter().any(|selector| selector.compounds.iter().any(has_scope)),
        _ => false,
    })
}
fn scoped_selector(src: &str, scoped: bool) -> Option<Selector> {
    let src = src.trim();
    if scoped && src.starts_with(['>', '+', '~']) {
        return parse_selector(&format!(":where(:scope) {src}"));
    }
    let mut selector = parse_selector(src)?;
    if scoped && !selector.compounds.iter().any(has_scope) {
        selector.compounds.insert(
            0,
            Compound {
                simple: vec![Simple::Where(vec![Compound {
                    simple: vec![Simple::Pseudo(Pseudo::Scope)],
                }])],
            },
        );
        selector.combinators.insert(0, Combinator::Descendant);
    }
    Some(selector)
}
fn compile_scope(scope: &Scope, nested: bool) -> Option<CompiledScope> {
    let compile = |list: &Option<Vec<String>>, relative| -> Option<Option<Vec<Selector>>> {
        match list {
            Some(list) => Some(Some(
                list.iter()
                    .map(|s| scoped_selector(s, relative))
                    .collect::<Option<_>>()?,
            )),
            None => Some(None),
        }
    };
    Some(CompiledScope {
        start: compile(&scope.start, nested)?,
        end: compile(&scope.end, true)?,
    })
}

impl CompiledRule {
    pub(super) fn scope_depends_on(&self, predicate: impl Fn(&Selector) -> bool) -> bool {
        self.scopes.iter().any(|scope| scope.start.iter().chain(&scope.end)
            .flatten().any(&predicate))
    }

    pub(super) fn targets(&self, pseudo: PseudoElement) -> bool {
        self.selectors
            .iter()
            .any(|(selector, _)| selector.pseudo_element() == Some(pseudo))
    }

    pub(super) fn matching(
        &self,
        dom: &Dom,
        el: NodeIdx,
        pseudo: Option<PseudoElement>,
    ) -> Option<(Specificity, usize)> {
        let matches_target = |sel: &Selector, scope| match (self.tree, pseudo) {
            (Some(tree),Some(kind)) => crate::selectors::matches_pseudo_in_tree(dom,el,sel,kind,scope,tree),
            (Some(tree),None) => crate::selectors::matches_in_tree(dom,el,sel,scope,tree),
            (None,Some(kind)) => matches_pseudo_in_scope(dom, el, sel, kind, scope),
            (None,None) => matches_in_scope(dom, el, sel, scope),
        };
        let mut best = None;
        for (index,(selector,spec)) in self.selectors.iter().enumerate() {
            let origins=self.tree.map_or_else(||vec![el],|tree|crate::selectors::selector_origins(dom,el,selector,tree));
            for origin in origins {for (root,proximity) in self.scope_roots(dom,origin)? {
                let inclusive_host=self.tree.and_then(|tree|dom.shadow_data(tree)).is_some_and(|data|root==Some(data.host) && el==data.host)
                    && self.implicit_scope_prefixes[index];
                let mut inclusive;
                let subject_selector=if inclusive_host {
                    inclusive=selector.clone();
                    inclusive.compounds.remove(0);
                    inclusive.combinators.remove(0);
                    &inclusive
                } else {selector};
                if matches_target(subject_selector, root)
                    && best.is_none_or(|(s, p)| *spec > s || (*spec == s && proximity < p))
                {
                    best = Some((*spec, proximity));
                }
            }}
        }
        best
    }

    /// Active scope roots after applying all start and end limits. A class
    /// edit whose scope selectors do not depend on that class retains this set.
    pub(super) fn scope_roots(&self, dom: &Dom, el: NodeIdx) -> Option<Vec<(Option<NodeIdx>, usize)>> {
        if self.scopes.is_empty() { return Some(vec![(None, usize::MAX)]); }
        let mut ancestors = vec![];
        let mut node = Some(el);
        while let Some(n) = node {
            if dom.name(n).is_some() {
                ancestors.push(n);
            }
            node = self.tree.map_or_else(||dom.get(n).parent,|tree|crate::selectors::selector_parent(dom,n,tree));
        }
        let document_root = *ancestors.last()?;
        let mut instances: Vec<Option<NodeIdx>> = vec![None];
        for scope in &self.scopes {
            let mut next = vec![];
            for outer in instances {
                for (distance, root) in ancestors.iter().copied().enumerate() {
                    if let Some(outer) = outer {
                        let outer_distance = ancestors.iter().position(|n| *n == outer)?;
                        if distance > outer_distance {
                            continue;
                        }
                    }
                    let starts = match &scope.start {
                        Some(selectors) => selectors
                            .iter()
                            .any(|s| self.tree.map_or_else(||matches_in_scope(dom,root,s,outer),|tree|crate::selectors::matches_in_tree(dom,root,s,outer,tree))),
                        None => root == self.owner.and_then(|owner| {
                            if self.tree==Some(owner) { dom.shadow_data(owner).map(|data|data.host).or(Some(owner)) }
                            else { Some(owner) }
                        }).unwrap_or(document_root),
                    };
                    if !starts {
                        continue;
                    }
                    let excluded = scope.end.as_ref().is_some_and(|selectors| {
                        ancestors[..=distance].iter().any(|n| {
                            selectors
                                .iter()
                                .any(|s| self.tree.map_or_else(||matches_in_scope(dom,*n,s,Some(root)),|tree|crate::selectors::matches_in_tree(dom,*n,s,Some(root),tree)))
                        })
                    });
                    if !excluded && !next.contains(&Some(root)) {
                        next.push(Some(root));
                    }
                }
            }
            instances = next;
        }
        Some(instances.into_iter().flatten().filter_map(|root|
            ancestors.iter().position(|n| *n == root).map(|distance| (Some(root), distance))).collect())
    }
}
