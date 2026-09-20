//! Compile layer order across sheets and find applicable scope instances.
use super::*;
use crate::parser::{LayerName, Scope};
use crate::selectors::{Combinator, Compound, Pseudo, Simple, matches_in_scope};

pub(super) struct CompiledRule {
    selectors: Vec<(Selector, Specificity)>,
    pub(super) decls: Vec<Declaration>,
    pub(super) ua: bool,
    pub(super) layer: usize,
    scopes: Vec<CompiledScope>,
    owner: Option<NodeIdx>,
}
struct CompiledScope {
    start: Option<Vec<Selector>>,
    end: Option<Vec<Selector>>,
}

#[derive(Default)]
struct Layers {
    children: Vec<(LayerName, usize, Layers)>,
}
impl Layers {
    fn register(&mut self, path: &[LayerName], sheet: usize) -> Vec<usize> {
        let Some((name, tail)) = path.split_first() else {
            return vec![usize::MAX];
        };
        let identity = if matches!(name, LayerName::Anonymous(_)) {
            sheet
        } else {
            0
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
        key.extend(self.children[index].2.register(tail, sheet));
        key
    }
}

pub(super) fn compile_sheets(sheets: &[Stylesheet], ua: bool) -> Vec<CompiledRule> {
    let mut layers = Layers::default();
    let mut pending = Vec::new();
    for (sheet_index, sheet) in sheets.iter().enumerate() {
        for rule in &sheet.rules {
            let key = layers.register(&rule.layer, sheet_index);
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
    pub(super) fn matching(&self, dom: &Dom, el: NodeIdx) -> Option<(Specificity, usize)> {
        if self.scopes.is_empty() {
            return self
                .selectors
                .iter()
                .filter(|(s, _)| matches_in_scope(dom, el, s, None))
                .map(|(_, spec)| (*spec, usize::MAX))
                .max();
        }
        let mut ancestors = vec![];
        let mut node = Some(el);
        while let Some(n) = node {
            if dom.name(n).is_some() {
                ancestors.push(n);
            }
            node = dom.get(n).parent;
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
                            .any(|s| matches_in_scope(dom, root, s, outer)),
                        None => root == self.owner.unwrap_or(document_root),
                    };
                    if !starts {
                        continue;
                    }
                    let excluded = scope.end.as_ref().is_some_and(|selectors| {
                        ancestors[..=distance].iter().any(|n| {
                            selectors
                                .iter()
                                .any(|s| matches_in_scope(dom, *n, s, Some(root)))
                        })
                    });
                    if !excluded && !next.contains(&Some(root)) {
                        next.push(Some(root));
                    }
                }
            }
            instances = next;
        }
        let mut best = None;
        for root in instances.into_iter().flatten() {
            let proximity = ancestors.iter().position(|n| *n == root)?;
            for (selector, spec) in &self.selectors {
                if matches_in_scope(dom, el, selector, Some(root))
                    && best.is_none_or(|(s, p)| *spec > s || (*spec == s && proximity < p))
                {
                    best = Some((*spec, proximity));
                }
            }
        }
        best
    }
}
